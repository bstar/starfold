//! One table describing every action, its keys and its help text.
//!
//! Copied from STAR/CORD's `ui/keymap.rs`, which explains the design in full;
//! this is the same three-layer read (a module's own bindings, then the
//! global table, then nothing) over three modules instead of five, with two
//! more layers above it that this module supplies the primitives for but
//! does not run itself.
//!
//! ## Dispatch order
//!
//! A key press reaches its action through five layers, tried in order: an
//! open overlay takes every key while it is up (`ui/overlays`, Phase 2d);
//! the `/` filter's text entry takes typing next ([`filter_eats`]); a `g`
//! waiting for its second key comes next ([`g_prefix`]); the focused
//! module's own bindings are offered the key after that ([`module`]); and
//! the global table ([`resolve`]) catches whatever nothing above wanted. The
//! first three layers are stateful -- is an overlay open, does the filter
//! have focus, was `g` just pressed -- and that state lives in `ui/app.rs`
//! (Phase 3a), not here: everything in this module is a pure function of one
//! `KeyEvent`.
//!
//! ## `gh` and `gr` are in the table now
//!
//! `gh` (home directory) and `gr` (root) are two-key sequences dispatched by
//! [`g_prefix`], alongside `gg` (top of the list). An earlier draft left `gh`
//! and `gr` out of [`BINDINGS`] altogether, because the "every binding
//! reaches its action" test below could not tell a chord from an
//! unparseable mistake. They are real bindings the help overlay ought to
//! list, though, so the fix is in the test rather than in the table: a
//! spelling of the exact shape `g<char>` -- `gg`, `gh`, `gr` -- is recognised
//! as a chord and checked against [`g_prefix`] instead of
//! [`starkit::keymap::KeySpec::parse`], which would (correctly) refuse to
//! parse two characters as one key. That refusal is also what keeps `gh` and
//! `gr` out of [`GLOBAL`] and [`MODULES`]: `starkit::keymap::Keymap::
//! from_table` drops whatever alternative it cannot parse rather than
//! panicking on it, so the chord spellings sit in [`BINDINGS`] for the help
//! overlay and contribute no dispatch entry of their own -- `g_prefix` is
//! the only way to reach [`Action::GoHome`] and [`Action::GoRoot`].
//!
//! ## Two scopes, one group split in two
//!
//! The plan's Keys section describes the OPERATIONS queue's actions in two
//! scopes at once -- `y`/`p`/`m`/`d`/`X`/`ctrl+x` reach from anywhere, while
//! `enter`/`r`/`x`/`esc`/`c` only mean "run this op", "drop this one" and "clear the
//! queue" while the OPERATIONS module has focus -- and [`Scope`] is a
//! property of a *group*, not of one binding, the same as in STAR/CORD. Two
//! scopes cannot share one group, so the global queue actions are the
//! `"operations"` group and the module's own three keys are `"queue"`; both
//! are documented under those names rather than one.
//!
//! Those three module keys deliberately reuse a global key -- `enter` and
//! `esc` -- to mean something more specific while OPERATIONS has focus. That
//! is the only place it happens: [`SHADOWS`] lists the two on purpose, with
//! why, and a test asserts nothing else in any module shadows a global
//! binding by accident.
//!
//! Ordinary PREVIEW content scrolls with the global navigation keys. Its
//! `c` header mnemonic closes the panel only while Preview has focus. While
//! an embedded player is pinned there, `App::audio_key` adds transport keys
//! only when Preview has focus, after overlays/filter dispatch and before
//! this static table. Global focus, quit, themes, and `h` parent navigation
//! still fall through. The generated documentation includes those controls.
//!
//! ## Invariants
//!
//! Carried over from STAR/CORD, each one a test at the bottom of this file:
//! a bare arrow moves one and a shifted one moves ten; every key in the table
//! can be spelled in the table, chords included, via `g_prefix`; `hjkl`
//! navigates and never adjusts a value; `esc` never quits; a label is at most
//! 19 characters; every group appears in one run of the table; no module key
//! shadows a global one except the ones in [`SHADOWS`].

use std::sync::LazyLock;

use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::keymap::{Binding as KitBinding, Keymap, MouseHelp};

pub type Binding = KitBinding<Action>;

