//! Approval overlay modal.

use uuid::Uuid;

/// A request for user approval before executing a tool.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Unique ID for this request.
    pub id: Uuid,
    /// Name of the tool requesting approval.
    pub tool_name: String,
    /// Description of what the tool will do.
    pub description: String,
    /// Parameters being passed to the tool.
    pub parameters: serde_json::Value,
    /// Whether this is a destructive operation.
    pub destructive: bool,
}

impl ApprovalRequest {
    /// Create a new approval request.
    pub fn new(
        tool_name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool_name: tool_name.into(),
            description: description.into(),
            parameters,
            destructive: false,
        }
    }

    /// Mark as destructive operation.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// Current selection in the approval overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSelection {
    /// Yes, approve this action.
    Yes,
    /// No, deny this action.
    No,
    /// Always approve this tool (for this session).
    Always,
}

impl ApprovalSelection {
    /// Get the next selection (cycling).
    pub fn next(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Always,
            Self::Always => Self::Yes,
        }
    }

    /// Get the previous selection (cycling).
    pub fn prev(self) -> Self {
        match self {
            Self::Yes => Self::Always,
            Self::No => Self::Yes,
            Self::Always => Self::No,
        }
    }
}

/// Approval overlay state.
pub struct ApprovalOverlay {
    /// The request being shown.
    pub request: ApprovalRequest,
    /// Current selection.
    pub selection: ApprovalSelection,
}

impl ApprovalOverlay {
    /// Create a new approval overlay.
    pub fn new(request: ApprovalRequest) -> Self {
        Self {
            request,
            selection: ApprovalSelection::Yes,
        }
    }

    /// Move selection left.
    pub fn select_prev(&mut self) {
        self.selection = self.selection.prev();
    }

    /// Move selection right.
    pub fn select_next(&mut self) {
        self.selection = self.selection.next();
    }

    /// Handle keyboard shortcut.
    pub fn handle_shortcut(&mut self, c: char) -> Option<bool> {
        match c.to_ascii_lowercase() {
            'y' => Some(true),
            'n' => Some(false),
            'a' => {
                self.selection = ApprovalSelection::Always;
                Some(true)
            }
            _ => None,
        }
    }

    /// Confirm the current selection.
    pub fn confirm(&self) -> (bool, bool) {
        match self.selection {
            ApprovalSelection::Yes => (true, false),
            ApprovalSelection::No => (false, false),
            ApprovalSelection::Always => (true, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approval_selection_cycle() {
        let sel = ApprovalSelection::Yes;
        assert_eq!(sel.next(), ApprovalSelection::No);
        assert_eq!(sel.next().next(), ApprovalSelection::Always);
        assert_eq!(sel.next().next().next(), ApprovalSelection::Yes);
    }

    #[test]
    fn test_approval_shortcuts() {
        let request = ApprovalRequest::new("test", "Test operation", serde_json::json!({}));
        let mut overlay = ApprovalOverlay::new(request);

        assert_eq!(overlay.handle_shortcut('y'), Some(true));
        assert_eq!(overlay.handle_shortcut('n'), Some(false));
        assert_eq!(overlay.handle_shortcut('x'), None);
    }
}
