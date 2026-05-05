use serde::{Deserialize, Serialize};

use crate::{ReplyTargetBindingRef, TurnRunId, TurnRunProfile, TurnStatus, events::EventCursor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmitTurnResponse {
    Accepted {
        turn_id: crate::TurnId,
        run_id: TurnRunId,
        status: TurnStatus,
        profile: TurnRunProfile,
        event_cursor: EventCursor,
        reply_target_binding_ref: ReplyTargetBindingRef,
    },
}

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
pub struct CancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
}
