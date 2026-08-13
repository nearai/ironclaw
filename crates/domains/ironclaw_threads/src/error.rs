use ironclaw_host_api::ids::ThreadId;
use thiserror::Error;

use crate::{MessageStatus, ThreadMessageId};

/// Specific timestamp contract violation on a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimestampViolation {
    #[error("missing durable timestamps")]
    MissingDurableTimestamps,
    #[error("cleared durable timestamps")]
    ClearedDurableTimestamps,
}

/// Canonical thread/transcript service errors.
#[derive(Debug, Error)]
pub enum SessionThreadError {
    #[error("unknown thread {thread_id}")]
    UnknownThread { thread_id: ThreadId },
    #[error("unknown message {message_id}")]
    UnknownMessage { message_id: ThreadMessageId },
    #[error("thread {thread_id} already exists in a different scope")]
    ThreadScopeMismatch { thread_id: ThreadId },
    #[error("message {message_id} is not an assistant draft")]
    MessageNotDraft { message_id: ThreadMessageId },
    #[error("message {message_id} cannot transition from {from:?} via {attempted}")]
    InvalidMessageTransition {
        message_id: ThreadMessageId,
        from: MessageStatus,
        attempted: &'static str,
    },
    #[error(
        "idempotent inbound event belongs to thread {stored_thread_id}, not requested thread {requested_thread_id}"
    )]
    IdempotentReplayThreadMismatch {
        stored_thread_id: ThreadId,
        requested_thread_id: ThreadId,
    },
    #[error(
        "idempotent inbound event belongs to actor {stored_actor_id}, not requested actor {requested_actor_id}"
    )]
    IdempotentReplayActorMismatch {
        stored_actor_id: String,
        requested_actor_id: String,
    },
    #[error("invalid summary range {start_sequence}..={end_sequence}")]
    InvalidSummaryRange {
        start_sequence: u64,
        end_sequence: u64,
    },
    #[error(
        "summary range {start_sequence}..={end_sequence} overlaps an existing replacement summary"
    )]
    OverlappingSummaryRange {
        start_sequence: u64,
        end_sequence: u64,
    },
    #[error("invalid attachment on inbound message: {0}")]
    InvalidAttachment(String),
    #[error("{context} timestamp violation on message {message_id}: {violation}")]
    InvalidMessageTimestamp {
        message_id: ThreadMessageId,
        context: &'static str,
        violation: TimestampViolation,
    },
    #[error("invalid unbound context: {reason}")]
    InvalidPreparedContext { reason: String },
    #[error("unbound context replay key mismatch for thread {thread_id}")]
    PreparedContextKeyMismatch { thread_id: ThreadId },
    #[error("failed to create generated thread id: {0}")]
    GeneratedThreadId(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
    #[error("thread backend error: {0}")]
    Backend(String),
}

impl SessionThreadError {
    /// Stable, detail-free classification for metrics and scrubbed diagnostics.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::UnknownThread { .. } => "unknown_thread",
            Self::UnknownMessage { .. } => "unknown_message",
            Self::ThreadScopeMismatch { .. } => "thread_scope_mismatch",
            Self::MessageNotDraft { .. } => "message_not_draft",
            Self::InvalidMessageTransition { .. } => "invalid_message_transition",
            Self::IdempotentReplayThreadMismatch { .. } => "idempotent_replay_thread_mismatch",
            Self::IdempotentReplayActorMismatch { .. } => "idempotent_replay_actor_mismatch",
            Self::InvalidSummaryRange { .. } => "invalid_summary_range",
            Self::OverlappingSummaryRange { .. } => "overlapping_summary_range",
            Self::InvalidAttachment(_) => "invalid_attachment",
            Self::InvalidMessageTimestamp { .. } => "invalid_message_timestamp",
            Self::InvalidPreparedContext { .. } => "invalid_prepared_context",
            Self::PreparedContextKeyMismatch { .. } => "prepared_context_key_mismatch",
            Self::GeneratedThreadId(_) => "generated_thread_id",
            Self::Serialization(_) => "serialization",
            Self::Deserialization(_) => "deserialization",
            Self::Backend(_) => "backend",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionThreadError;

    #[test]
    fn error_kind_name_never_formats_backend_detail() {
        let error = SessionThreadError::Backend("token sk-secret".to_string());
        assert_eq!(error.kind_name(), "backend");
        assert!(!error.kind_name().contains("sk-secret"));
    }
}
