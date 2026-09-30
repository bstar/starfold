//! Switching workspaces is a pure state change; filesystem work stays on workers.
use super::*;
use crate::session::{SearchSession, TabSession};

impl State {
    pub fn can_reopen_tab(&self) -> bool {
        !self.closed_tabs.is_empty()
    }

    fn swap_context(&mut self, context: &mut TabContext) {
        macro_rules! swap { ($($field:ident),*) => { $(std::mem::swap(&mut self.$field, &mut context.$field);)* }; }
        swap!(
            commander,
            commander_pane,
            parked_selections,
            selection,
            marked_search_identities,
            search,
            search_generation,
            preview,
            preview_generation,
            sort,
            pane_sorts,
            show_hidden,
            loading,
            pending_search
        );
    }

    pub fn tab_label(&self, id: TabId) -> String {
        let Some(tab) = self.tabs.tabs.iter().find(|t| t.id == id) else {
            return "closed tab".into();
        };
        let name = tab.name.clone().unwrap_or_else(|| {
            let path = &tab.active_stack().active().dir;
            if path == &self.home {
                return "~".into();
            }
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned()
        });
        name.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }

    pub fn tab_snapshots(&self) -> Vec<TabSession> {
        self.tabs
            .tabs
            .iter()
            .map(|tab| {
                let (commander, pane, sort, pane_sorts, hidden, search, pending) =
                    if let Some(c) = &tab.context {
                        (
                            c.commander,
                            c.commander_pane,
                            c.sort,
                            c.pane_sorts,
                            c.show_hidden,
                            &c.search,
                            &c.pending_search,
                        )
                    } else {
                        (
                            self.commander,
                            self.commander_pane,
                            self.sort,
                            self.pane_sorts,
                            self.show_hidden,
                            &self.search,
                            &self.pending_search,
                        )
                    };
                TabSession {
                    id: tab.id.0,
                    name: tab.name.clone(),
                    stacks: tab.stacks.iter().map(Stack::snapshot).collect(),
                    active_stack: tab.active_stack,
                    commander,
                    commander_pane: pane,
                    sort,
                    pane_sorts,
                    show_hidden: hidden,
                    search: search
                        .as_ref()
                        .map(|s| SearchSession {
                            query: s.query.clone(),
                            contents: s.mode == search::Mode::Contents,
                            root: Some(s.root.clone()),
                            cursor: s.cursor,
                            scroll: s.view,
                            cursor_path: s
                                .results
                                .get(s.cursor)
                                .map(|e| e.path.clone())
                                .or_else(|| s.restore_cursor_path.clone()),
                        })
                        .or_else(|| pending.clone()),
                    preview_open: true,
                    ..Default::default()
                }
            })
            .collect()
    }

    pub fn tab_search_running(&self, id: TabId) -> bool {
        self.tabs
            .tabs
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| {
                if t.context.is_none() {
                    self.search.as_ref()
                } else {
                    t.context.as_ref().and_then(|c| c.search.as_ref())
                }
            })
            .is_some_and(|s| s.status == SearchStatus::Running)
    }
}

fn activate_only(state: &mut State, index: usize) {
    let old = state.tabs.index();
    if old == index {
        return;
    }
    state.preview_generation += 1;
    state.preview = None;
    let mut context = state.tabs.tabs[index]
        .context
        .take()
        .expect("inactive tab has context");
    state.swap_context(&mut context);
    state.tabs.tabs[old].context = Some(context);
    state.tabs.activate(index);
}

fn activate_jobs(state: &mut State) -> Effects {
    rebuild_all_frames(state);
    let mut dirs = vec![state.active_frame().dir.clone()];
    if state.commander {
        dirs.extend(
            state
                .tabs
                .active()
                .stacks
                .iter()
                .skip(1)
                .map(|s| s.active().dir.clone()),
        );
    }
    dirs.sort();
    dirs.dedup();
    let mut jobs = vec![Job::ClosePreview];
    for dir in dirs {
        jobs.push(Job::List(dir));
    }
    state.loading = !state.listings.contains_key(&state.active_frame().dir);
    let mut effects = Effects {
        jobs,
        events: vec![Event::Stack, Event::Selection, Event::Preview],
    };
    if let Some(saved) = state.pending_search.take() {
        let mut search = cmd_start_search(
            state,
            saved.query,
            if saved.contents {
                search::Mode::Contents
            } else {
                search::Mode::Names
            },
        );
        if let Some(current) = &mut state.search {
            current.cursor = saved.cursor;
            current.view = saved.scroll;
            current.restore_cursor_path = saved.cursor_path;
            if let Some(root) = saved.root {
                current.root = root.clone();
                for job in &mut search.jobs {
                    if let Job::Search { root: job_root, .. } = job {
                        *job_root = root.clone();
                    }
                }
            }
        }
        effects.jobs.append(&mut search.jobs);
    }
    effects
}

