//! The window: the loop, and everything that holds state between frames.
//!
//! Synchronous, like STAR/CORD's: draw, poll for a frame's worth of time,
//! act, repeat. The fold core is on its own two threads and this never sees
//! a future; it drains events once a frame and reads the truth out of
//! [`crate::fold::state::State`] behind a read lock it drops before drawing.
//!
//! ## The order of a frame
//!
//! 1. [`App::tick`]: drain events, fold a fresh [`Note`] and a conflict
//!    overlay into place, then [`App::refresh`] if the version moved;
//! 2. [`LayoutState::regions`] once, kept in `layout.last`;
//! 3. [`App::draw`];
//! 4. poll, and dispatch whatever arrived.
//!
//! ## Key dispatch, outermost first
//!
//! overlay → the `/` filter's text entry → a `g` waiting for its second key
//! → the focused module's own bindings → the global table. See
//! `ui/keymap.rs`'s module doc, which this mirrors exactly -- the state that
//! decides which layer a key lands in (is an overlay open, is the filter
//! being typed, was `g` just pressed) lives here, in [`App`]; every function
//! in `keymap.rs` itself is a pure function of one key.
//!
//! ## One read lock
//!
//! [`App::refresh`] is the only place [`crate::fold::handle::Handle::state`]
//! is held for longer than it takes to clone a handful of small values out
//! of it: everything a frame draws is copied into [`ViewData`] while the
//! guard is up, and the guard is dropped before anything else happens. A
//! worker takes the write lock to fold its own result in, and a read guard
//! held across a draw or a channel send would stall it.
//!
//! ## Graphics before the terminal
//!
//! [`Graphics::probe_if_tty`] writes a capability query and reads the answer
//! off stdin, which only works before raw mode is on -- see
//! `starkit::term::init`'s own doc and `docs/graphics.md`. So it happens in
//! [`App::run`], before [`starkit::term::init`], never inside [`App::new`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use starkit::chrome::header;
use starkit::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent,
    MouseEventKind,
};
use starkit::graphics::{Graphics, ImageId, Mode};
use starkit::image::RgbaImage;
use starkit::input::TextInput;
use starkit::mouse::ClickTracker;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::widgets::Widget as _;
use starkit::ratatui_image::Image;
use starkit::term::{self, Tui};

use super::keymap::{self, Action};
use super::layout::{self, LayoutState, Regions};
use super::overlays::confirm::Confirm;
use super::overlays::{conflict, Answer, Overlay, Overlays, Pending};
use super::panels::{self, ModuleId};
use super::status;
use super::theme::{self, Theme};
use super::{Bar, Bars};
use crate::config::{Config, Scale};
use crate::fold::entry::{Entry, EntryKind};
use crate::fold::handle::{Command, Event, Handle, NoteLevel};
use crate::fold::ops::{Op, OpId, OpKind, OpStatus};
use crate::fold::preview::Preview;
use crate::fold::selection::Selection;
use crate::fold::sort::SortOrder;
use crate::fold::stack::FrameId;
use crate::session;

/// One frame. Thirty per second is the same ceiling STAR/CORD settled on.
pub const FRAME: Duration = Duration::from_millis(33);

/// How many events one frame will take before it draws anyway. Events carry
/// no state (see `fold/handle.rs`'s module doc), so whatever is left in the
/// channel is still true next frame.
pub const DRAIN_CAP: usize = 500;

/// Everything one frame draws, copied out of [`crate::fold::state::State`]
/// when [`crate::fold::state::State::version`] moves -- see the module doc's
/// note on the one read lock.
#[derive(Debug)]
pub struct ViewData {
    /// Parents only, root first -- the active level is [`Self::rows`], not
    /// one of these.
    pub crumbs: Vec<panels::stack::Crumb>,
    /// `"starwire/ ── 14 files · 3 dirs · 84.2 MB"`.
    pub rule: String,
    pub rows: Vec<panels::stack::Row>,
    pub cursor: usize,
    /// The path the cursor sits on, for `o`, `r` and for following the
    /// preview -- the stack panel's own `Row` carries no path, only a name.
    pub cursor_path: Option<PathBuf>,
    pub loading: bool,
    pub error: Option<String>,
    pub truncated: bool,
    /// The active frame's filter text, empty when there is none.
    pub filter: String,
    /// `Selection::summary()`.
    pub marked: String,
    pub dir_stats: Option<(usize, usize, u64)>,
    /// The active frame's directory, for `session.toml` and for `Push`ing a
    /// path that is not the cursor's.
    pub active_dir: PathBuf,
    pub home: PathBuf,
    /// The active directory with the home prefix replaced by `~`.
    pub location: String,
    pub preview_name: Option<String>,
    pub preview: Option<Arc<Preview>>,
    pub ops: Vec<panels::operations::OpRow>,
    /// [`OpId`]s in the same order as [`Self::ops`] -- `OpRow` is a pure
    /// render struct with no id of its own, so this is how a click or a key
    /// on a row finds the [`Op`] it names.
    pub op_ids: Vec<OpId>,
    /// The running op's `"COPYING ████████░░ 78%"`, for the status row.
    pub running_bar: Option<String>,
    pub has_forward: bool,
    pub trash_available: bool,
    pub show_hidden: bool,
    pub sort: SortOrder,
    /// The active frame's id, for [`App::scroll`]'s per-frame scroll map.
    pub frame_id: FrameId,
}

impl ViewData {
    fn empty() -> Self {
        Self {
            crumbs: Vec::new(),
            rule: String::new(),
            rows: Vec::new(),
            cursor: 0,
            cursor_path: None,
            loading: true,
            error: None,
            truncated: false,
            filter: String::new(),
            marked: String::new(),
            dir_stats: None,
            active_dir: PathBuf::new(),
            home: PathBuf::new(),
            location: String::new(),
            preview_name: None,
            preview: None,
            ops: Vec::new(),
            op_ids: Vec::new(),
            running_bar: None,
            has_forward: false,
            trash_available: false,
            show_hidden: false,
            sort: SortOrder::default(),
            frame_id: FrameId(0),
        }
    }
}

/// The one picture this application has grown, kept between frames.
///
/// Scaling runs on the drawing thread, so it must not run per frame. It is
/// rebuilt only when the picture, the mode or the target size changes, and
/// there is exactly one of these because exactly one picture is ever on
/// screen: the preview's. Everything downstream -- the encoded protocol --
/// is STAR/KIT's cache's problem, keyed on the identity this carries.
struct Scaled {
    source: ImageId,
    mode: Scale,
    pixels: (u32, u32),
    image: Arc<RgbaImage>,
}

pub struct App {
    core: Handle,
    cfg: Config,
    cfg_path: PathBuf,
    session_path: Option<PathBuf>,
    theme: Theme,
    /// The name last resolved through the registry -- `cfg.ui.theme`'s
    /// value, kept alongside it so `t`/`T` can find where in
    /// `registry().selectable()` the current theme sits without re-deriving
    /// it from the resolved [`Theme`], which carries no name of its own.
    theme_name: String,
    graphics: Graphics,
    layout: LayoutState,
    overlays: Overlays,
    /// Some while `/` is being typed.
    filter: Option<TextInput>,
    g_pending: bool,
    note: Option<(String, NoteLevel, Instant)>,
    view: ViewData,
    seen_version: u64,
    /// Per-frame list scroll, keyed by `FrameId` so a level's scroll survives
    /// backing out and returning to it.
    scroll: HashMap<FrameId, usize>,
    preview_scroll: usize,
    ops_cursor: usize,
    ops_scroll: usize,
    /// Every scrollbar the column drew last frame, and the one a press is
    /// holding -- see `starkit::chrome::scrollbar::Scrollbars`'s own doc for
    /// the contract [`App::scroll_bar_to`] keeps with it.
    bars: Bars,
    clicks: ClickTracker,
    /// Throw away what the diff believes is on the screen next frame -- see
    /// STAR/CORD's own `repaint` field, copied here for the same reason: an
    /// overlay that closes leaves cells behind it that nothing else will
    /// think to redraw.
    repaint: bool,
    quit: bool,
    tz: jiff::tz::TimeZone,
    /// The path the preview was last asked to build for, so the cursor
    /// moving is what re-asks for one rather than every frame doing it.
    last_preview_for: Option<PathBuf>,
    /// The one picture this application has scaled, and what it was scaled
    /// from -- see [`Scaled`].
    scaled: Option<Scaled>,
    /// Every identity a protocol has been built under for the picture
    /// currently in the preview, the source's own included.
    ///
    /// `ImageId::of_arc` is an address, and an address is only an identity
    /// for as long as something holds the allocation. STAR/KIT's cache holds
    /// the source picture, but the ids derived from it name pixels this
    /// application made, and nothing in the cache ties those back to the
    /// address they came from. So when the preview's picture changes, every
    /// id built for the old one is forgotten at once, and a later picture
    /// landing on the freed address cannot be handed the old protocol.
    picture_ids: Vec<ImageId>,
    /// Set by the frame snapshots ([`Self::set_now`]) so a row's `time`
    /// column is pinned to [`crate::fold::testing::now`] rather than the
    /// real clock -- `refresh` falls back to [`std::time::SystemTime::now`]
    /// whenever this is `None`, which is always true outside a test.
    now_override: Option<std::time::SystemTime>,
}

