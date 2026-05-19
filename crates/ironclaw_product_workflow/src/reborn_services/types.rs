use chrono::{DateTime, Utc};
use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
use ironclaw_threads::{SessionThreadRecord, SummaryArtifact, ThreadMessageRecord};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunResponse, EventCursor, GateRef, ResumeTurnResponse,
    SanitizedFailure, TurnCheckpointId, TurnRunId, TurnRunState, TurnStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCreateThreadResponse {
    pub thread: SessionThreadRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebornSubmitTurnResponse {
    Submitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        turn_id: String,
        run_id: TurnRunId,
        status: TurnStatus,
        resolved_run_profile_id: String,
        resolved_run_profile_version: u64,
        event_cursor: EventCursor,
    },
    DeferredBusy {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
    AlreadySubmitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineRequest {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineResponse {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornStreamEventsRequest {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornStreamEventsResponse {
    pub events: Vec<ProductOutboundEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
}

impl From<CancelRunResponse> for RebornCancelRunResponse {
    fn from(value: CancelRunResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
            already_terminal: value.already_terminal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornResumeGateResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

impl From<ResumeTurnResponse> for RebornResumeGateResponse {
    fn from(value: ResumeTurnResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebornResolveGateResponse {
    Resumed(RebornResumeGateResponse),
    Cancelled(RebornCancelRunResponse),
}

/// Browser body for the WebUI run-state read.
///
/// Pure read — no idempotency key. Caller authority is supplied separately by
/// `WebUiAuthenticatedCaller` and combined with `thread_id` to produce the
/// canonical [`ironclaw_turns::TurnScope`] inside the facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateRequest {
    pub thread_id: String,
    pub run_id: String,
}

/// Stable run-state projection returned to WebUI route handlers.
///
/// Deliberately omits M3-internal fields carried on [`TurnRunState`]:
/// `scope`, `source_binding_ref`, `reply_target_binding_ref`, and
/// `resolved_model_route`. Route handlers and downstream M5 consumers must
/// build their views from this surface only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateResponse {
    pub turn_id: String,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: String,
    pub resolved_run_profile_version: u64,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<TurnCheckpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<GateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SanitizedFailure>,
}

impl From<TurnRunState> for RebornGetRunStateResponse {
    fn from(value: TurnRunState) -> Self {
        Self {
            turn_id: value.turn_id.to_string(),
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
            accepted_message_ref: value.accepted_message_ref,
            resolved_run_profile_id: value.resolved_run_profile_id.as_str().to_string(),
            resolved_run_profile_version: value.resolved_run_profile_version.as_u64(),
            received_at: value.received_at,
            checkpoint_id: value.checkpoint_id,
            gate_ref: value.gate_ref,
            failure: value.failure,
        }
    }
}
