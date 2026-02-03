//! Submission types for the turn-based agent loop.
//!
//! Submissions are the different types of input the agent can receive
//! and process as part of the turn-based development loop.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A submission to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Submission {
    /// User text input (starts a new turn).
    UserInput {
        /// The user's message content.
        content: String,
    },

    /// Response to an execution approval request.
    ExecApproval {
        /// ID of the approval request being responded to.
        request_id: Uuid,
        /// Whether the execution was approved.
        approved: bool,
        /// If true, auto-approve this tool for the rest of the session.
        always: bool,
    },

    /// Interrupt the current turn.
    Interrupt,

    /// Request context compaction.
    Compact,

    /// Undo the last turn.
    Undo,

    /// Redo a previously undone turn (if available).
    Redo,

    /// Resume from a specific checkpoint.
    Resume {
        /// ID of the checkpoint to resume from.
        checkpoint_id: Uuid,
    },

    /// Clear the current thread and start fresh.
    Clear,

    /// Switch to a different thread.
    SwitchThread {
        /// ID of the thread to switch to.
        thread_id: Uuid,
    },

    /// Create a new thread.
    NewThread,
}

impl Submission {
    /// Create a user input submission.
    pub fn user_input(content: impl Into<String>) -> Self {
        Self::UserInput {
            content: content.into(),
        }
    }

    /// Create an approval submission.
    pub fn approval(request_id: Uuid, approved: bool) -> Self {
        Self::ExecApproval {
            request_id,
            approved,
            always: false,
        }
    }

    /// Create an "always approve" submission.
    pub fn always_approve(request_id: Uuid) -> Self {
        Self::ExecApproval {
            request_id,
            approved: true,
            always: true,
        }
    }

    /// Create an interrupt submission.
    pub fn interrupt() -> Self {
        Self::Interrupt
    }

    /// Create a compact submission.
    pub fn compact() -> Self {
        Self::Compact
    }

    /// Create an undo submission.
    pub fn undo() -> Self {
        Self::Undo
    }

    /// Create a redo submission.
    pub fn redo() -> Self {
        Self::Redo
    }

    /// Check if this submission starts a new turn.
    pub fn starts_turn(&self) -> bool {
        matches!(self, Self::UserInput { .. })
    }

    /// Check if this submission is a control command.
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Interrupt
                | Self::Compact
                | Self::Undo
                | Self::Redo
                | Self::Clear
                | Self::NewThread
        )
    }
}

/// Result of processing a submission.
#[derive(Debug, Clone)]
pub enum SubmissionResult {
    /// Turn completed with a response.
    Response {
        /// The agent's response.
        content: String,
    },

    /// Need approval before continuing.
    NeedApproval {
        /// ID of the approval request.
        request_id: Uuid,
        /// Tool that needs approval.
        tool_name: String,
        /// Description of what the tool will do.
        description: String,
        /// Parameters being passed.
        parameters: serde_json::Value,
    },

    /// Successfully processed (for control commands).
    Ok {
        /// Optional message.
        message: Option<String>,
    },

    /// Error occurred.
    Error {
        /// Error message.
        message: String,
    },

    /// Turn was interrupted.
    Interrupted,
}

impl SubmissionResult {
    /// Create a response result.
    pub fn response(content: impl Into<String>) -> Self {
        Self::Response {
            content: content.into(),
        }
    }

    /// Create an OK result.
    pub fn ok() -> Self {
        Self::Ok { message: None }
    }

    /// Create an OK result with a message.
    pub fn ok_with_message(message: impl Into<String>) -> Self {
        Self::Ok {
            message: Some(message.into()),
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_types() {
        let input = Submission::user_input("Hello");
        assert!(input.starts_turn());
        assert!(!input.is_control());

        let undo = Submission::undo();
        assert!(!undo.starts_turn());
        assert!(undo.is_control());
    }
}