impl App {
    /// Build the window. Never touches the terminal -- see [`Self::run`],
    /// which is the only thing that does.
    pub fn new(
        core: Handle,
        cfg: Config,
        cfg_path: PathBuf,
        session_path: Option<PathBuf>,
        graphics: Graphics,
    ) -> Self {
        let (theme, _reason) = theme::registry().resolve_named(&cfg.ui.theme);
        let theme_name = cfg.ui.theme.clone();
        let layout = LayoutState::new(cfg.ui.preview_rows, cfg.ui.ops_rows, cfg.ui.fold_rows);

        let mut app = Self {
            core,
            cfg,
            cfg_path,
            session_path,
            theme,
            theme_name,
            graphics,
            layout,
            overlays: Overlays::new(),
            filter: None,
            g_pending: false,
            note: None,
            view: ViewData::empty(),
            // Never equal to a fresh `State`'s starting version, so the
            // first `refresh` always copies a `ViewData` out rather than
            // seeing "nothing changed" and leaving the empty one in place.
            seen_version: u64::MAX,
            scroll: HashMap::new(),
            preview_scroll: 0,
            ops_cursor: 0,
            ops_scroll: 0,
            bars: Bars::new(),
            clicks: ClickTracker::new(),
            repaint: true,
            quit: false,
            tz: jiff::tz::TimeZone::system(),
            last_preview_for: None,
            scaled: None,
            picture_ids: Vec::new(),
            now_override: None,
        };
        app.refresh();
        app
    }

    /// Take over the terminal and run until something says to stop.
    ///
    /// The probe is before `term::init` and the restore is before the result
    /// is returned, so a failure inside the loop still leaves a usable
    /// terminal behind -- see `docs/graphics.md`'s probe-before-raw-mode
    /// rule.
    pub fn run(
        core: Handle,
        cfg: Config,
        cfg_path: PathBuf,
        session_path: Option<PathBuf>,
    ) -> Result<()> {
        let graphics = Graphics::probe_if_tty(Mode::parse(&cfg.ui.graphics));
        graphics.log_capabilities();
        let mut app = App::new(core, cfg, cfg_path, session_path, graphics);

        let mut term = term::init()?;
        let result = app.event_loop(&mut term);
        term::restore()?;

        if let Some(path) = app.session_path.clone() {
            let session = session::Session {
                last_dir: Some(app.view.active_dir.clone()),
                show_hidden: Some(app.view.show_hidden),
                sort: Some(app.view.sort),
            };
            if let Err(e) = session.save(&path) {
                tracing::warn!("could not save the session: {e}");
            }
        }
        app.core.send(Command::Shutdown);
        result
    }