/// Everything the UI can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // -- moving about --
    CursorUp,
    CursorDown,
    CursorUpBig,
    CursorDownBig,
    PageUp,
    PageDown,
    /// `gg`/`home`: to the top of the current listing -- the list, not the
    /// filesystem. See [`Action::GoHome`] for the directory called home.
    Home,
    End,
    Activate,
    Back,

    // -- focus --
    FocusNext,
    FocusPrev,
    FocusStack,
    FocusPreview,
    FocusOperations,

    // -- the stack --
    /// Into the entry under the cursor, if it is a directory.
    Enter,
    /// Back one level.
    Pop,
    JumpUp,
    JumpDown,
    /// `gh`: to the home directory (`~`). Not to be confused with
    /// [`Action::Home`], which moves the cursor to the top of the list.
    GoHome,
    /// `gr`: to the filesystem root (`/`).
    GoRoot,
    OpenExternal,
    Rename,
    FileActions,
    Reload,

    // -- selection --
    Mark,
    MarkAll,
    InvertMarks,
    ClearMarks,

    // -- operations, reachable from anywhere --
    QueueCopy,
    QueueMove,
    QueueDelete,
    RunQueue,
    /// `ctrl+x`: stop the running op. Not `ctrl+c` -- that quits, see
    /// [`Action::Quit`] -- so a slip of the finger during a long copy does
    /// not kill the whole application.
    CancelRun,

    // -- the operations queue's own keys --
    RunOp,
    DropOp,
    ClearQueue,

    // -- view --
    ToggleView,
    Places,
    Bookmark,
    TogglePreview,
    ToggleHidden,
    NextSortKey,
    ReverseSort,
    Filter,
    /// `z`: the next of the preview's three picture scales. Global, like
    /// every other `view` key.
    NextPictureScale,

    // -- appearance --
    NextTheme,
    PrevTheme,

    // -- application --
    Help,
    /// `q`/`ctrl+c`: quit. `ctrl+c` is bound here, not to cancel, because
    /// that is the terminal habit everywhere else; the running op's cancel
    /// key is `ctrl+x` ([`Action::CancelRun`]), deliberately a different key
    /// so the two are never confused under one's fingers.
    Quit,
    Redraw,
}

/// Which module a key is offered to first.
///
/// Mirrors [`ModuleId`](super::panels::ModuleId) and is its own type so this
/// module does not depend on the panels, which depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Module {
    Stack,
    Preview,
    Operations,
}

/// Where a group of bindings applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Modules(&'static [Module]),
}

/// Every group in [`BINDINGS`], and where it applies. See the module doc for
/// why the operations queue is two groups rather than one. Preview has a
/// scoped `c` binding for its header's close action.
pub const GROUPS: &[(&str, Scope)] = &[
    ("navigation", Scope::Global),
    ("stack", Scope::Modules(&[Module::Stack])),
    ("selection", Scope::Modules(&[Module::Stack])),
    ("operations", Scope::Global),
    ("queue", Scope::Modules(&[Module::Operations])),
    ("preview", Scope::Modules(&[Module::Preview])),
    ("view", Scope::Global),
    ("appearance", Scope::Global),
    ("application", Scope::Global),
];

