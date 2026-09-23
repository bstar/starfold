//! Tabs, and the forked stacks inside one.
//!
//! One tab owns Fold's stack at index 0 and, once opened, Commander's left
//! and right stacks at indices 1 and 2. Only one stack receives commands at
//! a time; Commander renders both of its stacks. Multiple tabs remain reserved.

use super::stack::Stack;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Identifies one of a tab's stacks. Not used until forked stacks arrive; a
/// `Tab` with exactly one stack still needs a name for it so that a later
/// fork is a new entry in `stacks` rather than a change of type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackId(pub u64);

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: TabId,
    pub stacks: Vec<Stack>,
    pub active_stack: usize,
}

impl Tab {
    pub fn single(stack: Stack) -> Self {
        Self {
            id: TabId(0),
            stacks: vec![stack],
            active_stack: 0,
        }
    }

    pub fn active_stack(&self) -> &Stack {
        &self.stacks[self.active_stack]
    }

    pub fn active_stack_mut(&mut self) -> &mut Stack {
        &mut self.stacks[self.active_stack]
    }
}

#[derive(Debug, Clone)]
pub struct Tabs {
    pub tabs: Vec<Tab>,
    active: usize,
}

impl Tabs {
    /// One tab, one stack -- what every session starts as until forked
    /// stacks and multiple tabs are more than a shape reserved for them.
    pub fn single(stack: Stack) -> Self {
        Self {
            tabs: vec![Tab::single(stack)],
            active: 0,
        }
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
