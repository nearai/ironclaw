use thiserror::Error;

use crate::cursor::EventCursor;

/// Event sink and durable-log error variants.
#[derive(Debug, Error)]
pub enum EventError {
    #[error("event serialization failed: {reason}")]
    Serialize { reason: String },
    #[error("event sink failed: {reason}")]
    Sink { reason: String },
    #[error("durable event log failed: {reason}")]
    DurableLog { reason: String },
    #[error(
        "replay gap: requested cursor {requested:?} predates earliest retained cursor {earliest:?}; consumer must request a scoped snapshot/rebase"
    )]
    ReplayGap {
        requested: EventCursor,
        earliest: EventCursor,
    },
    #[error("replay request rejected: {reason}")]
    InvalidReplayRequest { reason: String },
}
