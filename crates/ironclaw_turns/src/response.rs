use serde::{Deserialize, Serialize};

use crate::{
    AcceptedMessageRef, ReplyTargetBindingRef, RunProfileId, RunProfileVersion, TurnRunId,
    TurnStatus, events::EventCursor,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmitTurnResponse {
    Accepted {
        turn_id: crate::TurnId,
        run_id: TurnRunId,
        status: TurnStatus,
        resolved_run_profile_id: RunProfileId,
        resolved_run_profile_version: RunProfileVersion,
        event_cursor: EventCursor,
        accepted_message_ref: AcceptedMessageRef,
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