/// Every key, in the order the help overlay prints them.
pub const BINDINGS: &[Binding] = &[
    // -- navigation ----------------------------------------------------
    Binding {
        action: Action::FocusNext,
        keys: "tab",
        label: "next module",
        group: "navigation",
    },
    Binding {
        action: Action::FocusPrev,
        keys: "shift+tab",
        label: "previous module",
        group: "navigation",
    },
    Binding {
        action: Action::FocusStack,
        keys: "alt+1",
        label: "the stack",
        group: "navigation",
    },
    Binding {
        action: Action::FocusPreview,
        keys: "alt+2",
        label: "the preview",
        group: "navigation",
    },
    Binding {
        action: Action::FocusOperations,
        keys: "alt+3",
        label: "operations",
        group: "navigation",
    },
    Binding {
        action: Action::CursorUp,
        keys: "up/k",
        label: "up one",
        group: "navigation",
    },
    Binding {
        action: Action::CursorDown,
        keys: "down/j",
        label: "down one",
        group: "navigation",
    },
    Binding {
        action: Action::CursorUpBig,
        keys: "shift+up/K",
        label: "up ten",
        group: "navigation",
    },
    Binding {
        action: Action::CursorDownBig,
        keys: "shift+down/J",
        label: "down ten",
        group: "navigation",
    },
    Binding {
        action: Action::PageUp,
        keys: "pgup",
        label: "page up",
        group: "navigation",
    },
    Binding {
        action: Action::PageDown,
        keys: "pgdn",
        label: "page down",
        group: "navigation",
    },
    Binding {
        action: Action::Home,
        keys: "home/gg",
        label: "to the top",
        group: "navigation",
    },
    Binding {
        action: Action::End,
        keys: "end/G",
        label: "to the bottom",
        group: "navigation",
    },
    Binding {
        action: Action::Activate,
        keys: "enter",
        label: "open",
        group: "navigation",
    },
    Binding {
        action: Action::Back,
        keys: "esc",
        label: "cancel",
        group: "navigation",
    },
    // -- stack -----------------------------------------------------------
    Binding {
        action: Action::Enter,
        keys: "l/right",
        label: "into the directory",
        group: "stack",
    },
    Binding {
        action: Action::Pop,
        keys: "h/left/bs",
        label: "parent directory",
        group: "stack",
    },
    Binding {
        action: Action::JumpUp,
        keys: "alt+up",
        label: "jump to the parent",
        group: "stack",
    },
    Binding {
        action: Action::JumpDown,
        keys: "alt+down",
        label: "jump back down",
        group: "stack",
    },
    // `gh`/`gr` are chords, spelled but not parsed as one key -- see the
    // module doc's note on why they are still in this table.
    Binding {
        action: Action::GoHome,
        keys: "gh",
        label: "the home directory",
        group: "stack",
    },
    Binding {
        action: Action::GoRoot,
        keys: "gr",
        label: "the root",
        group: "stack",
    },
    Binding {
        action: Action::OpenExternal,
        keys: "o",
        label: "open externally",
        group: "stack",
    },
    Binding {
        action: Action::Rename,
        keys: "r",
        label: "rename",
        group: "stack",
    },
    Binding {
        action: Action::FileActions,
        keys: "c",
        label: "file actions menu",
        group: "stack",
    },
    Binding {
        action: Action::Reload,
        keys: "F5/ctrl+r",
        label: "reload",
        group: "stack",
    },
    // -- selection ---------------------------------------------------------
    Binding {
        action: Action::Mark,
        keys: "space",
        label: "mark, move down",
        group: "selection",
    },
    Binding {
        action: Action::MarkAll,
        keys: "a",
        label: "mark all here",
        group: "selection",
    },
    Binding {
        action: Action::InvertMarks,
        keys: "A",
        label: "invert the marks",
        group: "selection",
    },
    Binding {
        action: Action::ClearMarks,
        keys: "u",
        label: "unmark everything",
        group: "selection",
    },
    // -- operations, reachable from anywhere --------------------------------
    Binding {
        action: Action::QueueCopy,
        keys: "y/p",
        label: "copy marked here",
        group: "operations",
    },
    Binding {
        action: Action::QueueMove,
        keys: "m",
        label: "move marked here",
        group: "operations",
    },
    Binding {
        action: Action::QueueDelete,
        keys: "d",
        label: "delete marked",
        group: "operations",
    },
    Binding {
        action: Action::RunQueue,
        keys: "X",
        label: "run the queue",
        group: "operations",
    },
    Binding {
        action: Action::CancelRun,
        keys: "ctrl+x",
        label: "stop the running op",
        group: "operations",
    },
    // -- the operations queue's own keys -------------------------------------
    Binding {
        action: Action::RunOp,
        keys: "enter/r",
        label: "run it",
        group: "queue",
    },
    Binding {
        action: Action::DropOp,
        keys: "x/delete",
        label: "drop one",
        group: "queue",
    },
    Binding {
        action: Action::ClearQueue,
        keys: "esc/c",
        label: "clear the queue",
        group: "queue",
    },
    // -- the preview's own header key ---------------------------------------
    Binding {
        action: Action::TogglePreview,
        keys: "c",
        label: "close preview",
        group: "preview",
    },
    // -- view ----------------------------------------------------------------
    Binding {
        action: Action::ToggleView,
        keys: "v",
        label: "fold / commander",
        group: "view",
    },
    Binding {
        action: Action::Places,
        keys: "b/e",
        label: "places",
        group: "view",
    },
    Binding {
        action: Action::Bookmark,
        keys: "B",
        label: "bookmark directory",
        group: "view",
    },
    Binding {
        action: Action::TogglePreview,
        keys: "i",
        label: "show, fold preview",
        group: "view",
    },
    Binding {
        action: Action::ToggleHidden,
        keys: "./n",
        label: "hidden files",
        group: "view",
    },
    Binding {
        action: Action::NextSortKey,
        keys: "s",
        label: "next sort key",
        group: "view",
    },
    Binding {
        action: Action::ReverseSort,
        keys: "S",
        label: "reverse the sort",
        group: "view",
    },
    Binding {
        action: Action::Filter,
        keys: "f, /",
        label: "filter this level",
        group: "view",
    },
    Binding {
        action: Action::NextPictureScale,
        keys: "z",
        label: "picture scale",
        group: "view",
    },
    // -- appearance ------------------------------------------------------------
    Binding {
        action: Action::NextTheme,
        keys: "t",
        label: "next theme",
        group: "appearance",
    },
    Binding {
        action: Action::PrevTheme,
        keys: "T",
        label: "previous theme",
        group: "appearance",
    },
    // -- application -------------------------------------------------------
    Binding {
        action: Action::Help,
        keys: "?/F1",
        label: "this list",
        group: "application",
    },
    Binding {
        action: Action::Redraw,
        keys: "ctrl+l",
        label: "redraw the screen",
        group: "application",
    },
    Binding {
        action: Action::Quit,
        keys: "q/ctrl+c",
        label: "quit",
        group: "application",
    },
];

/// What the mouse does.
pub const MOUSE: &[MouseHelp] = &[
    MouseHelp {
        gesture: "click",
        label: "move the cursor",
        group: "stack",
    },
    MouseHelp {
        gesture: "double-click",
        label: "open it",
        group: "stack",
    },
    MouseHelp {
        gesture: "ctrl+click",
        label: "actions menu",
        group: "stack",
    },
    MouseHelp {
        gesture: "right-click",
        label: "actions if enabled",
        group: "stack",
    },
    MouseHelp {
        gesture: "click a crumb",
        label: "jump there",
        group: "stack",
    },
    MouseHelp {
        gesture: "wheel",
        label: "scroll three rows",
        group: "stack",
    },
    MouseHelp {
        gesture: "click a fold",
        label: "open it",
        group: "modules",
    },
    MouseHelp {
        gesture: "click a word",
        label: "what it says",
        group: "modules",
    },
    MouseHelp {
        gesture: "click the bar",
        label: "open operations",
        group: "status",
    },
];

