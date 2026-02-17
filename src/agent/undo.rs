//! Undo system with checkpoints.
//!
//! Provides the ability to roll back the conversation state to a previous point.
//! Checkpoints are created automatically at the start of each turn.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::ChatMessage;

/// Maximum number of checkpoints to keep by default.
const DEFAULT_MAX_CHECKPOINTS: usize = 20;

/// A snapshot of conversation state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique checkpoint ID.
    pub id: Uuid,
    /// Turn number this checkpoint was created at.
    pub turn_number: usize,
    /// Snapshot of messages at this point.
    pub messages: Vec<ChatMessage>,
    /// Description of what happened at this checkpoint.
    pub description: String,
}

impl Checkpoint {
    /// Create a new checkpoint.
    pub fn new(
        turn_number: usize,
        messages: Vec<ChatMessage>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            turn_number,
            messages,
            description: description.into(),
        }
    }
}

/// Manager for undo/redo functionality.
pub struct UndoManager {
    /// Stack of past checkpoints (for undo).
    undo_stack: VecDeque<Checkpoint>,
    /// Stack of future checkpoints (for redo).
    redo_stack: Vec<Checkpoint>,
    /// Maximum checkpoints to keep.
    max_checkpoints: usize,
}

impl UndoManager {
    /// Create a new undo manager.
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
        }
    }

    /// Create with a custom checkpoint limit.
    pub fn with_max_checkpoints(mut self, max: usize) -> Self {
        self.max_checkpoints = max;
        self
    }

    /// Create a checkpoint at the current state.
    ///
    /// This clears the redo stack since we're creating a new history branch.
    pub fn checkpoint(
        &mut self,
        turn_number: usize,
        messages: Vec<ChatMessage>,
        description: impl Into<String>,
    ) {
        // Clear redo stack (new branch of history)
        self.redo_stack.clear();

        // Create and push checkpoint
        let checkpoint = Checkpoint::new(turn_number, messages, description);
        self.undo_stack.push_back(checkpoint);

        // Trim if over limit
        while self.undo_stack.len() > self.max_checkpoints {
            self.undo_stack.pop_front();
        }
    }

    /// Undo: pop the last checkpoint and return it.
    ///
    /// The current state should be saved to redo stack before calling this.
    pub fn undo(
        &mut self,
        current_turn: usize,
        current_messages: Vec<ChatMessage>,
    ) -> Option<&Checkpoint> {
        if self.undo_stack.is_empty() {
            return None;
        }

        // Save current state to redo stack
        let current = Checkpoint::new(
            current_turn,
            current_messages,
            format!("Turn {}", current_turn),
        );
        self.redo_stack.push(current);

        // Return the most recent checkpoint without removing it
        // (we keep it so multiple undos can work)
        self.undo_stack.back()
    }

    /// Pop the last checkpoint from the undo stack.
    pub fn pop_undo(&mut self) -> Option<Checkpoint> {
        self.undo_stack.pop_back()
    }

    /// Redo: restore a previously undone state.
    pub fn redo(&mut self) -> Option<Checkpoint> {
        self.redo_stack.pop()
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the number of undo steps available.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of redo steps available.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Get a checkpoint by ID.
    pub fn get_checkpoint(&self, id: Uuid) -> Option<&Checkpoint> {
        self.undo_stack
            .iter()
            .find(|c| c.id == id)
            .or_else(|| self.redo_stack.iter().find(|c| c.id == id))
    }

    /// List all available checkpoints (for UI display).
    pub fn list_checkpoints(&self) -> Vec<&Checkpoint> {
        self.undo_stack.iter().collect()
    }

    /// Clear all checkpoints.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Restore to a specific checkpoint by ID.
    ///
    /// This invalidates all checkpoints after this one.
    pub fn restore(&mut self, checkpoint_id: Uuid) -> Option<Checkpoint> {
        // Find the checkpoint position
        let pos = self.undo_stack.iter().position(|c| c.id == checkpoint_id)?;

        // Clear redo stack
        self.redo_stack.clear();

        // Remove all checkpoints after this one
        while self.undo_stack.len() > pos + 1 {
            self.undo_stack.pop_back();
        }

        // Pop and return the target checkpoint
        self.undo_stack.pop_back()
    }
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_creation() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Initial state");
        manager.checkpoint(1, vec![ChatMessage::user("Hello")], "Turn 1");

        assert_eq!(manager.undo_count(), 2);
    }

    #[test]
    fn test_undo_redo() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Turn 0");
        manager.checkpoint(1, vec![ChatMessage::user("Hello")], "Turn 1");

        assert!(manager.can_undo());
        assert!(!manager.can_redo());

        // Undo
        let current = vec![ChatMessage::user("Hello"), ChatMessage::assistant("Hi")];
        let checkpoint = manager.undo(2, current);
        assert!(checkpoint.is_some());
        assert!(manager.can_redo());

        // Redo
        let restored = manager.redo();
        assert!(restored.is_some());
    }

    #[test]
    fn test_max_checkpoints() {
        let mut manager = UndoManager::new().with_max_checkpoints(3);

        for i in 0..5 {
            manager.checkpoint(i, vec![], format!("Turn {}", i));
        }

        assert_eq!(manager.undo_count(), 3);
    }

    #[test]
    fn test_restore_to_checkpoint() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Turn 0");
        let checkpoint_id = manager.undo_stack.back().unwrap().id;
        manager.checkpoint(1, vec![], "Turn 1");
        manager.checkpoint(2, vec![], "Turn 2");

        let restored = manager.restore(checkpoint_id);
        assert!(restored.is_some());
        assert_eq!(manager.undo_count(), 0);
    }
}
