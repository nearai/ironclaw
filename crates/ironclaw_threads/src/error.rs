use ironclaw_host_api::ThreadId;
use thiserror::Error;

use crate::{MessageStatus, ThreadMessageId};

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
    #[error("failed to create generated thread id: {0}")]
    GeneratedThreadId(String),
}
