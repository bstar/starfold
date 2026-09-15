//! Snapshots of whole frames.
//!
//! A layout regression is a diff of a drawn screen -- the one thing a pile of
//! assertions about rectangles cannot show: a column clipped mid-word, a rule
//! that runs into the size, a crumb row that squeezed when it should not
//! have. Every one of those is found by looking at a frame, which is what
//! `insta` lets a person (and a review) do without running the program.
//!
//! Two sizes, because they exercise different code: a hundred by thirty is
//! the column with room to spend, and sixty by twenty-one is the floor,
//! where every module is at its minimum and nothing is left over. Two
//! themes, because `[fold]`'s roles are resolved per theme and only a drawn
//! frame shows what they resolve to: `terminal`, the sixteen-colour one, and
//! `catppuccin-mocha`, the default, which has a full base16 palette.
//!
//! ## Determinism
//!
//! Every app here is built with [`starkit::graphics::Graphics::disabled`]
//! (so an image preview falls back to half blocks rather than reaching for a
//! terminal protocol nothing here has), UTC, and the clock pinned to
//! [`crate::fold::testing::now`] -- the same instant the fixture's own
//! mtimes are measured from, so a row's `time` column reads the same string
//! whatever day this is run on. `App::set_tz` and `App::set_now` are the two
//! `#[cfg(test)] pub(crate)` hooks this added to `ui/app.rs` for that; a
//! third, `App::view`, hands back a read-only [`super::app::ViewData`] so a
//! row can be found by name the way `key`/`mouse` already do internally
//! (`ui/app.rs`'s own tests do the same thing from inside the `app` module,
//! where a private field is reachable -- `frames` is a sibling module and
//! needs the accessor). [`fake::Fake::state_mut`] is a fourth hook, on
//! `ui/fake.rs`: a write guard on the fixture's `State`, for the one thing no
//! real run leaves sitting still long enough to draw -- an operation
//! `Running` at a chosen percentage. It bypasses `state::apply`, so every
//! caller that uses it bumps `State::version` itself afterwards for
//! `App::tick`'s `refresh` to notice.
//!
//! Every path in the fixture is under its own temporary home, so `~` is what
//! every frame's crumbs and location actually say -- nothing here depends on
//! whatever directory the test happens to run in.

use std::path::PathBuf;

use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::graphics::Graphics;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;

use super::app::App;
use super::fake;
use crate::config::{Config, Ui};
use crate::fold::ops::OpStatus;
use crate::fold::testing;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn alt_code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::ALT)
}

