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
    /// `true` when this response is a cached idempotency *replay* of a resume
    /// that already happened, rather than a fresh state transition produced by
    /// this call.
    ///
    /// Load-bearing for the attested-signing path: a `BlockedAttested` resume
    /// transitions to [`TurnStatus::AttestedResolved`] and the reborn layer
    /// starts a one-shot external signer continuation on a *fresh*
    /// `AttestedResolved` response. A same-key retry re-reads the cached success
    /// from the idempotency map; without this flag the continuation layer could
    /// not tell that success apart from the original and could fire the signer
    /// twice. Callers that drive a side effect off a successful resume MUST gate
    /// that side effect on `replayed == false`. Fresh transitions always set
    /// `false`; only idempotency-cache hits set `true`. The persisted
    /// idempotency record always stores the canonical fresh value (`false`).
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
}