/// The global half of the table, keyed.
///
/// `Keymap::from_table` parses each binding's alternatives with
/// `KeySpec::parse` and silently drops whatever does not parse as one key
/// (logging it rather than panicking) -- see `starkit::keymap::Keymap::
/// from_table`. That is what lets `gh`'s and `gr`'s single, two-character
/// spelling sit in [`BINDINGS`] for the help overlay to print without ever
/// producing an entry here: `g_prefix` is the only way to reach them.
static GLOBAL: LazyLock<Keymap<Action>> = LazyLock::new(|| Keymap::from_table(&global_bindings()));

/// A module's own half, one map each, built once. The same tolerant parsing
/// as [`GLOBAL`] applies, which is why `gh` and `gr` do not turn up as
/// one-key bindings in the stack module's map either.
static MODULES: LazyLock<Vec<(Module, Keymap<Action>)>> = LazyLock::new(|| {
    ALL_MODULES
        .iter()
        .map(|&m| (m, Keymap::from_table(&module_bindings(m))))
        .collect()
});

const ALL_MODULES: &[Module] = &[Module::Stack, Module::Preview, Module::Operations];

/// One module binding that intentionally claims a key the global table
/// already uses, and why. `enter` and `esc` are how the plan describes the
/// OPERATIONS queue's own keys (see the module doc), and both happen to be
/// spellings the global navigation group also binds. Every other repeat
/// between a module's own bindings and the global table is a mistake rather
/// than a decision, which is what the invariant test below is for: it walks
/// every module's keys, and any overlap with the global table that is not
/// listed here fails it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shadow {
    pub module: Module,
    /// The key spelling, exactly as [`starkit::keymap::KeySpec::parse`]
    /// accepts it.
    pub keys: &'static str,
    pub module_action: Action,
    pub global_action: Action,
    pub reason: &'static str,
}

/// The complete list of deliberate shadows. See [`Shadow`].
pub const SHADOWS: &[Shadow] = &[
    Shadow {
        module: Module::Operations,
        keys: "enter",
        module_action: Action::RunOp,
        global_action: Action::Activate,
        reason: "enter in the queue runs the op under the cursor, not the \
                 global \"open\"",
    },
    Shadow {
        module: Module::Operations,
        keys: "esc",
        module_action: Action::ClearQueue,
        global_action: Action::Back,
        reason: "esc in the queue clears it outright rather than merely \
                 cancelling",
    },
];

fn scope_of(group: &str) -> Scope {
    GROUPS
        .iter()
        .find(|(name, _)| *name == group)
        .map(|(_, scope)| *scope)
        .unwrap_or(Scope::Global)
}

fn global_bindings() -> Vec<Binding> {
    BINDINGS
        .iter()
        .filter(|b| scope_of(b.group) == Scope::Global)
        .map(copy_binding)
        .collect()
}

fn module_bindings(m: Module) -> Vec<Binding> {
    BINDINGS
        .iter()
        .filter(|b| match scope_of(b.group) {
            Scope::Global => false,
            Scope::Modules(list) => list.contains(&m),
        })
        .map(copy_binding)
        .collect()
}

fn copy_binding(b: &Binding) -> Binding {
    Binding {
        action: b.action,
        keys: b.keys,
        label: b.label,
        group: b.group,
    }
}

/// The focused module's own bindings, tried first. `None` means the module
/// has no use for this key and the global table should have it.
pub fn module(m: Module, k: KeyEvent) -> Option<Action> {
    MODULES
        .iter()
        .find(|(id, _)| *id == m)
        .and_then(|(_, map)| map.resolve(k))
}

/// The global table, tried after the focused module has declined.
pub fn resolve(k: KeyEvent) -> Option<Action> {
    GLOBAL.resolve(k)
}

/// What the key after a `g` means, if `g` was the one before it.
///
/// `gg` to the top, `gh` home, `gr` root -- see the module doc for why these
/// three, and only these three, are not ordinary one-key entries in
/// [`GLOBAL`] or [`MODULES`]. This function is pure and stateless: it is
/// only ever the *second* key of the sequence, and whether a `g` is
/// currently pending -- whether to call this at all rather than [`module`]
/// or [`resolve`] -- is state `ui/app.rs` (Phase 3a) owns, one bool beside
/// its other per-frame state, not this module. That keeps every function
/// here a function of one `KeyEvent`, which is what makes them easy to test
/// in isolation, this one included.
pub fn g_prefix(k: KeyEvent) -> Option<Action> {
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    match k.code {
        KeyCode::Char('g') => Some(Action::Home),
        KeyCode::Char('h') => Some(Action::GoHome),
        KeyCode::Char('r') => Some(Action::GoRoot),
        _ => None,
    }
}

/// Whether `k`, with no `ctrl`/`alt`, is a key someone typing text would
/// expect to work: a character, or a plain editing motion.
///
/// Factored out of [`filter_eats`] because the two things a text field has
/// to decide -- "is this a character or motion" and "which control keys
/// does *this* field also use" -- are different questions; STAR/CORD's
/// composer answers the second one differently (it also eats plain `enter`,
/// which sends, where the filter's `enter` is a way out and is not eaten).
pub fn is_text_key(k: KeyEvent) -> bool {
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    matches!(
        k.code,
        KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
    )
}