    fn event_loop(&mut self, term: &mut Tui) -> Result<()> {
        while !self.quit {
            self.tick();

            if std::mem::take(&mut self.repaint) {
                term.clear()?;
            }
            term.draw(|f| {
                self.draw(f.area(), f.buffer_mut());
            })?;

            if event::poll(FRAME)? {
                match event::read()? {
                    TermEvent::Key(k) if k.kind == KeyEventKind::Press => self.key(k),
                    TermEvent::Mouse(m) => self.mouse(m),
                    // A font zoom arrives as a resize, and it changes the
                    // cell size any built protocol was sized for.
                    TermEvent::Resize(..) => {
                        self.graphics.remeasure();
                        self.repaint = true;
                    }
                    TermEvent::Paste(text) => {
                        if let Some(input) = self.filter.as_mut() {
                            input.paste(&text);
                            let text = input.text().to_string();
                            self.core.send(Command::SetFilter(text));
                        }
                    }
                    TermEvent::FocusGained | TermEvent::FocusLost => {}
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Everything a frame does before it draws.
    pub fn tick(&mut self) {
        let batch: Vec<Event> = self.core.drain().take(DRAIN_CAP).collect();
        for event in batch {
            match event {
                Event::Note(note) => {
                    self.note = Some((note.text, note.level, Instant::now()));
                }
                Event::Conflicts(op) => self.open_conflicts(op),
                // Every other event, `Refresh` included, means only "go
                // re-read the truth" -- which `refresh` below does anyway,
                // guarded by the version it already has to check.
                _ => {}
            }
        }
        self.refresh();

        // The preview follows the cursor: a change of entry (or the panel
        // opening on one it had not asked for yet) is what re-sends
        // `Command::Preview`, not every frame.
        if self.layout.is_open(ModuleId::Preview) && self.view.cursor_path != self.last_preview_for
        {
            self.last_preview_for = self.view.cursor_path.clone();
            if let Some(path) = self.last_preview_for.clone() {
                self.core.send(Command::Preview(path));
            }
        }
    }

    /// Open the conflict overlay for an op whose plan just found one under
    /// `ConflictPolicy::Ask`.
    fn open_conflicts(&mut self, op: OpId) {
        let state = self.core.state();
        let Some(found) = state.queue.iter().find(|o| o.id == op) else {
            return;
        };
        let conflicts = found
            .plan
            .as_ref()
            .map(|p| p.conflicts.clone())
            .unwrap_or_default();
        drop(state);
        self.overlays
            .open_conflict(conflict::Prompt::new(op, conflicts));
        self.repaint = true;
    }

    // -- reading the truth ---------------------------------------------

    /// Copy what the panels draw out of `State`, if its version has moved.
    ///
    /// One read lock: everything below is read while `state` is held, and
    /// the guard is dropped (`drop(state)`) before anything else runs --
    /// see the module doc.
    fn refresh(&mut self) {
        let state = self.core.state();
        if state.version == self.seen_version {
            return;
        }
        self.seen_version = state.version;

        let crumbs_all = state.crumbs();
        let active = crumbs_all.last().expect("a stack is never empty");
        let parents = &crumbs_all[..crumbs_all.len().saturating_sub(1)];

        let crumbs: Vec<panels::stack::Crumb> = parents
            .iter()
            .map(|f| panels::stack::Crumb {
                name: level_name(&f.dir, &state.home),
                count: Some(f.rows.len()),
            })
            .collect();

        let now = self.now_override.unwrap_or_else(std::time::SystemTime::now);
        let rows: Vec<panels::stack::Row> = state
            .rows(active)
            .into_iter()
            .map(|entry| build_row(entry, &state.selection, &self.tz, now))
            .collect();

        let listing = state.listing_of(&active.dir);
        let error = listing.and_then(|l| l.error.clone());
        let truncated = listing.map(|l| l.truncated).unwrap_or(false);

        let name = level_name(&active.dir, &state.home);
        let rule = match state.dir_stats() {
            Some((files, dirs, bytes)) => format!(
                "{name} \u{2500}\u{2500} {files} files \u{b7} {dirs} dirs \u{b7} {}",
                crate::fold::format::size(bytes)
            ),
            None => name.clone(),
        };

        // A preview is for the entry under the cursor. With no cursor -- an
        // empty directory, one still loading -- whatever was built last is
        // for something else, and showing it would say the wrong thing.
        let (preview_name, preview) = match (&state.preview, state.cursor_entry()) {
            (Some((path, p)), Some(_)) => (Some(display_name(path)), Some(Arc::clone(p))),
            _ => (None, None),
        };

        // The picture on screen is about to be a different picture, or none.
        // Noted here, while both `Arc`s are alive and can be compared, and
        // acted on below once the read guard is gone. See `forget_picture`.
        let picture_changed =
            picture_address(self.view.preview.as_deref()) != picture_address(preview.as_deref());

        let ops: Vec<panels::operations::OpRow> = state
            .queue
            .iter()
            .map(|op| build_op_row(op, &state.home))
            .collect();
        let op_ids: Vec<OpId> = state.queue.iter().map(|op| op.id).collect();
        let running_bar = state
            .queue
            .running()
            .map(|op| op.progress.line(running_verb_upper(op.kind)));

        self.view = ViewData {
            crumbs,
            rule,
            rows,
            cursor: active.cursor,
            cursor_path: state.cursor_entry().map(|e| e.path.clone()),
            loading: active.loading,
            error,
            truncated,
            filter: active.filter.clone(),
            marked: state.selection.summary(),
            dir_stats: state.dir_stats(),
            active_dir: active.dir.clone(),
            home: state.home.clone(),
            location: home_relative(&active.dir, &state.home),
            preview_name,
            preview,
            ops,
            op_ids,
            running_bar,
            has_forward: state.tabs.active().active_stack().has_forward(),
            trash_available: state.trash_available,
            show_hidden: state.show_hidden,
            sort: state.sort,
            frame_id: active.id,
        };
        drop(state);
        if picture_changed {
            self.forget_picture();
        }
        self.clamp_scrolls();
    }

    /// Keep every scroll position inside what the last known layout can
    /// actually show. Called after `refresh` and after anything else that
    /// might have moved a cursor or a scroll without a fresh `ViewData`.
    fn clamp_scrolls(&mut self) {
        let ops_len = self.view.ops.len();
        self.ops_cursor = if ops_len == 0 {
            0
        } else {
            self.ops_cursor.min(ops_len - 1)
        };

        let Some(regions) = self.layout.last.clone() else {
            return;
        };

        let stack_rect = regions.rect_of(ModuleId::Stack);
        let list_rows =
            panels::stack::visible_rows(stack_rect, self.view.crumbs.len(), self.layout.fold_rows);
        let entry = self.scroll.entry(self.view.frame_id).or_insert(0);
        *entry = starkit::list::clamp_scroll(self.view.cursor, *entry, list_rows);

        if let Some(preview) = &self.view.preview {
            let preview_rect = regions.rect_of(ModuleId::Preview);
            let body = header::body(preview_rect);
            let total = panels::preview::lines(preview, body.width).len();
            let max = total.saturating_sub(usize::from(body.height));
            self.preview_scroll = self.preview_scroll.min(max);
        }

        let ops_rect = regions.rect_of(ModuleId::Operations);
        let ops_body = header::body(ops_rect);
        let ops_visible = usize::from(ops_body.height).max(1);
        self.ops_scroll =
            starkit::list::clamp_scroll(self.ops_cursor, self.ops_scroll, ops_visible);
    }

    // -- keys -------------------------------------------------------------

    pub fn key(&mut self, k: KeyEvent) {
        if self.overlays.is_open() {
            let answer = self.overlays.handle(k);
            self.after_overlay_answer(answer);
            return;
        }

        if let Some(input) = self.filter.as_mut() {
            if keymap::filter_eats(k) {
                let before = input.text().to_string();
                input.handle(k);
                if input.text() != before {
                    let text = input.text().to_string();
                    self.core.send(Command::SetFilter(text));
                }
                return;
            }
            match k.code {
                KeyCode::Enter => {
                    self.filter = None;
                    return;
                }
                KeyCode::Esc => {
                    self.filter = None;
                    self.core.send(Command::ClearFilter);
                    return;
                }
                _ => {}
            }
        }

        if self.g_pending {
            self.g_pending = false;
            if let Some(action) = keymap::g_prefix(k) {
                self.act(action);
            }
            return;
        }
        if k.modifiers.is_empty() && k.code == KeyCode::Char('g') {
            self.g_pending = true;
            return;
        }

        let module = self.layout.focus().module();
        if let Some(action) = keymap::module(module, k).or_else(|| keymap::resolve(k)) {
            self.act(action);
        }
    }

    fn after_overlay_answer(&mut self, answer: Answer) {
        match answer {
            Answer::Consumed | Answer::Closed => {}
            Answer::Confirmed(pending) => self.on_confirmed(pending),
            Answer::Renamed { from, to } => {
                self.core.send(Command::QueueRename { from, to });
            }
            Answer::Policy { op, policy } => {
                self.core.send(Command::SetPolicy(op, policy));
                self.core.send(Command::Run);
            }
            Answer::Quit => self.quit = true,
        }
        // Closing an overlay can uncover a panel that redraws differently
        // from what is on screen (a renamed row, a cleared queue), and a
        // half-typed rename repaints on every keystroke the same way
        // STAR/CORD's own overlays do -- cheap next to the cost of a frame
        // that is wrong.
        self.repaint = true;
    }

    fn on_confirmed(&mut self, pending: Pending) {
        match pending {
            Pending::DeletePermanently(_) => self.core.send(Command::Run),
            Pending::ClearQueue => self.core.send(Command::ClearQueue),
            Pending::CancelRunning(op) => self.core.send(Command::Cancel(op)),
            Pending::Quit => self.quit = true,
        }
    }

    /// One action -- the single place a key or a click becomes a change.
    fn act(&mut self, a: Action) {
        match a {
            Action::CursorUp => self.move_focused(-1),
            Action::CursorDown => self.move_focused(1),
            Action::CursorUpBig => self.move_focused(-10),
            Action::CursorDownBig => self.move_focused(10),
            Action::PageUp => {
                let n = self.page_size();
                self.move_focused(-n);
            }
            Action::PageDown => {
                let n = self.page_size();
                self.move_focused(n);
            }
            Action::Home => self.move_to_edge(true),
            Action::End => self.move_to_edge(false),
            // In practice `enter` on a focused Operations module always
            // resolves to `Action::RunOp` instead (see `keymap::SHADOWS`):
            // the module table is tried before the global one, so this arm
            // is unreachable there. Kept literal and unguarded regardless --
            // a future change to the table should not silently start
            // skipping the confirm `RunOp`/`RunQueue` already asks for.
            Action::Activate => match self.layout.focus() {
                ModuleId::Stack => self.core.send(Command::Enter),
                ModuleId::Operations => self.core.send(Command::Run),
                ModuleId::Preview => {}
            },
            Action::Back => {
                if self.filter.is_some() {
                    self.filter = None;
                    self.core.send(Command::ClearFilter);
                }
            }

            Action::FocusNext => self.layout.focus_next(),
            Action::FocusPrev => self.layout.focus_prev(),
            Action::FocusStack => self.layout.focus_set(ModuleId::Stack),
            Action::FocusPreview => self.layout.focus_set(ModuleId::Preview),
            Action::FocusOperations => self.layout.focus_set(ModuleId::Operations),

            Action::Enter => self.core.send(Command::Enter),
            Action::Pop => self.core.send(Command::Back),
            Action::JumpUp => {
                let target = self.view.crumbs.len().saturating_sub(1);
                self.core.send(Command::JumpTo(target));
            }
            Action::JumpDown => self.core.send(Command::Forward),
            Action::GoHome => self.core.send(Command::Push(self.view.home.clone())),
            Action::GoRoot => self.core.send(Command::Push(PathBuf::from("/"))),
            Action::OpenExternal => {
                if let Some(path) = self.view.cursor_path.clone() {
                    self.core.send(Command::OpenExternal(path));
                }
            }
            Action::Rename => {
                if let Some(path) = self.view.cursor_path.clone() {
                    self.overlays.open_rename(path);
                    self.repaint = true;
                }
            }
            Action::Reload => self.core.send(Command::Reload),

            Action::Mark => self.core.send(Command::ToggleMark),
            Action::MarkAll => self.core.send(Command::MarkAll),
            Action::InvertMarks => self.core.send(Command::InvertMarks),
            Action::ClearMarks => self.core.send(Command::ClearMarks),

            Action::QueueCopy => self.core.send(Command::QueueCopyHere),
            Action::QueueMove => self.core.send(Command::QueueMoveHere),
            Action::QueueDelete => self.core.send(Command::QueueDelete),
            Action::RunQueue | Action::RunOp => self.try_run(),
            Action::CancelRun => {
                if let Some(id) = self.running_op_id() {
                    self.core.send(Command::Cancel(id));
                }
            }
            Action::DropOp => {
                if let Some(id) = self.op_id_at_cursor() {
                    self.core.send(Command::RemoveOp(id));
                }
            }
            Action::ClearQueue => {
                if self.running_op_id().is_some() {
                    self.overlays
                        .open_confirm(Confirm::clear_queue(self.view.ops.len()));
                    self.repaint = true;
                } else {
                    self.core.send(Command::ClearQueue);
                }
            }

            Action::TogglePreview => self.layout.toggle_preview(),
            Action::ToggleHidden => self.core.send(Command::SetHidden(!self.view.show_hidden)),
            Action::NextSortKey => {
                let mut sort = self.view.sort;
                sort.key = sort.key.next();
                self.core.send(Command::SetSort(sort));
            }
            Action::ReverseSort => {
                let mut sort = self.view.sort;
                sort.reverse = !sort.reverse;
                self.core.send(Command::SetSort(sort));
            }
            Action::Filter => {
                self.filter = Some(TextInput::single().with_text(self.view.filter.clone()));
            }

            Action::NextPictureScale => self.cycle_picture_scale(),

            Action::NextTheme => self.cycle_theme(true),
            Action::PrevTheme => self.cycle_theme(false),

            Action::Help => {
                self.overlays.open_help();
                self.repaint = true;
            }
            Action::Quit => {
                if let Some(row) = self
                    .view
                    .ops
                    .iter()
                    .find(|r| r.tone == panels::operations::Tone::Running)
                {
                    self.overlays
                        .open_confirm(Confirm::quit_with_running(&row.title));
                    self.repaint = true;
                } else {
                    self.quit = true;
                }
            }
            Action::Redraw => self.repaint = true,
        }
        self.clamp_scrolls();
    }

    fn move_focused(&mut self, delta: i32) {
        match self.layout.focus() {
            ModuleId::Preview => {
                self.preview_scroll =
                    (self.preview_scroll as i64 + i64::from(delta)).max(0) as usize;
            }
            ModuleId::Operations => {
                let len = self.view.ops.len();
                if len == 0 {
                    return;
                }
                let next = (self.ops_cursor as i64 + i64::from(delta)).clamp(0, len as i64 - 1);
                self.ops_cursor = next as usize;
            }
            ModuleId::Stack => self.core.send(Command::CursorBy(delta)),
        }
    }

    fn move_to_edge(&mut self, top: bool) {
        match self.layout.focus() {
            ModuleId::Preview => self.preview_scroll = if top { 0 } else { usize::MAX },
            ModuleId::Operations => {
                self.ops_cursor = if top {
                    0
                } else {
                    self.view.ops.len().saturating_sub(1)
                };
            }
            ModuleId::Stack => {
                let row = if top { 0 } else { usize::MAX };
                self.core.send(Command::CursorTo(row));
            }
        }
    }

    /// How far a page moves: the focused module's own visible rows, when the
    /// layout is known, ten otherwise.
    fn page_size(&self) -> i32 {
        let Some(regions) = &self.layout.last else {
            return 10;
        };
        let rows = match self.layout.focus() {
            ModuleId::Stack => panels::stack::visible_rows(
                regions.rect_of(ModuleId::Stack),
                self.view.crumbs.len(),
                self.layout.fold_rows,
            ),
            ModuleId::Preview => {
                usize::from(header::body(regions.rect_of(ModuleId::Preview)).height)
            }
            ModuleId::Operations => {
                usize::from(header::body(regions.rect_of(ModuleId::Operations)).height)
            }
        };
        i32::try_from(rows).unwrap_or(i32::MAX).max(1)
    }

    fn op_id_at_cursor(&self) -> Option<OpId> {
        self.view.op_ids.get(self.ops_cursor).copied()
    }

    fn running_op_id(&self) -> Option<OpId> {
        self.view
            .op_ids
            .iter()
            .zip(self.view.ops.iter())
            .find(|(_, row)| row.tone == panels::operations::Tone::Running)
            .map(|(id, _)| *id)
    }

    /// `RunQueue`/`RunOp`: both just start the queue -- `Command::Run` always
    /// picks the first runnable entry, whichever row the cursor happens to
    /// be on. If that entry is a permanent delete and `[ops] confirm_delete`
    /// is set, ask first; `Command::Run` itself is sent only once the answer
    /// is yes.
    fn try_run(&mut self) {
        let state = self.core.state();
        let candidate = state.queue.iter().find(|op| op.status == OpStatus::Queued);
        let prompt = self.cfg.ops.confirm_delete
            && candidate.is_some_and(|op| {
                matches!(
                    op.kind,
                    OpKind::Delete(crate::fold::ops::DeleteHow::Permanent)
                )
            });
        let confirm = prompt.then(|| {
            let op = candidate.expect("prompt is only true when candidate is Some");
            (op.id, op.sources.len())
        });
        drop(state);

        match confirm {
            Some((id, n)) => {
                self.overlays
                    .open_confirm(Confirm::delete_permanently(id, n));
                self.repaint = true;
            }
            None => self.core.send(Command::Run),
        }
    }

    fn cycle_theme(&mut self, forward: bool) {
        let ids = theme::registry().selectable();
        if ids.is_empty() {
            return;
        }
        let current = ids
            .iter()
            .position(|id| *id == self.theme_name)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % ids.len()
        } else {
            (current + ids.len() - 1) % ids.len()
        };
        let name = ids[next].clone();
        let (theme, _reason) = theme::registry().resolve_named(&name);
        self.theme = theme;
        self.theme_name = name.clone();
        self.cfg.ui.theme = name.clone();
        if let Err(e) = starkit::config::edit::set(
            &self.cfg_path,
            "ui",
            "theme",
            &starkit::config::edit::Value::Str(name.clone()),
        ) {
            tracing::warn!("could not save the theme: {e}");
        }
        self.note = Some((format!("theme: {name}"), NoteLevel::Info, Instant::now()));
        self.repaint = true;
    }

    /// `z`: the next picture scale, remembered.
    ///
    /// Written back to `config.toml` the way `t` writes the theme -- this is
    /// a preference somebody sets once for the way they read screenshots,
    /// not a per-session mood, and losing it on every quit would mean
    /// setting it again every time.
    fn cycle_picture_scale(&mut self) {
        let next = self.cfg.preview.image_scale.next();
        self.cfg.preview.image_scale = next;
        if let Err(e) = starkit::config::edit::set(
            &self.cfg_path,
            "preview",
            "image_scale",
            &starkit::config::edit::Value::Str(next.name().to_string()),
        ) {
            tracing::warn!("could not save the picture scale: {e}");
        }
        self.note = Some((
            format!("picture scale: {next}"),
            NoteLevel::Info,
            Instant::now(),
        ));
        self.repaint = true;
    }

    /// The picture at `scaling`, built if this is not the one already held.
    ///
    /// One result is kept, because one picture is ever on screen. Scaling
    /// happens on this thread, so the three things that decide the pixels --
    /// which picture, which mode, how big -- are all compared before any
    /// work is done.
    fn scale_picture(
        &mut self,
        source: ImageId,
        data: &Arc<RgbaImage>,
        scaling: &panels::preview::Scaling,
    ) -> Arc<RgbaImage> {
        let matches = self.scaled.as_ref().is_some_and(|s| {
            s.source == source && s.mode == scaling.mode && s.pixels == scaling.pixels
        });
        if !matches {
            let (w, h) = scaling.pixels;
            let started = Instant::now();
            let image = starkit::image::imageops::resize(&**data, w, h, scaling.filter);
            tracing::debug!(
                "scaled a picture to {w}x{h} ({}) in {:?}",
                scaling.mode,
                started.elapsed()
            );
            self.scaled = Some(Scaled {
                source,
                mode: scaling.mode,
                pixels: scaling.pixels,
                image: Arc::new(image),
            });
        }
        // Just built, or already matched.
        Arc::clone(&self.scaled.as_ref().expect("a scaled picture").image)
    }

    /// Let go of every protocol built for the picture that was in the
    /// preview, and of the scaled copy made from it.
    ///
    /// Called when the preview stops being that picture, whatever the
    /// reason. See [`Self::picture_ids`] for why an address alone is not
    /// enough to make this safe to skip.
    fn forget_picture(&mut self) {
        for id in std::mem::take(&mut self.picture_ids) {
            self.graphics.forget(id);
        }
        self.scaled = None;
    }

    // -- the mouse ----------------------------------------------------------

    pub fn mouse(&mut self, m: MouseEvent) {
        let Some(regions) = self.layout.last.clone() else {
            return;
        };

        // Every scrollbar's own drag, ahead of both the overlay dispatch and
        // the module one below -- a press or a drag that lands on a bar is
        // never anything else's to answer, overlay open or not, because only
        // the bars actually recorded this frame (see `draw`'s two
        // `begin_frame`s) are ones `press` can find.
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((bar, above)) = self.bars.press(m.column, m.row) {
                    self.scroll_bar_to(bar, above);
                    return;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((bar, above)) = self.bars.drag(m.row) {
                    self.scroll_bar_to(bar, above);
                    return;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.bars.release();
            }
            _ => {}
        }

        if self.overlays.is_open() {
            match m.kind {
                MouseEventKind::ScrollDown => self.overlays.scroll(false),
                MouseEventKind::ScrollUp => self.overlays.scroll(true),
                MouseEventKind::Down(MouseButton::Left) => {
                    let answer = self.overlays.click(m.column, m.row, regions.area);
                    self.after_overlay_answer(answer);
                }
                _ => {}
            }
            return;
        }

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(&regions, m.column, m.row),
            MouseEventKind::Down(MouseButton::Right) => self.right_click(&regions, m.column, m.row),
            MouseEventKind::ScrollDown => self.scroll_at(&regions, m.column, m.row, 3),
            MouseEventKind::ScrollUp => self.scroll_at(&regions, m.column, m.row, -3),
            _ => {}
        }
        self.clamp_scrolls();
    }

    /// Apply a press's or a drag's `above` to whichever scroll position
    /// `bar` answers for -- the one rule `Scrollbars` asks of a caller (see
    /// its own doc): the position applied here is what the next `record`/
    /// `draw` for `bar` must report back, or its thumb snaps to wherever the
    /// untouched value is still sitting.
    ///
    /// The stack and the operations queue both scroll a cursor into view
    /// with them (`starkit::list::cursor_into_view`), the same inverse of
    /// `clamp_scroll` their own keyboard paging needs -- otherwise the next
    /// `clamp_scrolls` would re-derive the scroll from wherever the cursor
    /// still is and drag it right back. The stack's cursor also goes back
    /// through `Command::CursorTo`, exactly as a click on a row sends it, so
    /// the fold core agrees once its own event reaches it; the operations
    /// cursor is `App`'s own field, moved the same direct way a key already
    /// moves it in `move_focused`.
    fn scroll_bar_to(&mut self, bar: Bar, above: u32) {
        let above = above as usize;
        match bar {
            Bar::Preview => {
                // `clamp_scrolls` only ever caps this to its max, so a value
                // the drag already kept in range survives it untouched.
                self.preview_scroll = above;
            }
            Bar::Operations => {
                self.ops_scroll = above;
                let height = self
                    .bars
                    .track_of(Bar::Operations)
                    .map(|r| r.height)
                    .unwrap_or(0);
                self.ops_cursor = starkit::list::cursor_into_view(
                    self.ops_cursor,
                    above,
                    usize::from(height),
                    self.view.ops.len(),
                );
            }
            Bar::Stack => {
                self.scroll.insert(self.view.frame_id, above);
                let height = self
                    .bars
                    .track_of(Bar::Stack)
                    .map(|r| r.height)
                    .unwrap_or(0);
                let cursor = starkit::list::cursor_into_view(
                    self.view.cursor,
                    above,
                    usize::from(height),
                    self.view.rows.len(),
                );
                self.view.cursor = cursor;
                self.core.send(Command::CursorTo(cursor));
            }
            Bar::Conflict => {
                if let Some(Overlay::Conflict(p)) = self.overlays.current_mut() {
                    p.scroll = above;
                }
            }
        }
    }

    fn click(&mut self, regions: &Regions, x: u16, y: u16) {
        if let Some(hit) = status::hit(regions.status, &self.status_view(Instant::now()), x, y) {
            match hit {
                status::Hit::Help => {
                    self.overlays.open_help();
                    self.repaint = true;
                }
                status::Hit::Progress => self.layout.focus_set(ModuleId::Operations),
                status::Hit::Location => {}
            }
            return;
        }

        let Some(module) = regions.hit(x, y) else {
            return;
        };
        let rect = regions.rect_of(module);
        let words = panels::words(module);
        if let Some(word) = header::hit(rect, &words, x, y) {
            self.word_click(word);
            return;
        }

        let double = self.clicks.click(x, y);
        if module.folds() && !self.layout.is_open(module) {
            self.layout.focus_set(module);
            return;
        }
        self.layout.focus_set(module);

        match module {
            ModuleId::Stack => {
                let v = self.stack_view();
                match panels::stack::hit(rect, &v, x, y) {
                    Some(panels::stack::Hit::Crumb(i)) => self.core.send(Command::JumpTo(i)),
                    Some(panels::stack::Hit::Row(i)) => {
                        self.core.send(Command::CursorTo(i));
                        if double {
                            self.core.send(Command::Enter);
                        }
                    }
                    None => {}
                }
            }
            ModuleId::Operations => {
                let v = self.operations_view();
                if let Some(i) = panels::operations::hit(rect, &v, x, y) {
                    self.ops_cursor = i;
                    if double {
                        self.try_run();
                    }
                }
            }
            ModuleId::Preview => {}
        }
    }

    fn right_click(&mut self, regions: &Regions, x: u16, y: u16) {
        if regions.hit(x, y) != Some(ModuleId::Stack) {
            return;
        }
        let rect = regions.rect_of(ModuleId::Stack);
        let v = self.stack_view();
        if let Some(panels::stack::Hit::Row(i)) = panels::stack::hit(rect, &v, x, y) {
            self.core.send(Command::CursorTo(i));
            self.core.send(Command::ToggleMark);
        }
    }

    fn scroll_at(&mut self, regions: &Regions, x: u16, y: u16, delta: i32) {
        let Some(module) = regions.hit(x, y) else {
            return;
        };
        match module {
            ModuleId::Stack => {
                let entry = self.scroll.entry(self.view.frame_id).or_insert(0);
                *entry = (*entry as i64 + i64::from(delta)).max(0) as usize;
            }
            ModuleId::Preview => {
                self.preview_scroll =
                    (self.preview_scroll as i64 + i64::from(delta)).max(0) as usize;
            }
            ModuleId::Operations => {
                self.ops_scroll = (self.ops_scroll as i64 + i64::from(delta)).max(0) as usize;
            }
        }
    }

    fn word_click(&mut self, word: panels::Word) {
        match word {
            panels::Word::Back => self.core.send(Command::Back),
            panels::Word::Hidden => self.core.send(Command::SetHidden(!self.view.show_hidden)),
            panels::Word::Sort => {
                let mut sort = self.view.sort;
                sort.key = sort.key.next();
                self.core.send(Command::SetSort(sort));
            }
            panels::Word::Filter => self.act(Action::Filter),
            panels::Word::Run => self.try_run(),
            panels::Word::Clear => self.act(Action::ClearQueue),
            panels::Word::Close => self.layout.toggle_preview(),
        }
    }

    // -- building the views the panels render from --------------------------

    fn stack_view(&self) -> panels::stack::View<'_> {
        panels::stack::View {
            theme: &self.theme,
            focused: self.layout.focus() == ModuleId::Stack,
            crumbs: &self.view.crumbs,
            rule: self.view.rule.clone(),
            rows: &self.view.rows,
            cursor: self.view.cursor,
            scroll: self.scroll.get(&self.view.frame_id).copied().unwrap_or(0),
            fold_rows: self.layout.fold_rows,
            loading: self.view.loading,
            filter: (!self.view.filter.is_empty()).then_some(self.view.filter.as_str()),
            error: self.view.error.as_deref(),
            truncated: self.view.truncated,
        }
    }

    fn operations_view(&self) -> panels::operations::View<'_> {
        panels::operations::View {
            theme: &self.theme,
            focused: self.layout.focus() == ModuleId::Operations,
            folded: !self.layout.is_open(ModuleId::Operations),
            rows: &self.view.ops,
            cursor: self.ops_cursor,
            scroll: self.ops_scroll,
            hint: "enter run \u{b7} x drop \u{b7} esc clear",
        }
    }

