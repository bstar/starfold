//! Tabs, and the forked stacks inside one.
//!
//! One tab owns Fold's stack at index 0 and, once opened, Commander's left
//! and right stacks at indices 1 and 2. Only one stack receives commands at
//! a time; Commander renders both of its stacks. Each tab retains an independent browsing workspace.

use super::stack::Stack;
use super::{
    preview::Preview,
    search::{Identity, Search},
    selection::Selection,
    sort::SortOrder,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// Parked core state for an inactive workspace. The active workspace is
/// materialized in State for the existing command/query API.
#[derive(Debug)]
pub struct Context {
    pub commander: bool,
    pub commander_pane: usize,
    pub parked_selections: [Selection; 3],
    pub selection: Selection,
    pub marked_search_identities: HashMap<PathBuf, Identity>,
    pub search: Option<Search>,
    pub search_generation: u64,
    pub preview: Option<(PathBuf, Arc<Preview>)>,
    pub preview_generation: u64,
    pub sort: SortOrder,
    pub pane_sorts: [SortOrder; 2],
    pub show_hidden: bool,
    pub loading: bool,
    pub pending_search: Option<crate::session::SearchSession>,
}

impl Context {
    pub fn fresh(sort: SortOrder, hidden: bool) -> Self {
        Self {
            commander: false,
            commander_pane: 0,
            parked_selections: Default::default(),
            selection: Selection::default(),
            marked_search_identities: HashMap::new(),
            search: None,
            search_generation: 0,
            preview: None,
            preview_generation: 0,
            sort,
            pane_sorts: [sort; 2],
            show_hidden: hidden,
            loading: true,
            pending_search: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Identifies one of a tab's stacks. Not used until forked stacks arrive; a
/// `Tab` with exactly one stack still needs a name for it so that a later
/// fork is a new entry in `stacks` rather than a change of type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackId(pub u64);

#[derive(Debug)]
pub struct Tab {
    pub id: TabId,
    pub stacks: Vec<Stack>,
    pub active_stack: usize,
    pub name: Option<String>,
    pub context: Option<Context>,
}

impl Tab {
    pub fn single(stack: Stack) -> Self {
        Self {
            id: TabId(0),
            stacks: vec![stack],
            active_stack: 0,
            name: None,
            context: None,
        }
    }

    pub fn active_stack(&self) -> &Stack {
        &self.stacks[self.active_stack]
    }

    pub fn active_stack_mut(&mut self) -> &mut Stack {
        &mut self.stacks[self.active_stack]
    }
}

#[derive(Debug)]
pub struct Tabs {
    pub tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
}

impl Tabs {
    /// One tab, one stack -- what every session starts as until forked
    /// stacks and multiple tabs are more than a shape reserved for them.
    pub fn single(stack: Stack) -> Self {
        Self {
            tabs: vec![Tab::single(stack)],
            active: 0,
            next_id: 1,
        }
    }

    pub fn index(&self) -> usize {
        self.active
    }
    pub fn activate(&mut self, index: usize) {
        self.active = index.min(self.tabs.len() - 1);
    }
    pub fn allocate_id(&mut self) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        id
    }
    pub fn replace(&mut self, tabs: Vec<Tab>, active: usize) {
        if tabs.is_empty() {
            return;
        }
        self.next_id = self
            .next_id
            .max(tabs.iter().map(|t| t.id.0).max().unwrap_or(0) + 1);
        self.tabs = tabs;
        self.activate(active);
    }

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_tabs_starts_with_one_tab_and_one_stack() {
        let tabs = Tabs::single(Stack::new("/home".into()));
        assert_eq!(tabs.tabs.len(), 1);
        assert_eq!(tabs.active().stacks.len(), 1);
        assert_eq!(
            tabs.active().active_stack().active().dir,
            std::path::PathBuf::from("/home")
        );
    }

    #[test]
    fn active_mut_reaches_the_same_tab_active_reads() {
        let mut tabs = Tabs::single(Stack::new("/home".into()));
        tabs.active_mut()
            .active_stack_mut()
            .push("/home/projects".into());
        assert_eq!(
            tabs.active().active_stack().active().dir,
            std::path::PathBuf::from("/home/projects")
        );
    }
}
