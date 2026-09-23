//! Commander behavior exercised through the same core commands as the UI.

use crate::fold::handle::Command;
use crate::fold::ops::{OpKind, OpStatus};
use crate::fold::FoldConfig;

use super::fake::{self, Fake};

fn select_row(handle: &crate::fold::Handle, fake: &Fake, name: &str) {
    let row = {
        let state = fake.state();
        let frame = state.active_frame();
        state
            .rows(frame)
            .iter()
            .position(|entry| entry.display == name)
            .unwrap_or_else(|| panic!("missing {name} in {}", frame.dir.display()))
    };
    handle.send(Command::CursorTo(row));
}

fn restore(handle: &crate::fold::Handle, fake: &Fake, left: &str, right: &str) {
    handle.send(Command::RestoreCommander {
        dirs: [fake.fixture.path(left), fake.fixture.path(right)],
        active: 0,
        enabled: true,
    });
    fake.pump();
}

fn visible_names(fake: &Fake, pane: usize) -> Vec<String> {
    let state = fake.state();
    let frame = state.tabs.active().stacks[pane + 1].active();
    state
        .rows(frame)
        .iter()
        .map(|entry| entry.display.clone())
        .collect()
}

#[test]
fn panes_keep_independent_marks_and_navigation_clears_only_current_marks() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    select_row(&handle, &fake, "blob.bin");
    handle.send(Command::ToggleMark);
    let fold_mark = fake.fixture.path("blob.bin");

    restore(&handle, &fake, "projects/starwire", "pictures");
    select_row(&handle, &fake, "README.md");
    handle.send(Command::ToggleMark);
    let left_mark = fake.fixture.path("projects/starwire/README.md");

    handle.send(Command::FocusPane(1));
    select_row(&handle, &fake, "harbour.png");
    handle.send(Command::ToggleMark);
    let right_mark = fake.fixture.path("pictures/harbour.png");
    {
        let state = fake.state();
        assert!(state.selection_for_stack(0).is_marked(&fold_mark));
        assert!(state.selection_for_stack(1).is_marked(&left_mark));
        assert!(state.selection_for_stack(2).is_marked(&right_mark));
    }

    handle.send(Command::Push(fake.fixture.path("empty")));
    fake.pump();
    {
        let state = fake.state();
        assert!(state.selection_for_stack(2).is_empty());
        assert!(state.selection_for_stack(1).is_marked(&left_mark));
        assert!(state.selection_for_stack(0).is_marked(&fold_mark));
    }
    handle.send(Command::ToggleView);
    assert!(fake.state().selection.is_marked(&fold_mark));
}

#[test]
fn queued_copy_uses_opposite_directory_even_after_pane_navigation() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    restore(&handle, &fake, "projects/starwire", "empty");
    select_row(&handle, &fake, "README.md");
    handle.send(Command::QueueCopyHere); // highlighted row fallback
    let source = fake.fixture.path("projects/starwire/README.md");
    let destination = fake.fixture.path("empty");
    {
        let state = fake.state();
        let op = state.queue.iter().next().unwrap();
        assert_eq!(op.kind, OpKind::Copy);
        assert_eq!(op.sources, vec![source.clone()]);
        assert_eq!(op.dest.as_deref(), Some(destination.as_path()));
    }

    handle.send(Command::FocusPane(1));
    handle.send(Command::Push(fake.fixture.path("pictures")));
    fake.pump();
    handle.send(Command::Run);
    fake.pump();

    assert!(destination.join("README.md").is_file());
    assert!(!fake.fixture.path("pictures/README.md").exists());
    let state = fake.state();
    assert_eq!(state.queue.iter().next().unwrap().status, OpStatus::Done);
}

#[test]
fn move_from_right_to_left_refreshes_both_listings() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    restore(&handle, &fake, "empty", "projects/starwire");
    handle.send(Command::FocusPane(1));
    select_row(&handle, &fake, "Cargo.toml");
    handle.send(Command::QueueMoveHere);
    let dest = fake.fixture.path("empty/Cargo.toml");
    assert_eq!(
        fake.state().queue.iter().next().unwrap().dest.as_deref(),
        dest.parent()
    );
    handle.send(Command::Run);
    fake.pump();

    assert!(dest.is_file());
    assert!(!fake.fixture.path("projects/starwire/Cargo.toml").exists());
    assert!(visible_names(&fake, 0).contains(&"Cargo.toml".to_string()));
    assert!(!visible_names(&fake, 1).contains(&"Cargo.toml".to_string()));
}

#[test]
fn frames_with_the_same_local_id_keep_separate_cursor_and_filter() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    restore(&handle, &fake, "projects/starwire", "projects/starwire");
    select_row(&handle, &fake, "README.md");
    handle.send(Command::SetFilter("README".into()));
    handle.send(Command::FocusPane(1));
    select_row(&handle, &fake, "Cargo.toml");
    let right_cursor = fake.state().active_frame().cursor;
    handle.send(Command::FocusPane(0));
    let state = fake.state();
    let left = state.tabs.active().stacks[1].active();
    let right = state.tabs.active().stacks[2].active();
    assert_eq!(left.id, right.id); // FrameId is local to its stack.
    assert_eq!(left.filter, "README");
    assert!(right.filter.is_empty());
    assert_eq!(right.cursor, right_cursor);
    assert_eq!(state.rows(left).len(), 1);
}

#[test]
fn unavailable_location_stays_visible_and_parent_navigation_recovers() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    restore(&handle, &fake, "empty", "pictures");
    let missing = fake.fixture.path("disconnected");
    handle.send(Command::Push(missing.clone()));
    fake.pump();
    {
        let state = fake.state();
        assert_eq!(state.active_frame().dir, missing);
        assert!(state.active_listing().unwrap().error.is_some());
        assert_eq!(
            state.tabs.active().stacks[2].active().dir,
            fake.fixture.path("pictures")
        );
    }
    handle.send(Command::Back);
    fake.pump();
    assert_eq!(fake.state().active_frame().dir, fake.home());
}

#[test]
fn commander_parent_navigation_uses_cached_parent_and_highlights_departed_child() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    restore(&handle, &fake, "empty", "pictures");
    handle.send(Command::Back);
    fake.pump();
    let state = fake.state();
    assert_eq!(state.active_frame().dir, fake.home());
    assert_eq!(state.cursor_entry().unwrap().display, "empty");
    assert_eq!(
        state.tabs.active().stacks[2].active().dir,
        fake.fixture.path("pictures")
    );
}

#[test]
fn switching_to_commander_while_listing_preserves_loading_in_both_panes() {
    let (handle, fake) = fake::handle(FoldConfig::default());
    handle.send(Command::Push(fake.fixture.path("pictures")));
    handle.send(Command::ToggleView);
    {
        let state = fake.state();
        assert!(state.tabs.active().stacks[1].active().loading);
        assert!(state.tabs.active().stacks[2].active().loading);
    }
    fake.pump();
    let state = fake.state();
    assert!(!state.tabs.active().stacks[1].active().loading);
    assert!(!state.tabs.active().stacks[2].active().loading);
}