    fn status_view(&self, now: Instant) -> status::View<'_> {
        let hints: &[(&str, &str)] = match self.layout.focus() {
            ModuleId::Stack => &[
                ("/", "filter"),
                ("space", "mark"),
                ("enter", "open"),
                ("y", "copy"),
                ("m", "move"),
                ("d", "delete"),
            ],
            ModuleId::Operations => &[("enter", "run"), ("x", "drop"), ("esc", "clear")],
            ModuleId::Preview => &[("j/k", "scroll"), ("i", "fold"), ("z", "scale")],
        };
        status::View {
            theme: &self.theme,
            note: self.note.as_ref(),
            now,
            progress: self.view.running_bar.as_deref(),
            hints,
            marked: &self.view.marked,
            location: &self.view.location,
            graphics: self.graphics.name(),
        }
    }

    // -- drawing --------------------------------------------------------

    pub fn draw(&mut self, area: Rect, buf: &mut Buffer) {
        let bg = Style::default()
            .bg(panels::rgb(self.theme.bg))
            .fg(panels::rgb(self.theme.fg));
        buf.set_style(area, bg);

        // Taken out for the length of the draw rather than borrowed: every
        // panel's own `*_view()` builder takes `&self`, which would otherwise
        // make `&mut self.bars` conflict with it for as long as the view it
        // returned stays alive. `self.bars` is put back before returning,
        // held's survival across `begin_frame` included, exactly as if it
        // had never left.
        let mut bars = std::mem::take(&mut self.bars);
        bars.begin_frame();

        let padding = (self.cfg.ui.padding_x, self.cfg.ui.padding_y);
        let queued = u16::try_from(self.view.ops.len()).unwrap_or(u16::MAX);
        let Some(regions) = self.layout.regions(area, padding, queued).cloned() else {
            too_small(area, buf, &self.theme);
            self.bars = bars;
            return;
        };

        let focus = self.layout.focus();

        {
            let sv = self.stack_view();
            panels::stack::render(regions.rect_of(ModuleId::Stack), buf, &sv, &mut bars);
        }

        let placement = {
            let mut pv = panels::preview::View {
                theme: &self.theme,
                focused: focus == ModuleId::Preview,
                folded: !self.layout.is_open(ModuleId::Preview),
                name: self.view.preview_name.as_deref(),
                preview: self.view.preview.as_deref(),
                scroll: self.preview_scroll,
                graphics: Some(&mut self.graphics),
                scale: self.cfg.preview.image_scale,
            };
            panels::preview::render(regions.rect_of(ModuleId::Preview), buf, &mut pv, &mut bars)
        };

        {
            let ov = self.operations_view();
            panels::operations::render(regions.rect_of(ModuleId::Operations), buf, &ov, &mut bars);
        }

        status::render(regions.status, buf, &self.status_view(Instant::now()));

        if let Some(p) = placement {
            // Taken out of the view for the length of the paint: scaling
            // needs `&mut self`, and the picture is behind an `Arc` for
            // exactly this.
            let data = match self.view.preview.as_deref() {
                Some(Preview::Image { data, .. }) => Some(Arc::clone(data)),
                _ => None,
            };
            if let Some(data) = data {
                // The pixels the protocol is built from: the source picture
                // where the mode asks for nothing, and a scaled copy of it
                // where the mode asks it to grow.
                let pixels = match &p.scale {
                    Some(scaling) => self.scale_picture(p.source, &data, scaling),
                    None => Arc::clone(&data),
                };
                if !self.picture_ids.contains(&p.id) {
                    self.picture_ids.push(p.id);
                }
                if let Some(proto) = self.graphics.protocol(p.id, &pixels, p.area) {
                    Image::new(proto).render(p.area, buf);
                }
            }
        }

        // Overlays are modal: while one is open, only its own bar (the
        // conflict prompt's list) may be pressed, not whatever the stack,
        // preview and operations just drew behind it. A second
        // `begin_frame` clears those three back out; a grab already held
        // survives it regardless, since that is the whole point of a grab
        // outliving the frame it started on.
        if self.overlays.is_open() {
            bars.begin_frame();
        }
        let overlay_cursor = self
            .overlays
            .render(regions.area, buf, &self.theme, &mut bars);
        match overlay_cursor {
            Some((x, y)) => reverse_cell(buf, regions.area, x, y),
            None => {
                if self.filter.is_some() {
                    if let Some((x, y)) = self.filter_caret(&regions) {
                        reverse_cell(buf, regions.area, x, y);
                    }
                }
            }
        }

        self.bars = bars;
    }

    /// Where the live filter text ends on the active level's rule row --
    /// an approximation, since `panels::stack` composes that row's text
    /// itself rather than through a widget that would hand a caret column
    /// back. Close enough for a blinking mark at the right edge of the row.
    fn filter_caret(&self, regions: &Regions) -> Option<(u16, u16)> {
        let body = header::body(regions.rect_of(ModuleId::Stack));
        let split = panels::stack::split(body, self.view.crumbs.len(), self.layout.fold_rows);
        if split.rule.height == 0 || split.rule.width < 2 {
            return None;
        }
        Some((split.rule.x + split.rule.width - 2, split.rule.y))
    }

    // -- frame snapshots --------------------------------------------------
    //
    // Three small hooks `ui/frames.rs` needs and nothing else does: a frame
    // has to be pinned to a fixed clock and zone to be deterministic (see
    // that module's doc), and it needs to read `ViewData` to find a row by
    // name the way `key`/`mouse` already do internally.

    /// Read what the last `refresh` copied out of `State` -- everything a
    /// frame snapshot needs to find a row by name or assert on what is
    /// marked, queued or previewed.
    #[cfg(test)]
    pub(crate) fn view(&self) -> &ViewData {
        &self.view
    }

    /// Pin the zone a row's `time` column is formatted in. Invalidates
    /// `seen_version` and re-runs `refresh` immediately, the same way
    /// `App::new` primes the first one, so the change is visible without a
    /// fresh `Event` to react to.
    #[cfg(test)]
    pub(crate) fn set_tz(&mut self, tz: jiff::tz::TimeZone) {
        self.tz = tz;
        self.seen_version = u64::MAX;
        self.refresh();
    }

    /// Pin the clock `refresh` reads a row's `time` column against --
    /// [`crate::fold::testing::now`], so it agrees with the fixture's own
    /// pinned mtimes regardless of when the test actually runs. See
    /// [`Self::set_tz`] for why this re-runs `refresh` on the spot.
    #[cfg(test)]
    pub(crate) fn set_now(&mut self, now: std::time::SystemTime) {
        self.now_override = Some(now);
        self.seen_version = u64::MAX;
        self.refresh();
    }
}