/// Whether the `/` filter's text entry, having focus, takes this key as
/// typing rather than as a command.
///
/// Every `alt+…` falls through, the same rule and the same reason as STAR/
/// CORD's composer: closing the OPERATIONS module or switching the theme
/// while a filter is half-typed has to keep working. `ctrl+a`/`e`/`u`/`w`
/// are the line-editing motions the filter implements and nothing else is,
/// so `ctrl+l` (redraw) and every other `ctrl+…` fall through too. `?` is
/// the one plain character carved out of [`is_text_key`]'s rule: it is
/// [`Action::Help`], and help has to stay reachable while a filter is
/// half-typed the same as it does mid-sentence in STAR/CORD's composer, so
/// it is not typed into the filter the way every other character is. `esc`
/// and `enter` are the ways out and neither is a text key to begin with.
pub fn filter_eats(k: KeyEvent) -> bool {
    if k.modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    if k.code == KeyCode::Char('?') {
        return false;
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(
            k.code,
            KeyCode::Char('u') | KeyCode::Char('w') | KeyCode::Char('a') | KeyCode::Char('e')
        );
    }
    is_text_key(k)
}

/// What `docs/keys-and-mouse.md` says before the tables.
const HEADER: &str = "\
# Keys and the mouse

Every key STAR/FOLD knows, in the order the `?` overlay prints them. This
file is generated from the table in `src/ui/keymap.rs`, and a test fails if
the two disagree.

A key reaches its action through five layers, tried in order: an open
overlay takes every key while it is up; the `/` filter's text entry takes
typing next, because a letter typed into it is a letter, not a command;
`g` waiting for a second key (`gg`, `gh`, `gr`) comes next; the focused
module's own bindings are offered the key after that, so a binding under a
module heading works while that module has focus; and the global table
catches whatever nothing above wanted, which is what makes it work from
everywhere. While the filter has focus, every `alt+\u{2026}` falls through
it and so does `?`, so help and the appearance and panel keys stay reachable
mid-search; `esc` and `enter` are always the way out.

The header highlights each word's keyboard letter. `n` also toggles hidden
files, `f` also opens the filter, and `e` also opens Places. The existing
`.`, `/`, and `b` keys still work. Preview's `c` and Operations' `r`/`c` work only
when that module has focus. The back arrow keeps its `h` navigation key.
Press `c` with the Stack focused, or click `actions` in its heading, to open
the modal file actions menu.

In Commander view, `tab` and `shift+tab` switch file panes. `alt+1` focuses
the active pane; `alt+2` and `alt+3` reach preview and operations. `y/p` and
`m` queue files from the active pane into the opposite directory, using
current-directory marks or the highlighted file. `v` switches views, `b`
opens Places, and `B` bookmarks the current directory. In Places, type to
search, use arrows and `enter` to open a location, `F2` to rename a bookmark,
`F3` to inspect the selected place, `delete` to remove one after confirmation,
`F5` to rescan mounts, and `F6` to unmount a selected local drive after
confirmation.
";

/// The key table as `docs/keys-and-mouse.md`. Run with `STARFOLD_UPDATE_DOCS=1`
/// to rewrite the file the test below compares against.
pub fn document() -> String {
    let mut out = String::new();
    out.push_str(HEADER);

    let mut group = "";
    for b in BINDINGS {
        if b.group != group {
            group = b.group;
            let scope = match scope_of(group) {
                Scope::Global => "everywhere".to_string(),
                Scope::Modules(list) => {
                    let names: Vec<&str> = list.iter().map(|m| module_name(*m)).collect();
                    format!("in {}", names.join(", "))
                }
            };
            out.push_str(&format!("\n## {group}\n\n_{scope}_\n\n"));
            out.push_str("| key | what it does |\n|---|---|\n");
        }
        let keys = format!("`{}`", b.keys);
        out.push_str(&format!(
            "| {keys:<width$} | {} |\n",
            b.label,
            width = starkit::keymap::KEYS_COLUMN
        ));
    }

    out.push_str("\n## The mouse\n\n");
    out.push_str(
        "\
While an embedded STAR/AMP player has Preview focus (`alt+2`), `space` or
`enter` pauses/resumes, `[` / `]` selects the previous/next track, left/right
seeks five seconds, and `+` / `-` changes volume. `x` or `esc` stops the player
and restores ordinary previews; `i` closes Preview and stops playback. `o`
toggles graphical/text transport buttons; `shift+o` opens the playing track
externally. On newer STAR/AMP, `w` / `shift+w` cycles visualizers forward/back
and `d` cycles seek-bar styles; these choices persist separately for STAR/FOLD.
These controls are scoped to Preview;
browser navigation, marking, global focus shortcuts, and quit keep their
normal meanings. The player's transport, seek, and volume controls are also
clickable. Left-click a visualizer to cycle it; Ctrl+click a visualizer or
seek bar to cycle the seek style; wheel over a visualizer cycles it. Physical
right-click does the same when `[ui] right_click = true`.

",
    );
    out.push_str("| where | gesture | what it does |\n|---|---|---|\n");
    for m in MOUSE {
        out.push_str(&format!(
            "| {:<8} | {:<width$} | {} |\n",
            m.group,
            m.gesture,
            m.label,
            width = starkit::keymap::GESTURE_COLUMN
        ));
    }
    out
}

