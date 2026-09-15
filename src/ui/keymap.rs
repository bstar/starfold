//! One table describing every action, its keys and its help text.
//!
//! Copied from STAR/CORD's `ui/keymap.rs`, which explains the design in full;
//! this is the same three-layer read (a module's own bindings, then the
//! global table, then nothing) over three modules instead of five.
//!
//! ## Where this plan's grouping had to bend
//!
//! The plan's Keys section describes the OPERATIONS queue's actions in two
//! scopes at once -- `y`/`p`/`m`/`d`/`X`/`ctrl+x` reach from anywhere, while
//! `enter`/`x`/`esc` only mean "run this op", "drop this one" and "clear the
//! queue" while the OPERATIONS module has focus -- and [`Scope`] is a
//! property of a *group*, not of one binding, the same as in STAR/CORD. Two
//! scopes cannot share one group, so the global queue actions are the
//! `"operations"` group and the module's own three keys are `"queue"`; both
//! are documented under those names rather than one.
//!
//! `gh` and `gr` (go home, go root) are two-key sequences dispatched by
//! [`g_prefix`] alongside `gg`, the same as STAR/CORD's. Unlike `gg`, which
//! is also an alternate spelling on a binding that has a real single-key
//! spelling of its own (`home`), `gh` and `gr` have no such spelling to ride
//! along on, so they are not in [`BINDINGS`] at all: a table entry whose only
//! key cannot be parsed as one key would fail the "every binding reaches its
//! action" invariant below. They are still real bindings -- `g_prefix` is
//! tested directly -- just not ones the generated help document lists.
//!
//! ## Invariants
//!
//! Carried over from STAR/CORD, each one a test at the bottom of this file:
//! a bare arrow moves one and a shifted one moves ten; every key in the table
//! can be spelled in the table; `hjkl` navigates and never adjusts a value;
//! `esc` never quits; a label is at most 19 characters; every group appears
//! in one run of the table.

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
    /// `gg`: to the top of the list. Not to be confused with `GoHome`.
    GoHome,
    GoRoot,
    OpenExternal,
    Rename,
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
    CancelRun,

    // -- the operations queue's own keys --
    RunOp,
    DropOp,
    ClearQueue,

    // -- view --
    TogglePreview,
    ToggleHidden,
    NextSortKey,
    ReverseSort,
    Filter,

    // -- appearance --
    NextTheme,
    PrevTheme,

    // -- application --
    Help,
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
/// why the operations queue is two groups rather than one.
pub const GROUPS: &[(&str, Scope)] = &[
    ("navigation", Scope::Global),
    ("stack", Scope::Modules(&[Module::Stack])),
    ("selection", Scope::Modules(&[Module::Stack])),
    ("operations", Scope::Global),
    ("queue", Scope::Modules(&[Module::Operations])),
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
        label: "back one level",
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
        keys: "enter",
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
        keys: "esc",
        label: "clear the queue",
        group: "queue",
    },
    // -- view ----------------------------------------------------------------
    Binding {
        action: Action::TogglePreview,
        keys: "i",
        label: "show, fold preview",
        group: "view",
    },
    Binding {
        action: Action::ToggleHidden,
        keys: ".",
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
        keys: "/",
        label: "filter this level",
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
        gesture: "right-click",
        label: "mark it",
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
static GLOBAL: LazyLock<Keymap<Action>> = LazyLock::new(|| Keymap::from_table(&global_bindings()));

/// A module's own half, one map each, built once.
static MODULES: LazyLock<Vec<(Module, Keymap<Action>)>> = LazyLock::new(|| {
    ALL_MODULES
        .iter()
        .map(|&m| (m, Keymap::from_table(&module_bindings(m))))
        .collect()
});

const ALL_MODULES: &[Module] = &[Module::Stack, Module::Preview, Module::Operations];

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

/// Where a `g`-prefixed sequence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixKey {
    Waiting,
    Action(Action),
    None,
}