fn reverse_cell(buf: &mut Buffer, area: Rect, x: u16, y: u16) {
    if x >= area.x && y >= area.y && x < area.x + area.width && y < area.y + area.height {
        buf[(x, y)].modifier |= Modifier::REVERSED;
    }
}

fn too_small(area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let text = format!(
        "STAR/FOLD needs at least {}\u{d7}{}",
        layout::MIN_COLS,
        layout::MIN_ROWS
    );
    let text: String = text.chars().take(usize::from(area.width)).collect();
    let x = area.x + area.width.saturating_sub(text.chars().count() as u16) / 2;
    let y = area.y + area.height / 2;
    buf.set_string(x, y, text, Style::default().fg(panels::rgb(theme.error)));
}

/// `~` for the home directory, `name/` for anything else.
fn level_name(dir: &std::path::Path, home: &std::path::Path) -> String {
    if dir == home {
        return "~".to_string();
    }
    match dir.file_name() {
        Some(n) => format!("{}/", n.to_string_lossy()),
        None => format!("{}/", dir.display()),
    }
}

/// The active directory, with the home prefix replaced by `~`.
fn home_relative(dir: &std::path::Path, home: &std::path::Path) -> String {
    if dir == home {
        return "~".to_string();
    }
    match dir.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => dir.display().to_string(),
    }
}