fn module_name(m: Module) -> &'static str {
    match m {
        Module::Stack => "the stack",
        Module::Preview => "the preview",
        Module::Operations => "operations",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starkit::keymap::KeySpec;

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn with(c: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(c, m)
    }

    fn specs(b: &Binding) -> Vec<KeySpec> {
        starkit::keymap::alternatives(b.keys)
            .filter_map(KeySpec::parse)
            .collect()
    }

    /// The second character of a `g<char>` chord spelling -- `gg`, `gh`,
    /// `gr` -- or `None` if `alt` is not shaped like one. Exactly two
    /// characters, the first a plain `g`: `KeySpec::parse` refuses this (it
    /// is not one key), and `g_prefix` is how it dispatches instead. See the
    /// module doc's note on `gh`/`gr`.
    fn chord_char(alt: &str) -> Option<char> {
        let mut chars = alt.chars();
        let first = chars.next()?;
        let second = chars.next()?;
        if chars.next().is_some() || first != 'g' {
            return None;
        }
        Some(second)
    }

    #[test]
    fn every_group_in_the_table_declares_its_scope() {
        for b in BINDINGS {
            assert!(
                GROUPS.iter().any(|(name, _)| *name == b.group),
                "the group {:?} is in the table and not in GROUPS",
                b.group
            );
        }
    }

    #[test]
    fn every_binding_group_is_listed_once() {
        let mut seen: Vec<&str> = Vec::new();
        let mut last = "";
        for b in BINDINGS {
            if b.group != last {
                assert!(
                    !seen.contains(&b.group),
                    "{} appears in two places in the table",
                    b.group
                );
                seen.push(b.group);
                last = b.group;
            }
        }
    }

    #[test]
    fn nothing_in_the_table_overruns_its_column() {
        for b in BINDINGS {
            assert!(
                b.keys.chars().count() <= 13,
                "{:?} leaves no gap before {:?}",
                b.keys,
                b.label
            );
            assert!(b.label.chars().count() <= 19, "{:?} would wrap", b.label);
        }
        for m in MOUSE {
            assert!(m.gesture.chars().count() <= 19, "{:?}", m.gesture);
            assert!(m.label.chars().count() <= 19, "{:?} would wrap", m.label);
        }
    }

    /// Every key in [`BINDINGS`] can be pressed and reaches its action --
    /// through [`resolve`]/[`module`] for an ordinary spelling, and through
    /// [`g_prefix`] for a `g<char>` chord ([`chord_char`]). A spelling that
    /// is neither is the "gg has no real key" mistake the chord rule exists
    /// to catch: this test still refuses a binding with no reachable
    /// alternative at all.
    #[test]
    fn every_binding_reaches_its_action() {
        for b in BINDINGS {
            let mut reachable = false;
            for alt in starkit::keymap::alternatives(b.keys) {
                if let Some(c) = chord_char(alt) {
                    assert_eq!(
                        g_prefix(plain(c)),
                        Some(b.action),
                        "{:?} ({:?}) is a chord and g_prefix does not dispatch it",
                        b.keys,
                        b.action
                    );
                    reachable = true;
                    continue;
                }
                let Some(spec) = KeySpec::parse(alt) else {
                    continue;
                };
                let k = KeyEvent::new(spec.code, spec.mods);
                let hit = match scope_of(b.group) {
                    Scope::Global => resolve(k) == Some(b.action),
                    Scope::Modules(list) => list.iter().any(|&m| module(m, k) == Some(b.action)),
                };
                reachable |= hit;
            }
            assert!(
                reachable,
                "{:?} ({:?}) is in the help and dispatches to nothing",
                b.keys, b.action
            );
        }
    }

    #[test]
    fn shift_is_the_big_step() {
        assert_eq!(resolve(code(KeyCode::Up)), Some(Action::CursorUp));
        assert_eq!(
            resolve(with(KeyCode::Up, KeyModifiers::SHIFT)),
            Some(Action::CursorUpBig)
        );
        assert_eq!(
            resolve(with(KeyCode::Down, KeyModifiers::SHIFT)),
            Some(Action::CursorDownBig)
        );
    }

    /// `hjkl` navigates and never adjusts a value: `j`/`k` reach the global
    /// cursor and no module claims them; `h`/`l` move into and out of a
    /// directory in the stack module, which is movement too.
    #[test]
    fn hjkl_only_ever_moves() {
        for &m in ALL_MODULES {
            for c in ['j', 'k'] {
                assert_eq!(module(m, plain(c)), None, "{m:?} claims {c:?}");
            }
            for c in ['h', 'l'] {
                let got = module(m, plain(c));
                assert!(
                    matches!(got, None | Some(Action::Enter) | Some(Action::Pop)),
                    "{m:?} binds {c:?} to {got:?}, which is not a movement"
                );
            }
        }
        assert_eq!(resolve(plain('j')), Some(Action::CursorDown));
        assert_eq!(resolve(plain('k')), Some(Action::CursorUp));
    }

    #[test]
    fn esc_never_quits() {
        assert_ne!(resolve(code(KeyCode::Esc)), Some(Action::Quit));
        for &m in ALL_MODULES {
            assert_ne!(module(m, code(KeyCode::Esc)), Some(Action::Quit), "{m:?}");
        }
        for b in BINDINGS {
            if b.action == Action::Quit {
                assert!(!b.keys.contains("esc"), "{:?}", b.keys);
            }
        }
    }

    #[test]
    fn every_alt_binding_survives_the_filter() {
        for b in BINDINGS {
            if scope_of(b.group) != Scope::Global {
                continue;
            }
            for s in specs(b) {
                if !s.mods.contains(KeyModifiers::ALT) {
                    continue;
                }
                let k = KeyEvent::new(s.code, s.mods);
                assert!(
                    !filter_eats(k),
                    "the filter eats {:?}, so {:?} cannot be reached while typing",
                    b.keys,
                    b.action
                );
                assert_eq!(resolve(k), Some(b.action));
            }
        }
    }

    #[test]
    fn no_module_has_a_key_twice() {
        for &m in ALL_MODULES {
            let bindings = module_bindings(m);
            let mut seen: Vec<(KeySpec, Action)> = Vec::new();
            for b in &bindings {
                for s in specs(b) {
                    if let Some((_, other)) = seen.iter().find(|(k, _)| *k == s) {
                        assert_eq!(
                            *other, b.action,
                            "{m:?} binds {s:?} to two different actions"
                        );
                    }
                    seen.push((s, b.action));
                }
            }
        }
    }

    #[test]
    fn no_global_key_means_two_things() {
        let mut seen: Vec<(KeySpec, Action)> = Vec::new();
        for b in global_bindings() {
            for s in specs(&b) {
                if let Some((_, other)) = seen.iter().find(|(k, _)| *k == s) {
                    panic!("{s:?} is both {other:?} and {:?}", b.action);
                }
                seen.push((s, b.action));
            }
        }
    }

    /// `g_prefix` is the second half of a chord: it is asked "what does this
    /// key mean, given a `g` just came before it" and answers for exactly
    /// `g`/`h`/`r`, with no memory of its own of whether a `g` really did.
    #[test]
    fn the_g_prefix_reaches_home_and_the_two_go_actions() {
        assert_eq!(g_prefix(plain('g')), Some(Action::Home));
        assert_eq!(g_prefix(plain('h')), Some(Action::GoHome));
        assert_eq!(g_prefix(plain('r')), Some(Action::GoRoot));
        assert_eq!(g_prefix(plain('x')), None, "a mistake reaches nothing");
    }

    /// `ctrl`/`alt` on the second key is never part of the sequence -- it is
    /// some other binding's, not a mistyped chord.
    #[test]
    fn the_g_prefix_ignores_ctrl_and_alt() {
        assert_eq!(
            g_prefix(with(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(g_prefix(with(KeyCode::Char('r'), KeyModifiers::ALT)), None);
    }

    /// `g` on its own -- with no chord in progress -- resolves to nothing in
    /// either table, or the first half of `gg`/`gh`/`gr` would do something
    /// before the second half arrived.
    #[test]
    fn g_on_its_own_resolves_to_nothing() {
        assert_eq!(resolve(plain('g')), None);
        for &m in ALL_MODULES {
            assert_eq!(module(m, plain('g')), None, "{m:?}");
        }
    }

    /// `gh`/`gr` sit in [`BINDINGS`] for the help overlay, but a bare `h` or
    /// `r` -- no `g` first -- keeps its ordinary, unprefixed meaning in the
    /// stack module: the chord costs nothing to the keys it borrows from.
    #[test]
    fn chord_spellings_do_not_shadow_their_own_letters() {
        assert_eq!(module(Module::Stack, plain('h')), Some(Action::Pop));
        assert_eq!(module(Module::Stack, plain('r')), Some(Action::Rename));
    }

    #[test]
    fn plain_letters_are_typing_in_the_filter_and_esc_and_enter_are_not() {
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            if c == '?' {
                continue;
            }
            assert!(
                filter_eats(plain(c)),
                "{c:?} should be typed, not dispatched"
            );
        }
        assert!(!filter_eats(code(KeyCode::Esc)));
        assert!(!filter_eats(code(KeyCode::Enter)));
    }

    /// `?` is a plain character and every other one is typed into the
    /// filter, but this one is reserved for [`Action::Help`], which must
    /// stay reachable while a search is half-typed.
    #[test]
    fn the_filter_does_not_eat_question_mark() {
        assert!(!filter_eats(plain('?')));
        assert_eq!(resolve(plain('?')), Some(Action::Help));
    }

    /// The rest of what the module doc promises about `filter_eats`:
    /// `up`/`down`, `pgup`/`pgdn`, `tab`/`shift+tab` and `ctrl+l` all fall
    /// through it, alongside `esc`/`enter` and every `alt+…` covered
    /// elsewhere.
    #[test]
    fn the_filter_does_not_eat_the_navigation_and_redraw_keys() {
        for k in [
            code(KeyCode::Up),
            code(KeyCode::Down),
            code(KeyCode::PageUp),
            code(KeyCode::PageDown),
            code(KeyCode::Tab),
            code(KeyCode::BackTab),
            with(KeyCode::Char('l'), KeyModifiers::CONTROL),
        ] {
            assert!(!filter_eats(k), "{k:?} should fall through the filter");
        }
    }

    #[test]
    fn quitting_and_help_work_from_every_module() {
        for &m in ALL_MODULES {
            for (k, want) in [
                (plain('q'), Action::Quit),
                (plain('?'), Action::Help),
                (
                    with(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    Action::Quit,
                ),
            ] {
                let got = module(m, k).or_else(|| resolve(k));
                assert_eq!(got, Some(want), "{m:?} + {k:?}");
            }
        }
    }

    #[test]
    fn a_module_declines_what_it_does_not_want() {
        for &m in ALL_MODULES {
            for c in ['q', 't', 'T', '/'] {
                assert_eq!(
                    module(m, plain(c)),
                    None,
                    "{m:?} swallowed {c:?}, which is global"
                );
            }
        }
    }

    /// Preview owns only its close mnemonic; scrolling still uses global
    /// navigation, and its other controls are handled by the audio layer.
    #[test]
    fn the_preview_module_only_claims_its_close_mnemonic() {
        assert!(
            module_bindings(Module::Preview)
                .iter()
                .all(|b| b.action == Action::TogglePreview && b.keys == "c"),
            "preview should only add its focused close mnemonic"
        );
        assert_eq!(
            module(Module::Preview, plain('c')),
            Some(Action::TogglePreview)
        );
    }

    #[test]
    fn header_aliases_keep_existing_keys_and_panel_scopes() {
        for (new, old, action) in [
            ('n', '.', Action::ToggleHidden),
            ('f', '/', Action::Filter),
            ('e', 'b', Action::Places),
        ] {
            assert_eq!(resolve(plain(new)), Some(action));
            assert_eq!(resolve(plain(old)), Some(action));
            assert!(filter_eats(plain(new)), "{new} must type into a filter");
        }
        assert_eq!(resolve(plain('B')), Some(Action::Bookmark));
        assert_eq!(resolve(plain('s')), Some(Action::NextSortKey));
        assert_eq!(resolve(plain('S')), Some(Action::ReverseSort));
        assert_eq!(resolve(plain('v')), Some(Action::ToggleView));

        assert_eq!(module(Module::Stack, plain('h')), Some(Action::Pop));
        assert_eq!(module(Module::Stack, plain('r')), Some(Action::Rename));
        assert_eq!(module(Module::Operations, plain('r')), Some(Action::RunOp));
        assert_eq!(
            module(Module::Operations, plain('c')),
            Some(Action::ClearQueue)
        );
        assert_eq!(
            module(Module::Preview, plain('c')),
            Some(Action::TogglePreview)
        );
        assert_eq!(module(Module::Stack, plain('c')), Some(Action::FileActions));
        assert!(filter_eats(plain('c')), "c must type into a filter");
        assert_eq!(resolve(plain('c')), None);
        assert_eq!(resolve(plain('r')), None);
    }

    /// A module's own binding is allowed to claim a key the global table
    /// already uses only where [`SHADOWS`] says so. Every other repeat is
    /// the accident the rest of this file's tests exist to catch, one key at
    /// a time; this one checks the whole table at once.
    #[test]
    fn no_module_key_shadows_a_global_one_except_the_documented_shadows() {
        for &m in ALL_MODULES {
            for b in module_bindings(m) {
                for s in specs(&b) {
                    let k = KeyEvent::new(s.code, s.mods);
                    let Some(global_action) = resolve(k) else {
                        continue;
                    };
                    let documented = SHADOWS.iter().any(|sh| {
                        sh.module == m
                            && sh.module_action == b.action
                            && sh.global_action == global_action
                    });
                    assert!(
                        documented,
                        "{m:?} binds {s:?} to {:?}, shadowing the global {global_action:?}, \
                         and this is not in SHADOWS",
                        b.action
                    );
                }
            }
        }

        // And every declared shadow is a real overlap, not a stale entry.
        for sh in SHADOWS {
            let spec = KeySpec::parse(sh.keys).unwrap_or_else(|| panic!("{sh:?} does not parse"));
            let k = KeyEvent::new(spec.code, spec.mods);
            assert_eq!(module(sh.module, k), Some(sh.module_action), "{sh:?}");
            assert_eq!(resolve(k), Some(sh.global_action), "{sh:?}");
            assert!(!sh.reason.is_empty(), "{sh:?} has no reason");
        }
    }
}

#[cfg(test)]
mod doc_tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/keys-and-mouse.md")
    }

    #[test]
    fn the_document_is_the_table() {
        let want = document();
        let path = path();
        if std::env::var_os("STARFOLD_UPDATE_DOCS").is_some() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).expect("the docs directory");
            }
            std::fs::write(&path, &want).expect("writing the document");
            return;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have, want,
            "docs/keys-and-mouse.md is out of step with the key table; \
             run STARFOLD_UPDATE_DOCS=1 cargo test to rewrite it"
        );
    }

    #[test]
    fn every_key_and_gesture_is_in_the_document() {
        let text = std::fs::read_to_string(path()).unwrap_or_default();
        for b in BINDINGS {
            assert!(text.contains(b.keys), "{:?} is not in the document", b.keys);
            assert!(
                text.contains(b.label),
                "{:?} is not in the document",
                b.label
            );
        }
        for m in MOUSE {
            assert!(
                text.contains(m.gesture),
                "{:?} is not in the document",
                m.gesture
            );
            assert!(
                text.contains(m.label),
                "{:?} is not in the document",
                m.label
            );
        }
    }
}