pub(super) fn switch_tab(state: &mut State, id: TabId) -> Effects {
    let Some(index) = state.tabs.tabs.iter().position(|t| t.id == id) else {
        return Effects::default();
    };
    if index == state.tabs.index() {
        return Effects::default();
    }
    activate_only(state, index);
    activate_jobs(state)
}

fn from_saved(state: &mut State, saved: &TabSession) -> Option<Tab> {
    let mut stacks: Vec<_> = saved
        .stacks
        .iter()
        .take(3)
        .filter_map(Stack::restore)
        .collect();
    if stacks.is_empty() {
        return None;
    }
    if saved.commander {
        while stacks.len() < 3 {
            stacks.push(Stack::new(stacks[0].active().dir.clone()));
        }
    }
    let mut context = TabContext::fresh(saved.sort, saved.show_hidden);
    context.commander = saved.commander;
    context.commander_pane = saved.commander_pane.min(1);
    context.pane_sorts = saved.pane_sorts;
    context.pending_search = saved.search.clone();
    let active_stack = if saved.commander {
        context.commander_pane + 1
    } else {
        0
    };
    Some(Tab {
        id: state.tabs.allocate_id(),
        stacks,
        active_stack,
        name: saved.name.clone(),
        context: Some(context),
    })
}

pub(super) fn new_tab(state: &mut State, duplicate: bool) -> Effects {
    state.durable_tabs = true;
    let mut saved = state.tab_snapshots().remove(state.tabs.index());
    if !duplicate {
        for stack in &mut saved.stacks {
            let dir = stack.frames[stack.active].dir.clone();
            *stack = Stack::new(dir).snapshot();
        }
        saved.name = None;
        saved.search = None;
    }
    let mut tab = from_saved(state, &saved).expect("active tab has stacks");
    if duplicate {
        tab.stacks = state.tabs.active().stacks.clone();
    }
    let index = state.tabs.index() + 1;
    state.tabs.tabs.insert(index, tab);
    activate_only(state, index);
    activate_jobs(state)
}

