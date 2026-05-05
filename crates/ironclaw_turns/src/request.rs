use serde::{Deserialize, Serialize};

use crate::{
    AcceptedMessageRef, GateRef, IdempotencyKey, ReplyTargetBindingRef, SanitizedCancelReason,
    SourceBindingRef, TurnActor, TurnRunId, TurnRunProfile, TurnScope,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitTurnRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub profile: TurnRunProfile,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeTurnRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub gate_resolution_ref: GateRef,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub reason: SanitizedCancelReason,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRunStateRequest {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
}
