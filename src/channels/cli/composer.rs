//! Input composer with history and completion.

use std::collections::VecDeque;

/// Maximum number of history entries to keep.
const MAX_HISTORY: usize = 100;

/// Available slash commands for completion.
const SLASH_COMMANDS: &[&str] = &[
    "/help", "/job", "/status", "/cancel", "/list", "/tools", "/clear", "/quit",
];

/// Chat input composer with history navigation and slash command completion.
pub struct ChatComposer {
    /// Current input buffer.
    buffer: String,
    /// Cursor position in the buffer.
    cursor: usize,
    /// Input history.
    history: VecDeque<String>,
    /// Current position in history (-1 = current input).
    history_index: Option<usize>,
    /// Saved current input when navigating history.
    saved_input: String,
    /// Completion candidates.
    completions: Vec<String>,
    /// Current completion index.
    completion_index: Option<usize>,
}

impl ChatComposer {
    /// Create a new composer.
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: VecDeque::with_capacity(MAX_HISTORY),
            history_index: None,
            saved_input: String::new(),
            completions: Vec::new(),
            completion_index: None,
        }
    }

    /// Get the current input buffer.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Get the cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        self.clear_completion();
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a string at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        self.clear_completion();
        self.buffer.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        self.clear_completion();
        if self.cursor > 0 {
            // Find the previous character boundary
            let prev = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    /// Delete the character at the cursor (delete).
    pub fn delete(&mut self) {
        self.clear_completion();
        if self.cursor < self.buffer.len() {
            // Find the next character boundary
            let next = self.buffer[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.buffer.len());
            self.buffer.drain(self.cursor..next);
        }
    }

    /// Move cursor left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// Move cursor right.
    pub fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor = self.buffer[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.buffer.len());
        }
    }

    /// Move cursor to start.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end.
    pub fn move_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    /// Delete from cursor to end of line.
    pub fn kill_line(&mut self) {
        self.clear_completion();
        self.buffer.truncate(self.cursor);
    }

    /// Delete from start to cursor.
    pub fn kill_to_start(&mut self) {
        self.clear_completion();
        self.buffer.drain(..self.cursor);
        self.cursor = 0;
    }

    /// Clear the entire buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.clear_completion();
    }

    /// Submit the current input and return it.
    pub fn submit(&mut self) -> String {
        let input = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        self.clear_completion();

        // Add to history if non-empty and different from last entry
        if !input.is_empty() && self.history.front() != Some(&input) {
            self.history.push_front(input.clone());
            if self.history.len() > MAX_HISTORY {
                self.history.pop_back();
            }
        }

        self.history_index = None;
        self.saved_input.clear();

        input
    }

    /// Navigate to previous history entry.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                // Save current input and go to first history entry
                self.saved_input = std::mem::take(&mut self.buffer);
                self.history_index = Some(0);
                self.buffer = self.history[0].clone();
            }
            Some(i) if i + 1 < self.history.len() => {
                self.history_index = Some(i + 1);
                self.buffer = self.history[i + 1].clone();
            }
            _ => {}
        }

        self.cursor = self.buffer.len();
        self.clear_completion();
    }

    /// Navigate to next history entry.
    pub fn history_next(&mut self) {
        match self.history_index {
            Some(0) => {
                // Go back to saved input
                self.history_index = None;
                self.buffer = std::mem::take(&mut self.saved_input);
            }
            Some(i) => {
                self.history_index = Some(i - 1);
                self.buffer = self.history[i - 1].clone();
            }
            None => {}
        }

        self.cursor = self.buffer.len();
        self.clear_completion();
    }

    /// Attempt tab completion.
    pub fn complete(&mut self) {
        // Only complete slash commands for now
        if !self.buffer.starts_with('/') {
            return;
        }

        if self.completions.is_empty() {
            // Generate completions
            let prefix = &self.buffer;
            self.completions = SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(prefix))
                .map(|s| s.to_string())
                .collect();

            if !self.completions.is_empty() {
                self.completion_index = Some(0);
            }
        } else if let Some(i) = self.completion_index {
            // Cycle through completions
            self.completion_index = Some((i + 1) % self.completions.len());
        }

        // Apply completion
        if let Some(i) = self.completion_index {
            if let Some(completion) = self.completions.get(i) {
                self.buffer = completion.clone();
                self.cursor = self.buffer.len();
            }
        }
    }

    /// Clear completion state.
    fn clear_completion(&mut self) {
        self.completions.clear();
        self.completion_index = None;
    }

    /// Get current completion hint (for display).
    pub fn completion_hint(&self) -> Option<&str> {
        if let Some(i) = self.completion_index {
            self.completions.get(i).map(|s| s.as_str())
        } else {
            None
        }
    }

    /// Get the number of completions available.
    pub fn completion_count(&self) -> usize {
        self.completions.len()
    }
}

impl Default for ChatComposer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_backspace() {
        let mut composer = ChatComposer::new();
        composer.insert('h');
        composer.insert('i');
        assert_eq!(composer.buffer(), "hi");
        composer.backspace();
        assert_eq!(composer.buffer(), "h");
    }

    #[test]
    fn test_history_navigation() {
        let mut composer = ChatComposer::new();
        composer.insert_str("first");
        composer.submit();
        composer.insert_str("second");
        composer.submit();

        composer.insert_str("current");
        composer.history_prev();
        assert_eq!(composer.buffer(), "second");
        composer.history_prev();
        assert_eq!(composer.buffer(), "first");
        composer.history_next();
        assert_eq!(composer.buffer(), "second");
        composer.history_next();
        assert_eq!(composer.buffer(), "current");
    }

    #[test]
    fn test_completion() {
        let mut composer = ChatComposer::new();
        composer.insert_str("/hel");
        composer.complete();
        assert_eq!(composer.buffer(), "/help");
    }
}