/// A frame as text, one row per line, trailing spaces trimmed. Styles are
/// asserted by the theme legibility test and by `ui/panels`' own golden
/// dumps; what is asserted here is the shape.
fn render(app: &mut App, w: u16, h: u16) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    app.draw(area, &mut buf);
    (0..h)
        .map(|y| {
            let row: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run whatever a key (or a hand-set change to the fake's `State`) produced,
/// and fold the result into `App` -- the same `fk.pump(); app.tick();` shape
/// `ui/app.rs`'s own tests drive the fake with, twice over: the preview
/// follows the cursor one tick behind (`App::tick` only asks for a fresh one
/// *after* `refresh` notices the cursor moved, which is *after* the `pump`
/// that would have built it), so a single round leaves a freshly-moved-to
/// preview one request short. A second `pump`/`tick` round picks up exactly
/// that request; a caller that has not moved the cursor finds nothing left
/// to do on the second round, so this is never wrong to call.
fn settle(app: &mut App, fk: &fake::Fake) {
    fk.pump();
    app.tick();
    fk.pump();
    app.tick();
}

/// An app over the fixture, pinned to `theme`, UTC, and the fixture's own
/// clock -- see the module doc.
fn build(theme: &str) -> (App, fake::Fake) {
    let cfg = Config {
        ui: Ui {
            theme: theme.into(),
            ..Ui::default()
        },
        ..Config::default()
    };
    let (core, fk) = fake::handle(cfg.core());
    let mut app = App::new(
        core,
        cfg,
        PathBuf::from("/nonexistent/config.toml"),
        None,
        Graphics::disabled(),
    );
    app.set_tz(jiff::tz::TimeZone::UTC);
    app.set_now(testing::now());
    (app, fk)
}

/// The row named `name`'s index in whatever level is active, by the same
/// name-matching `ui/app.rs`'s own tests use -- panics with the whole row
/// list when it is not there, which is a better failure than an out-of-range
/// index a moment later.
fn row_index(app: &App, name: &str) -> usize {
    app.view()
        .rows
        .iter()
        .position(|r| r.name.trim_end_matches('/') == name)
        .unwrap_or_else(|| panic!("{name} is not listed: {:?}", app.view().rows))
}

/// Move the cursor onto the row named `name` in the active level: `home`
/// first, so this lands on `name` wherever the cursor already was -- a
/// second call to an earlier row still gets there, rather than pressing `j`
/// zero times and silently staying put -- then `j` at a time, and settle
/// once at the end.
fn cursor_to(app: &mut App, fk: &fake::Fake, name: &str) {
    app.key(code(KeyCode::Home));
    settle(app, fk);
    let idx = row_index(app, name);
    for _ in 0..idx {
        app.key(key('j'));
    }
    settle(app, fk);
}

/// `l` into the entry the cursor is already on, and settle.
fn enter(app: &mut App, fk: &fake::Fake) {
    app.key(key('l'));
    settle(app, fk);
}

/// `y`: queue a copy of whatever is already marked into the active
/// directory, run it, and set the resulting op's progress by hand to
/// `done`/`total` -- the shared setup behind the progress and the
/// quit-with-running-confirm snapshots. The caller marks its own entries and
/// settles into the destination directory first, the same order a real `y`
/// needs: the mark has to land while the marked entry's own level is active.
fn running_op(app: &mut App, fk: &fake::Fake, done: u64, total: u64) {
    app.key(key('y'));
    settle(app, fk);

    let op_id = fk.state().queue.iter().next().expect("one queued op").id;
    {
        let mut state = fk.state_mut();
        let op = state
            .queue
            .get_mut(op_id)
            .expect("the op is still in the queue");
        op.status = OpStatus::Running;
        op.progress.set_total(total);
        op.progress.add(done);
        state.version += 1;
    }
    app.tick();
}

// -- the column, and the floor --------------------------------------------

#[test]
fn the_column_with_the_fixture_loaded() {
    let (mut app, _fk) = build("terminal");
    insta::assert_snapshot!("column-terminal-100x30", render(&mut app, 100, 30));
    // The floor: every module at its minimum and the status row.
    insta::assert_snapshot!("column-terminal-60x21", render(&mut app, 60, 21));

    let (mut app, _fk) = build("catppuccin-mocha");
    insta::assert_snapshot!("column-mocha-100x30", render(&mut app, 100, 30));
}

/// The two ways below the floor, each of which draws one line and nothing
/// else: a row short, and a column short.
#[test]
fn a_terminal_below_the_floor_draws_the_size_message() {
    let (mut app, _fk) = build("terminal");
    insta::assert_snapshot!("floor-59x21", render(&mut app, 59, 21));
    insta::assert_snapshot!("floor-60x20", render(&mut app, 60, 20));
}

// -- the stack -------------------------------------------------------------

/// `l` into `projects/` then into `starwire/`: three levels, two of them
/// folded to crumb rows. Drawn at both sizes from the same app, since it is
/// the same scenario -- only the room to show the crumbs in differs.
#[test]
fn drilling_two_levels_deep() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "projects");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "starwire");
    enter(&mut app, &fk);

    insta::assert_snapshot!("deep-terminal-100x30", render(&mut app, 100, 30));
    insta::assert_snapshot!("deep-squeezed-terminal-100x24", render(&mut app, 100, 24));
}