pub(super) fn close_tab(state: &mut State, id: TabId) -> Effects {
    let Some(index) = state.tabs.tabs.iter().position(|t| t.id == id) else {
        return Effects::default();
    };
    if state.tabs.tabs.len() == 1 {
        return Effects {
            jobs: vec![],
            events: vec![Event::Note(Note::info("Last tab stays open"))],
        };
    }
    let snapshot = state.tab_snapshots().remove(index);
    state.closed_tabs.push(snapshot);
    if state.closed_tabs.len() > 10 {
        state.closed_tabs.remove(0);
    }
    let was_active = index == state.tabs.index();
    if was_active {
        if let Some(search) = &state.search {
            search
                .progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        activate_only(
            state,
            if index + 1 < state.tabs.tabs.len() {
                index + 1
            } else {
                index - 1
            },
        );
    }
    let tab = state.tabs.tabs.remove(index);
    if let Some(search) = tab.context.and_then(|c| c.search) {
        search
            .progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let active = state.tabs.index();
    state
        .tabs
        .activate(if active > index { active - 1 } else { active });
    evict_unreferenced_listings(state);
    if was_active {
        activate_jobs(state)
    } else {
        Effects {
            jobs: vec![],
            events: vec![Event::Stack],
        }
    }
}

pub(super) fn reopen_tab(state: &mut State) -> Effects {
    let Some(saved) = state.closed_tabs.pop() else {
        return Effects::default();
    };
    let Some(tab) = from_saved(state, &saved) else {
        return Effects::default();
    };
    let index = state.tabs.tabs.len();
    state.tabs.tabs.push(tab);
    activate_only(state, index);
    activate_jobs(state)
}

pub(super) fn restore_tabs(state: &mut State, saved: Vec<TabSession>, active: usize) -> Effects {
    state.durable_tabs = true;
    let mut tabs: Vec<_> = saved.iter().filter_map(|s| from_saved(state, s)).collect();
    if tabs.is_empty() {
        return Effects::default();
    }
    let active = active.min(tabs.len() - 1);
    let mut context = tabs[active].context.take().unwrap();
    state.swap_context(&mut context);
    state.tabs.replace(tabs, active);
    activate_jobs(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::{
        ops::{ConflictPolicy, DeleteHow, OpKind, OpStatus},
        testing::Fixture,
    };

    fn send(state: &mut State, command: Command) -> Effects {
        apply(state, Change::Command(command))
    }
    fn listed(state: &mut State, path: &Path) {
        let listing = crate::fold::listing::read(path, &FoldConfig::default().list);
        apply(state, Change::Done(Done::Listed(listing)));
    }
    fn fixture() -> (Fixture, State) {
        let f = Fixture::tree();
        let mut state = State::new(
            &FoldConfig::default(),
            f.home().to_path_buf(),
            f.home().to_path_buf(),
            true,
        );
        listed(&mut state, f.home());
        (f, state)
    }
    #[test]
    fn tabs_isolate_marks_filters_sort_hidden_and_commander() {
        let (f, mut state) = fixture();
        let first = state.tabs.active().id;
        send(&mut state, Command::ToggleMark);
        let marks: Vec<_> = state.selection.paths().map(Path::to_path_buf).collect();
        assert!(!marks.is_empty());
        send(&mut state, Command::SetHidden(true));
        let sort = SortOrder {
            reverse: true,
            ..Default::default()
        };
        send(&mut state, Command::SetSort(sort));
        send(&mut state, Command::SetFilter("star".into()));
        send(&mut state, Command::NewTab { duplicate: false });
        let second = state.tabs.active().id;
        assert_ne!(first, second);
        assert!(state.selection.is_empty());
        assert!(state.active_frame().filter.is_empty());
        send(&mut state, Command::SetHidden(false));
        send(&mut state, Command::SetSort(SortOrder::default()));
        send(&mut state, Command::ToggleView);
        send(&mut state, Command::FocusPane(1));
        send(&mut state, Command::Push(f.path("projects")));
        send(&mut state, Command::SwitchTab(first));
        assert!(!state.commander);
        assert!(state.show_hidden);
        assert_eq!(state.sort, sort);
        assert_eq!(state.active_frame().filter, "star");
        assert_eq!(
            state
                .selection
                .paths()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>(),
            marks
        );
        send(&mut state, Command::SwitchTab(second));
        assert!(state.commander);
        assert_eq!(state.commander_pane, 1);
        assert!(!state.show_hidden);
        assert!(state.selection.is_empty());
    }
    #[test]
    fn duplicate_and_reopen_keep_forward_trails_and_have_fresh_identities() {
        let (f, mut state) = fixture();
        send(&mut state, Command::Push(f.path("projects")));
        send(&mut state, Command::Push(f.path("projects/starfold")));
        send(&mut state, Command::JumpTo(0));
        let original = state.tabs.active().id;
        let trail = state.tabs.active().stacks[0].snapshot();
        send(&mut state, Command::NewTab { duplicate: true });
        let duplicate = state.tabs.active().id;
        assert_eq!(state.tabs.active().stacks[0].snapshot(), trail);
        send(&mut state, Command::CloseTab(duplicate));
        assert_eq!(state.tabs.active().id, original);
        send(&mut state, Command::ReopenTab);
        assert_ne!(state.tabs.active().id, duplicate);
        assert_eq!(state.tabs.active().stacks[0].snapshot(), trail);
        send(&mut state, Command::Forward);
        assert_eq!(state.active_frame().dir, f.path("projects"));
        let id = state.tabs.active().id;
        send(&mut state, Command::CloseTab(id));
        send(&mut state, Command::CloseTab(original));
        assert_eq!(state.tabs.tabs.len(), 1);
    }
    #[test]
    fn background_search_is_delivered_to_owner_and_closed_results_are_discarded() {
        let (f, mut state) = fixture();
        let first = state.tabs.active().id;
        send(&mut state, Command::StartSearch("star".into()));
        let generation = state.search.as_ref().unwrap().generation;
        let found = Arc::new(search::scan(
            f.home(),
            "star",
            false,
            &search::Progress::default(),
        ));
        assert!(!found.entries.is_empty());
        send(&mut state, Command::NewTab { duplicate: false });
        let second = state.tabs.active().id;
        apply(
            &mut state,
            Change::Done(Done::Searched {
                tab: first,
                generation,
                found: found.clone(),
            }),
        );
        assert!(state.search.is_none());
        assert!(!state.tabs.tabs[0]
            .context
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .results
            .is_empty());
        send(&mut state, Command::CloseTab(first));
        let effects = apply(
            &mut state,
            Change::Done(Done::Searched {
                tab: first,
                generation,
                found,
            }),
        );
        assert!(effects.events.is_empty());
        assert_eq!(state.tabs.active().id, second);
    }
    #[test]
    fn matching_preview_generation_cannot_cross_tab_identity() {
        let (_, mut state) = fixture();
        let first = state.tabs.active().id;
        send(&mut state, Command::NewTab { duplicate: false });
        let generation = state.preview_generation;
        let effects = apply(
            &mut state,
            Change::Done(Done::Previewed {
                tab: first,
                generation,
                path: "/old/image".into(),
                preview: Preview::Empty,
            }),
        );
        assert!(effects.events.is_empty());
        assert!(state.preview.is_none());
    }
    #[test]
    fn shared_copy_queue_locks_paths_after_owner_closes() {
        let (f, mut state) = fixture();
        let first = state.tabs.active().id;
        send(
            &mut state,
            Command::RenameTab(first, "source project".into()),
        );
        let source = f.path("README.md");
        let id = state.queue.enqueue(
            OpKind::Copy,
            vec![source.clone()],
            Some(f.path("projects")),
            ConflictPolicy::Ask,
        );
        let op = state.queue.get_mut(id).unwrap();
        op.status = OpStatus::Running;
        op.origin_tab = Some(first);
        op.origin_name = "source project".into();
        send(&mut state, Command::NewTab { duplicate: false });
        send(&mut state, Command::CloseTab(first));
        let count = state.queue.iter().count();
        let effects = send(
            &mut state,
            Command::QueueOperation {
                kind: OpKind::Delete(DeleteHow::Permanent),
                sources: vec![source],
                dest: None,
            },
        );
        assert_eq!(state.queue.iter().count(), count);
        assert!(effects
            .events
            .iter()
            .any(|e| matches!(e, Event::Note(note) if note.text.contains("locked"))));
        let op = state.queue.get_mut(id).unwrap();
        assert_eq!(op.status, OpStatus::Running);
        assert!(!op.progress.is_cancelled());
        assert_eq!(op.origin_name, "source project");
    }
    #[test]
    fn restoring_is_lazy_and_retains_unavailable_locations_and_search_identity() {
        let (f, mut state) = fixture();
        let mut tabs = state.tab_snapshots();
        tabs[0].search = Some(SearchSession {
            query: "star".into(),
            root: Some(f.path("projects")),
            cursor: 2,
            cursor_path: Some(f.path("projects/starwire/README.md")),
            ..Default::default()
        });
        let mut missing = tabs[0].clone();
        missing.search = None;
        missing.stacks[0] = Stack::new("/unmounted/starfold-tab-test".into()).snapshot();
        tabs.push(missing);
        let effects = send(&mut state, Command::RestoreTabs(tabs, 0));
        assert!(!effects.jobs.iter().any(|job| matches!(job, Job::List(path) if path == Path::new("/unmounted/starfold-tab-test"))));
        assert_eq!(state.search.as_ref().unwrap().root, f.path("projects"));
        assert_eq!(
            state.search.as_ref().unwrap().restore_cursor_path,
            Some(f.path("projects/starwire/README.md"))
        );
        let other = state.tabs.tabs[1].id;
        send(&mut state, Command::SwitchTab(other));
        let missing = state.active_frame().dir.clone();
        listed(&mut state, &missing);
        assert_eq!(state.active_frame().dir, missing);
        assert!(!state.active_frame().loading);
    }
    #[test]
    fn reordering_preserves_active_identity_and_closed_history_is_bounded() {
        let (_, mut state) = fixture();
        let original = state.tabs.active().id;
        for _ in 0..12 {
            send(&mut state, Command::NewTab { duplicate: false });
            let id = state.tabs.active().id;
            send(&mut state, Command::MoveTab(id, -1));
            assert_eq!(state.tabs.active().id, id);
            send(&mut state, Command::CloseTab(id));
        }
        assert_eq!(state.closed_tabs.len(), 10);
        assert_eq!(state.tabs.active().id, original);
    }
}
