//! Durable projection handoff for completed-run channel replies.
//!
//! The authoritative fact remains `TurnLifecycleEvent::Completed`. This row is
//! only the rebuildable projection key needed to decouple lifecycle commit
//! from provider I/O. It intentionally contains no message content, provider
//! identity, credentials, or copied target metadata: delivery re-opens the
//! canonical run and immutable `RunFinalReplyTargetRecord`, then revalidates
//! current authority immediately before egress.

use ironclaw_turns::{EventCursor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

pub const MAX_RUN_FINAL_REPLY_HANDOFF_PAGE: usize = 1_000;

/// Minimal rebuildable projection key for one completed lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFinalReplyHandoffRecord {
    pub event_cursor: EventCursor,
    pub scope: TurnScope,
    pub run_id: TurnRunId,
}