/// `/` then typing narrows the rows to whatever matches, and the rule row
/// carries the live query.
#[test]
fn filtering_narrows_the_rows() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "projects");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "starwire");
    enter(&mut app, &fk);

    app.key(key('/'));
    for c in "car".chars() {
        app.key(key(c));
    }
    settle(&mut app, &fk);

    insta::assert_snapshot!("filter-terminal-100x30", render(&mut app, 100, 30));
}

/// `space` twice marks two entries, and the status says how many.
#[test]
fn two_marks_show_in_the_status() {
    let (mut app, fk) = build("terminal");
    app.key(key(' '));
    app.key(key('j'));
    app.key(key(' '));
    settle(&mut app, &fk);

    insta::assert_snapshot!("marked-terminal-100x30", render(&mut app, 100, 30));
}

/// Two marks in `starwire/`, `alt+up` to the parent without losing the child
/// frame, then `y` queues a copy of them into the level jumped back to.
/// Focusing OPERATIONS (`alt+3`) opens the module so the queued row shows.
#[test]
fn marking_jumping_up_and_copying_queues_an_operation() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "projects");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "starwire");
    enter(&mut app, &fk);

    app.key(key(' '));
    app.key(key('j'));
    app.key(key(' '));
    settle(&mut app, &fk);

    app.key(alt_code(KeyCode::Up));
    settle(&mut app, &fk);

    app.key(key('y'));
    settle(&mut app, &fk);

    app.key(alt('3'));
    settle(&mut app, &fk);

    insta::assert_snapshot!("queue-terminal-100x30", render(&mut app, 100, 30));
}

/// An operation `Running` at 78%, its counters set by hand through
/// [`fake::Fake::state_mut`] -- no real copy sits at a chosen percentage
/// long enough to draw. OPERATIONS is focused so its own row, with the bar,
/// is on screen alongside the status row's.
#[test]
fn an_operation_running_shows_its_percentage() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key(' '));
    settle(&mut app, &fk);
    cursor_to(&mut app, &fk, "empty");
    enter(&mut app, &fk);

    running_op(&mut app, &fk, 78, 100);

    app.key(alt('3'));
    settle(&mut app, &fk);

    insta::assert_snapshot!("progress-terminal-100x30", render(&mut app, 100, 30));
}

// -- overlays ---------------------------------------------------------------

/// A second copy of `blob.bin` into `empty/`, after the first one already
/// landed there: the plan finds a real collision and the queue stops at
/// `NeedsPolicy`, which is what opens the conflict overlay -- nothing here
/// is synthesised.
#[test]
fn the_conflict_overlay_opens_when_a_second_copy_collides_with_the_first() {
    let (mut app, fk) = build("terminal");

    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key(' '));
    settle(&mut app, &fk);
    cursor_to(&mut app, &fk, "empty");
    enter(&mut app, &fk);
    app.key(key('y'));
    settle(&mut app, &fk);
    app.key(key('X'));
    settle(&mut app, &fk);
    settle(&mut app, &fk);

    app.key(key('h'));
    settle(&mut app, &fk);
    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key(' '));
    settle(&mut app, &fk);
    cursor_to(&mut app, &fk, "empty");
    enter(&mut app, &fk);
    app.key(key('y'));
    settle(&mut app, &fk);
    app.key(key('X'));
    settle(&mut app, &fk);
    settle(&mut app, &fk);

    insta::assert_snapshot!("conflict-terminal-100x30", render(&mut app, 100, 30));
}

/// `r` on the cursor entry opens the rename field, pre-loaded with its name.
#[test]
fn the_rename_overlay_opens_on_the_cursor_entry() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key('r'));

    insta::assert_snapshot!("rename-terminal-100x30", render(&mut app, 100, 30));
}