/// Which picture a preview is showing, if it is showing one.
///
/// The address rather than the `Arc`, because the answer is only ever
/// compared with another answer, and the two `Arc`s involved are both alive
/// at the moment of the comparison.
fn picture_address(preview: Option<&Preview>) -> Option<ImageId> {
    match preview {
        Some(Preview::Image { data, .. }) => Some(ImageId::of_arc(data)),
        _ => None,
    }
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn build_row(
    entry: &Entry,
    selection: &Selection,
    tz: &jiff::tz::TimeZone,
    now: std::time::SystemTime,
) -> panels::stack::Row {
    let kind = if entry.kind == EntryKind::Symlink {
        panels::stack::Kind::Symlink {
            broken: entry.link_kind.is_none(),
        }
    } else if entry.kind == EntryKind::Dir {
        panels::stack::Kind::Dir
    } else if entry.executable {
        panels::stack::Kind::Exec
    } else if entry.hidden {
        panels::stack::Kind::Hidden
    } else {
        panels::stack::Kind::File
    };

    let name = if entry.is_dir_like() {
        format!("{}/", entry.display)
    } else {
        entry.display.clone()
    };

    let ext = match kind {
        panels::stack::Kind::Dir => "dir".to_string(),
        panels::stack::Kind::Symlink { .. } => "link".to_string(),
        _ => entry.ext(),
    };

    let size = if entry.is_dir_like() {
        "-".to_string()
    } else {
        crate::fold::format::size(entry.len)
    };

    let time = entry
        .modified
        .map(|t| crate::fold::format::when(t, tz, now))
        .unwrap_or_else(|| "-".to_string());

    let mark = if selection.is_empty() {
        panels::stack::Mark::None
    } else if selection.is_marked(&entry.path) {
        panels::stack::Mark::Marked
    } else {
        panels::stack::Mark::Unmarked
    };

    panels::stack::Row {
        mark,
        name,
        kind,
        ext,
        size,
        time,
    }
}

fn build_op_row(op: &Op, home: &std::path::Path) -> panels::operations::OpRow {
    use panels::operations::Tone;

    let (status, tone) = match op.status {
        OpStatus::Queued => ("queued".to_string(), Tone::Pending),
        OpStatus::Planning => ("planning".to_string(), Tone::Pending),
        OpStatus::NeedsPolicy => {
            let n = op.plan.as_ref().map(|p| p.conflicts.len()).unwrap_or(0);
            let noun = if n == 1 { "conflict" } else { "conflicts" };
            (format!("waiting: {n} {noun}"), Tone::Conflict)
        }
        OpStatus::Running => {
            let pct = (op.progress.fraction() * 100.0).round() as u32;
            (
                format!("{} {pct}%", running_verb_lower(op.kind)),
                Tone::Running,
            )
        }
        OpStatus::Done => ("done".to_string(), Tone::Done),
        // `Op` keeps no `Outcome` of its own (see `fold/ops/mod.rs`) -- the
        // detailed "n of m failed" count already reached the status row once,
        // as an `Event::Note`, when the op finished.
        OpStatus::Failed => ("failed".to_string(), Tone::Failed),
        OpStatus::Cancelled => ("cancelled".to_string(), Tone::Failed),
    };
    let bar = matches!(op.status, OpStatus::Running).then(|| op.progress.bar(10));

    panels::operations::OpRow {
        title: op_title(op, home),
        status,
        bar,
        tone,
    }
}

/// `Op::title` with the destination spelled the way the status row spells a
/// directory: `~` for home and relative to it, since the queue is read next
/// to the location it will land in.
fn op_title(op: &Op, home: &std::path::Path) -> String {
    match &op.dest {
        Some(dest) => {
            let verb = op.title();
            let verb = verb.split(" \u{2192} ").next().unwrap_or(&verb).to_string();
            format!("{verb} \u{2192} {}", home_relative(dest, home))
        }
        None => op.title(),
    }
}

fn running_verb_lower(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Copy => "copying",
        OpKind::Move => "moving",
        OpKind::Delete(_) => "deleting",
        OpKind::Rename => "renaming",
    }
}