/// `gg` to the top, `gh` home, `gr` root -- see the module doc for why these
/// three, and only these three, are not in [`BINDINGS`].
pub fn g_prefix(pending: &mut bool, k: KeyEvent) -> PrefixKey {
    let plain = !k
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    if *pending {
        *pending = false;
        if plain {
            if let KeyCode::Char(c) = k.code {
                match c {
                    'g' => return PrefixKey::Action(Action::Home),
                    'h' => return PrefixKey::Action(Action::GoHome),
                    'r' => return PrefixKey::Action(Action::GoRoot),
                    _ => {}
                }
            }
        }
        return PrefixKey::None;
    }
    if plain && k.code == KeyCode::Char('g') {
        *pending = true;
        return PrefixKey::Waiting;
    }
    PrefixKey::None
}

/// Whether the `/` filter's text entry, having focus, takes this key as
/// typing rather than as a command.
///
/// Every `alt+…` falls through, the same rule and the same reason as STAR/
/// CORD's composer: closing the OPERATIONS module or switching the theme
/// while a filter is half-typed has to keep working. `esc` and `enter` are
/// the ways out and neither is eaten.
pub fn filter_eats(k: KeyEvent) -> bool {
    if k.modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(
            k.code,
            KeyCode::Char('u') | KeyCode::Char('w') | KeyCode::Char('a') | KeyCode::Char('e')
        );
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

/// What `docs/keys-and-mouse.md` says before the tables.
const HEADER: &str = "\
# Keys and the mouse

Every key STAR/FOLD knows, in the order the `?` overlay prints them. This
file is generated from the table in `src/ui/keymap.rs`, and a test fails if
the two disagree.

A key is offered to the focused module first and to the global table second,
so a binding under a module heading works while that module has focus and
the global ones work from everywhere. While the `/` filter has focus it takes
raw keys, because a letter typed into it is a letter, not a command; every
`alt+\u{2026}` falls through it, and `esc` and `enter` are always the way out.
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

    out.push_str("\n## the mouse\n\n| where | gesture | what it does |\n|---|---|---|\n");
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

    #[test]
    fn every_binding_reaches_its_action() {
        for b in BINDINGS {
            let parsed = specs(b);
            assert!(
                !parsed.is_empty(),
                "{:?} has no key anything could press",
                b.keys
            );
            let reachable = parsed.iter().any(|s| {
                let k = KeyEvent::new(s.code, s.mods);
                match scope_of(b.group) {
                    Scope::Global => resolve(k) == Some(b.action),
                    Scope::Modules(list) => list.iter().any(|&m| module(m, k) == Some(b.action)),
                }
            });
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

    #[test]
    fn the_g_prefix_reaches_home_and_the_two_go_actions() {
        let mut pending = false;
        assert_eq!(g_prefix(&mut pending, plain('g')), PrefixKey::Waiting);
        assert_eq!(
            g_prefix(&mut pending, plain('g')),
            PrefixKey::Action(Action::Home)
        );
        assert!(!pending);

        assert_eq!(g_prefix(&mut pending, plain('g')), PrefixKey::Waiting);
        assert_eq!(
            g_prefix(&mut pending, plain('h')),
            PrefixKey::Action(Action::GoHome)
        );

        assert_eq!(g_prefix(&mut pending, plain('g')), PrefixKey::Waiting);
        assert_eq!(
            g_prefix(&mut pending, plain('r')),
            PrefixKey::Action(Action::GoRoot)
        );

        assert_eq!(g_prefix(&mut pending, plain('g')), PrefixKey::Waiting);
        assert_eq!(
            g_prefix(&mut pending, plain('x')),
            PrefixKey::None,
            "a mistake cancels the sequence"
        );
        assert!(!pending);
    }

    #[test]
    fn g_on_its_own_resolves_to_nothing() {
        assert_eq!(resolve(plain('g')), None);
        for &m in ALL_MODULES {
            assert_eq!(module(m, plain('g')), None, "{m:?}");
        }
    }

    #[test]
    fn plain_letters_are_typing_in_the_filter_and_esc_and_enter_are_not() {
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            assert!(
                filter_eats(plain(c)),
                "{c:?} should be typed, not dispatched"
            );
        }
        assert!(!filter_eats(code(KeyCode::Esc)));
        assert!(!filter_eats(code(KeyCode::Enter)));
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
