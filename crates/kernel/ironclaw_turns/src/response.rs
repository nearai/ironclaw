use serde::{Deserialize, Serialize};

use crate::{EventCursor, TurnActor, TurnRunId, TurnStatus};

// `SubmitTurnResponse` used to live here. It descended to
// `ironclaw_host_api::turn` (WS5): every field type was already that module's,
// and `ironclaw_conversations` persists and replays the value from its inbound
// idempotency ledger without holding a coordinator, so the ledger contract must
// be able to name it without importing this crate. It is re-exported through
// this crate's documented `host_api::turn` facade in `lib.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadBusy {
    pub active_run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeTurnResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryTurnResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
    /// Carried from the store so the coordinator can populate
    /// `owner_user_id` on the cancel lifecycle event without a follow-up
    /// `get_run_state` lookup. `AgentTurnRuntimePort` implementors must populate
    /// this field. Skipped on the wire because actor identity is internal.
    #[serde(skip)]
    pub actor: Option<TurnActor>,
}