fn running_verb_upper(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Copy => "COPYING",
        OpKind::Move => "MOVING",
        OpKind::Delete(_) => "DELETING",
        OpKind::Rename => "RENAMING",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fake;
    use starkit::crossterm::event::KeyModifiers;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn app() -> (App, fake::Fake, tempfile::TempDir) {
        let cfg = Config::default();
        let (core, fk) = fake::handle(cfg.core());
        let dir = tempfile::tempdir().expect("a temporary directory");
        let cfg_path = dir.path().join("config.toml");
        let app = App::new(core, cfg, cfg_path, None, Graphics::disabled());
        (app, fk, dir)
    }

    fn dump(buf: &Buffer, area: Rect) -> String {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row_index(app: &App, name: &str) -> usize {
        app.view
            .rows
            .iter()
            .position(|r| r.name.trim_end_matches('/') == name)
            .unwrap_or_else(|| panic!("{name} is not listed: {:?}", app.view.rows))
    }

    /// A fresh directory under the fixture's home, with more files in it
    /// than a 60x21 frame's stack listing has rows for -- see
    /// `dragging_the_stack_scrollbar_scrolls_and_keeps_the_cursor_in_view`'s
    /// own comment for the arithmetic that makes that true.
    fn many_files(fk: &fake::Fake, n: usize) -> std::path::PathBuf {
        let dir = fk.home().join("many");
        std::fs::create_dir(&dir).expect("a fresh directory");
        for i in 0..n {
            std::fs::write(dir.join(format!("file{i:02}.txt")), b"x").expect("a small file");
        }
        dir
    }

    /// `App::new` over the fake core draws a whole frame -- the heading, a
    /// fixture name and the help hint -- without panicking.
    #[test]
    fn app_new_draws_a_frame_with_the_heading_a_fixture_name_and_the_help_hint() {
        let (mut app, _fk, _dir) = app();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        app.draw(area, &mut buf);
        let text = dump(&buf, area);
        assert!(text.contains("S T A R / F O L D"), "{text}");
        assert!(text.contains("projects/"), "{text}");
        assert!(text.contains("? help"), "{text}");
    }

    #[test]
    fn pressing_j_moves_the_cursor_down_one_row() {
        let (mut app, fk, _dir) = app();
        let before = app.view.cursor;
        app.key(key('j'));
        fk.pump();
        app.tick();
        assert_eq!(app.view.cursor, before + 1);
    }

    #[test]
    fn l_enters_the_directory_under_the_cursor_and_the_home_crumb_appears() {
        let (mut app, fk, _dir) = app();
        let idx = row_index(&app, "projects");
        for _ in 0..idx {
            app.key(key('j'));
        }
        fk.pump();
        app.tick();

        app.key(key('l'));
        fk.pump();
        app.tick();

        assert!(
            app.view.crumbs.iter().any(|c| c.name == "~"),
            "{:?}",
            app.view.crumbs
        );
        assert!(app
            .view
            .rows
            .iter()
            .any(|r| r.name.trim_end_matches('/') == "starwire"));
    }

    #[test]
    fn space_marks_the_cursor_entry_and_the_status_shows_one_marked() {
        let (mut app, fk, _dir) = app();
        app.key(key(' '));
        fk.pump();
        app.tick();
        assert!(app.view.marked.contains("1 marked"), "{}", app.view.marked);
    }

    #[test]
    fn slash_then_typing_and_enter_filters_the_rows() {
        let (mut app, fk, _dir) = app();
        fake::open(&app.core, &fk, "projects/starwire");
        app.tick();
        assert!(row_index(&app, "Cargo.toml") < usize::MAX);

        app.key(key('/'));
        for c in "car".chars() {
            app.key(key(c));
        }
        fk.pump();
        app.tick();
        app.key(code(KeyCode::Enter));

        assert!(!app.view.rows.is_empty(), "{:?}", app.view.rows);
        for row in &app.view.rows {
            assert!(
                row.name.to_lowercase().contains("car"),
                "{:?} does not match the filter",
                row.name
            );
        }
        assert!(app.view.rows.iter().any(|r| r.name == "Cargo.toml"));
        assert!(!app.view.rows.iter().any(|r| r.name == "README.md"));
    }

    #[test]
    fn question_mark_opens_help_and_escape_closes_it() {
        let (mut app, _fk, _dir) = app();
        app.key(key('?'));
        assert!(app.overlays.is_open());
        app.key(code(KeyCode::Esc));
        assert!(!app.overlays.is_open());
    }

    #[test]
    fn y_then_shift_x_copies_the_marked_file_to_the_destination() {
        let (mut app, fk, _dir) = app();
        let idx = row_index(&app, "blob.bin");
        for _ in 0..idx {
            app.key(key('j'));
        }
        fk.pump();
        app.tick();
        app.key(key(' '));
        fk.pump();
        app.tick();

        fake::open(&app.core, &fk, "empty");
        app.tick();

        app.key(key('y'));
        fk.pump();
        app.tick();
        app.key(key('X'));
        fk.pump();
        app.tick();
        fk.pump();
        app.tick();

        assert!(fk.home().join("empty/blob.bin").exists());
    }

    /// A 60x21 frame gives the stack listing exactly `LIST_ROWS_MIN` (7)
    /// rows once a level is pushed under the fixture's home (one crumb row,
    /// one rule row, both floors -- see `layout`'s own doc for the
    /// arithmetic): forty files overflow that by a wide margin, so the bar
    /// dragged here is never in doubt about having something to scroll.
    #[test]
    fn dragging_the_stack_scrollbar_scrolls_and_keeps_the_cursor_in_view() {
        let (mut app, fk, _dir) = app();
        many_files(&fk, 40);
        fake::open(&app.core, &fk, "many");
        app.tick();
        assert_eq!(app.view.rows.len(), 40, "{:?}", app.view.rows);

        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        app.draw(area, &mut buf);

        let track = app
            .bars
            .track_of(Bar::Stack)
            .expect("an overflowing list has a thumb to press");

        // The thumb starts flush with the track's own top -- scroll is
        // still 0 -- so a press there takes hold of it rather than jumping
        // it.
        app.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            track.x,
            track.y,
        ));
        assert_eq!(app.bars.held(), Some(Bar::Stack));

        let bottom = track.y + track.height - 1;
        app.mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            track.x,
            bottom,
        ));

        let max_scroll = app.view.rows.len() - usize::from(track.height);
        let scroll = *app
            .scroll
            .get(&app.view.frame_id)
            .expect("the active frame has a scroll entry");
        assert_eq!(
            scroll, max_scroll,
            "dragging to the track's bottom scrolls to the end"
        );
        assert!(
            app.view.cursor >= scroll && app.view.cursor < scroll + usize::from(track.height),
            "cursor {} is not inside the dragged-to viewport {scroll}..{}",
            app.view.cursor,
            scroll + usize::from(track.height)
        );

        // A fresh draw re-records the same bar with the value the drag just
        // applied, so `clamp_scrolls` -- which every real mouse event calls,
        // but a bare `draw` never does -- has nothing left to pull back.
        app.draw(area, &mut buf);
        app.clamp_scrolls();
        assert_eq!(
            *app.scroll.get(&app.view.frame_id).unwrap(),
            max_scroll,
            "clamp_scrolls must leave a value the drag already kept in range alone"
        );

        app.mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            track.x,
            bottom,
        ));
        assert_eq!(app.bars.held(), None, "releasing lets go of the grab");
    }

    #[test]
    fn dragging_the_preview_scrollbar_scrolls_it() {
        let (mut app, fk, _dir) = app();
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        std::fs::write(fk.home().join("long.txt"), text).expect("a long text file");
        app.core.send(Command::Reload);
        fk.pump();
        app.tick();

        let idx = row_index(&app, "long.txt");
        for _ in 0..idx {
            app.key(key('j'));
        }
        fk.pump();
        app.tick();
        // The cursor landing on `long.txt` is what asks the core to build
        // its preview; a second pump-and-tick picks the answer up.
        fk.pump();
        app.tick();
        assert!(
            matches!(app.view.preview.as_deref(), Some(Preview::Text { .. })),
            "{:?}",
            app.view.preview
        );

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        app.draw(area, &mut buf);

        let track = app
            .bars
            .track_of(Bar::Preview)
            .expect("a 200-line file overflows the preview");

        app.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            track.x,
            track.y,
        ));
        assert_eq!(app.bars.held(), Some(Bar::Preview));

        let bottom = track.y + track.height - 1;
        app.mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            track.x,
            bottom,
        ));
        assert!(
            app.preview_scroll > 0,
            "dragging the preview's thumb down should scroll it"
        );

        app.mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            track.x,
            bottom,
        ));
        assert_eq!(app.bars.held(), None);
    }

    #[test]
    fn a_press_beside_the_thumb_jumps_there() {
        let (mut app, fk, _dir) = app();
        many_files(&fk, 40);
        fake::open(&app.core, &fk, "many");
        app.tick();

        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        app.draw(area, &mut buf);

        let track = app.bars.track_of(Bar::Stack).expect("the list overflows");
        assert_eq!(*app.scroll.get(&app.view.frame_id).unwrap_or(&0), 0);

        // The thumb sits at the track's top; a press at its bottom is a
        // press beside it, which jumps it there in this same call rather
        // than waiting for a drag.
        let bottom = track.y + track.height - 1;
        app.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            track.x,
            bottom,
        ));

        let scroll = *app
            .scroll
            .get(&app.view.frame_id)
            .expect("the active frame has a scroll entry");
        assert!(scroll > 0, "a press beside the thumb should jump it");
        assert_eq!(
            app.bars.held(),
            Some(Bar::Stack),
            "the jumped-to thumb is now held"
        );
    }

    #[test]
    fn q_quits() {
        let (mut app, _fk, _dir) = app();
        assert!(!app.quit);
        app.key(key('q'));
        assert!(app.quit);
    }

    #[test]
    fn drawing_below_the_floor_writes_the_size_message() {
        let (mut app, _fk, _dir) = app();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        app.draw(area, &mut buf);
        let text = dump(&buf, area);
        assert!(text.contains("STAR/FOLD needs at least"), "{text}");
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn arb_key() -> impl Strategy<Value = KeyEvent> {
            let code = prop_oneof![
                Just(KeyCode::Esc),
                Just(KeyCode::Enter),
                Just(KeyCode::Up),
                Just(KeyCode::Down),
                Just(KeyCode::Left),
                Just(KeyCode::Right),
                Just(KeyCode::PageUp),
                Just(KeyCode::PageDown),
                Just(KeyCode::Home),
                Just(KeyCode::End),
                Just(KeyCode::Tab),
                Just(KeyCode::Backspace),
                Just(KeyCode::Delete),
                prop_oneof![
                    Just('j'),
                    Just('k'),
                    Just('h'),
                    Just('l'),
                    Just('g'),
                    Just(' '),
                    Just('a'),
                    Just('A'),
                    Just('u'),
                    Just('y'),
                    Just('m'),
                    Just('d'),
                    Just('X'),
                    Just('x'),
                    Just('i'),
                    Just('.'),
                    Just('s'),
                    Just('S'),
                    Just('/'),
                    Just('t'),
                    Just('T'),
                    Just('o'),
                    Just('r'),
                    Just('?'),
                    Just('q'),
                ]
                .prop_map(KeyCode::Char),
            ];
            let modifiers = prop_oneof![
                Just(KeyModifiers::NONE),
                Just(KeyModifiers::SHIFT),
                Just(KeyModifiers::ALT),
                Just(KeyModifiers::CONTROL),
            ];
            (code, modifiers).prop_map(|(code, modifiers)| KeyEvent::new(code, modifiers))
        }

        proptest! {
            /// Whatever keys arrive, in whatever order, the window never
            /// panics -- pressing, draining the fake core and drawing a
            /// frame in between, the way a real session interleaves them.
            #[test]
            fn random_keys_never_panic(keys in proptest::collection::vec(arb_key(), 0..40)) {
                let (mut app, fk, _dir) = app();
                let area = Rect::new(0, 0, 100, 30);
                for k in keys {
                    if app.quit {
                        break;
                    }
                    app.key(k);
                    fk.pump();
                    app.tick();
                    let mut buf = Buffer::empty(area);
                    app.draw(area, &mut buf);
                }
            }
        }
    }
}
