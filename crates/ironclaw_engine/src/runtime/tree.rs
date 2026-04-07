//! Thread tree — parent-child relationship tracking.

use std::collections::HashMap;

use crate::types::thread::ThreadId;

/// Manages parent-child thread relationships.
///
/// Simple in-memory tree. Threads form a forest (multiple roots).
#[derive(Debug, Default)]
pub struct ThreadTree {
    /// child → parent
    parents: HashMap<ThreadId, ThreadId>,
    /// parent → children (ordered by insertion)
    children: HashMap<ThreadId, Vec<ThreadId>>,
}

impl ThreadTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a parent-child relationship.
    pub fn add_child(&mut self, parent_id: ThreadId, child_id: ThreadId) {
        self.parents.insert(child_id, parent_id);
        self.children.entry(parent_id).or_default().push(child_id);
    }

    /// Get the parent of a thread, if any.
    pub fn parent_of(&self, thread_id: ThreadId) -> Option<ThreadId> {
        self.parents.get(&thread_id).copied()
    }

    /// Get the children of a thread.
    pub fn children_of(&self, thread_id: ThreadId) -> &[ThreadId] {
        self.children
            .get(&thread_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Walk up the tree to collect all ancestors (parent, grandparent, ...).
    pub fn ancestors(&self, thread_id: ThreadId) -> Vec<ThreadId> {
        let mut result = Vec::new();
        let mut current = thread_id;
        while let Some(parent) = self.parents.get(&current) {
            result.push(*parent);
            current = *parent;
        }
        result
    }

    /// Remove a thread from the tree. Does not remove its children.
    pub fn remove(&mut self, thread_id: ThreadId) {
        if let Some(parent) = self.parents.remove(&thread_id)
            && let Some(siblings) = self.children.get_mut(&parent)
        {
            siblings.retain(|id| *id != thread_id);
        }
        // Orphan any children (their parent_id entries become stale)
        self.children.remove(&thread_id);
    }

    /// Check if a thread is a root (no parent).
    pub fn is_root(&self, thread_id: ThreadId) -> bool {
        !self.parents.contains_key(&thread_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query() {
        let mut tree = ThreadTree::new();
        let parent = ThreadId::new();
        let child1 = ThreadId::new();
        let child2 = ThreadId::new();

        tree.add_child(parent, child1);
        tree.add_child(parent, child2);

        assert_eq!(tree.parent_of(child1), Some(parent));
        assert_eq!(tree.parent_of(child2), Some(parent));
        assert_eq!(tree.children_of(parent).len(), 2);
        assert!(tree.is_root(parent));
        assert!(!tree.is_root(child1));
    }

    #[test]
    fn ancestors_walk_up() {
        let mut tree = ThreadTree::new();
        let root = ThreadId::new();
        let mid = ThreadId::new();
        let leaf = ThreadId::new();

        tree.add_child(root, mid);
        tree.add_child(mid, leaf);

        let ancestors = tree.ancestors(leaf);
        assert_eq!(ancestors, vec![mid, root]);
    }

    #[test]
    fn remove_detaches_from_parent() {
        let mut tree = ThreadTree::new();
        let parent = ThreadId::new();
        let child = ThreadId::new();

        tree.add_child(parent, child);
        tree.remove(child);

        assert_eq!(tree.parent_of(child), None);
        assert!(tree.children_of(parent).is_empty());
    }

    #[test]
    fn children_of_unknown_returns_empty() {
        let tree = ThreadTree::new();
        assert!(tree.children_of(ThreadId::new()).is_empty());
    }

    #[test]
    fn ancestors_of_root_is_empty() {
        let tree = ThreadTree::new();
        assert!(tree.ancestors(ThreadId::new()).is_empty());
    }
}