/// `q` while an operation is running asks first, rather than quitting out
/// from under it.
#[test]
fn the_confirm_overlay_opens_when_quitting_with_a_running_operation() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key(' '));
    settle(&mut app, &fk);
    cursor_to(&mut app, &fk, "empty");
    enter(&mut app, &fk);

    running_op(&mut app, &fk, 40, 100);

    app.key(key('q'));

    insta::assert_snapshot!("confirm-terminal-100x30", render(&mut app, 100, 30));
}

// -- preview -----------------------------------------------------------------

#[test]
fn preview_text_shows_the_head_of_the_file() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "projects");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "starwire");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "README.md");
    settle(&mut app, &fk);

    insta::assert_snapshot!("preview-text", render(&mut app, 100, 30));
}

#[test]
fn preview_dir_shows_a_summary() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "projects");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "starwire");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "src");
    settle(&mut app, &fk);

    insta::assert_snapshot!("preview-dir", render(&mut app, 100, 30));
}

#[test]
fn preview_binary_shows_a_hexdump() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "blob.bin");
    settle(&mut app, &fk);

    insta::assert_snapshot!("preview-binary", render(&mut app, 100, 30));
}

#[test]
fn preview_symlink_shows_its_target() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "notes.txt");
    settle(&mut app, &fk);

    insta::assert_snapshot!("preview-symlink", render(&mut app, 100, 30));
}

/// With graphics disabled, an image preview falls back to half blocks --
/// [`starkit::graphics::halfblocks`], asserted directly since a `▀` is the
/// one thing that says a picture, rather than a placeholder, was drawn.
/// PREVIEW is focused (`alt+2`) so it gets half the body to draw the
/// picture in.
#[test]
fn preview_image_falls_back_to_half_blocks() {
    let (mut app, fk) = build("terminal");
    cursor_to(&mut app, &fk, "pictures");
    enter(&mut app, &fk);
    cursor_to(&mut app, &fk, "harbour.png");
    settle(&mut app, &fk);
    app.key(alt('2'));
    settle(&mut app, &fk);

    let text = render(&mut app, 100, 30);
    assert!(text.contains('\u{2580}'), "no half block in:\n{text}");
    insta::assert_snapshot!("preview-image-blocks", text);
}

// -- help ---------------------------------------------------------------

#[test]
fn the_help_overlay() {
    let (mut app, _fk) = build("terminal");
    app.key(key('?'));
    insta::assert_snapshot!("help-terminal-100x30", render(&mut app, 100, 30));

    let (mut app, _fk) = build("catppuccin-mocha");
    app.key(key('?'));
    insta::assert_snapshot!("help-mocha-100x30", render(&mut app, 100, 30));
}

// -- operations ---------------------------------------------------------

/// Three distinct queued operations -- a copy, a move and a delete -- none
/// of them running, with OPERATIONS focused so the module is open.
#[test]
fn three_queued_operations_with_the_module_focused() {
    let (mut app, fk) = build("terminal");

    cursor_to(&mut app, &fk, "blob.bin");
    app.key(key(' '));
    settle(&mut app, &fk);
    app.key(key('y'));
    settle(&mut app, &fk);
    app.key(key('u'));
    settle(&mut app, &fk);

    cursor_to(&mut app, &fk, "notes.txt");
    app.key(key(' '));
    settle(&mut app, &fk);
    app.key(key('m'));
    settle(&mut app, &fk);
    app.key(key('u'));
    settle(&mut app, &fk);

    cursor_to(&mut app, &fk, "dangling");
    app.key(key('d'));
    settle(&mut app, &fk);

    // PREVIEW is served its room first (see `ui/layout.rs`'s module doc) and
    // stays open, unfocused, by default -- at this height that alone eats
    // every spare row, leaving OPERATIONS pinned to its one-row floor
    // whatever is queued. Folding it (`i`) is what actually lets the module
    // grow to show all three rows.
    app.key(key('i'));
    settle(&mut app, &fk);
    app.key(alt('3'));
    settle(&mut app, &fk);

    insta::assert_snapshot!("operations-open-terminal-100x30", render(&mut app, 100, 30));
}
