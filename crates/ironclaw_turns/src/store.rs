use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    AcceptedMessageRef, AdmissionRejection, CancelRunRequest, CancelRunResponse, GateRef,
    GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnRequest,
    ResumeTurnResponse, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy,
    TurnActor, TurnAdmissionPolicy, TurnCheckpointId, TurnError, TurnErrorCategory, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunProfile, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    TurnTimestamp, events::EventCursor,
};

#[async_trait]
pub trait TurnStateStore: Send + Sync {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
    ) -> Result<SubmitTurnResponse, TurnError>;

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError>;

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError>;

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnLockVersion(u64);

impl TurnLockVersion {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn incremented(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnActiveLockKey {
    pub scope: TurnScope,
}

impl From<&TurnScope> for TurnActiveLockKey {
    fn from(scope: &TurnScope) -> Self {
        Self {
            scope: scope.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_id: TurnId,
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub created_at: TurnTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunRecord {
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
    pub scope: TurnScope,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub status: TurnStatus,
    pub profile: TurnRunProfile,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    pub failure: Option<crate::SanitizedFailure>,
    pub event_cursor: EventCursor,
    pub runner_id: Option<TurnRunnerId>,
    pub lease_token: Option<TurnLeaseToken>,
    pub lease_expires_at: Option<TurnTimestamp>,
    pub last_heartbeat_at: Option<TurnTimestamp>,
    pub claim_count: u64,
    pub received_at: TurnTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnActiveLockRecord {
    pub key: TurnActiveLockKey,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub lock_version: TurnLockVersion,
    pub acquired_at: TurnTimestamp,
    pub updated_at: TurnTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpointRecord {
    pub checkpoint_id: TurnCheckpointId,
    pub run_id: TurnRunId,
    pub sequence: u64,
    pub status: TurnStatus,
    pub gate_ref: GateRef,
    pub created_at: TurnTimestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnIdempotencyOperationKind {
    Submit,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnIdempotencyOutcomeKind {
    Accepted,
    ThreadBusy,
    AdmissionRejected,
    Resumed,
    CancelRecorded,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
}

impl TurnIdempotencyOutcomeKind {
    pub fn from_error(error: &TurnError) -> Self {
        match error {
            TurnError::ThreadBusy(_) => Self::ThreadBusy,
            TurnError::AdmissionRejected(_) => Self::AdmissionRejected,
            TurnError::ScopeNotFound => Self::ScopeNotFound,
            TurnError::Unauthorized => Self::Unauthorized,
            TurnError::InvalidRequest { .. } => Self::InvalidRequest,
            TurnError::Unavailable { .. } => Self::Unavailable,
            TurnError::Conflict { .. }
            | TurnError::InvalidTransition { .. }
            | TurnError::LeaseMismatch => Self::Conflict,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnIdempotencyReplay {
    SubmitAccepted(SubmitTurnResponse),
    SubmitThreadBusy(ThreadBusy),
    SubmitAdmissionRejected(AdmissionRejection),
    ResumeSucceeded(ResumeTurnResponse),
    CancelRecorded(CancelRunResponse),
    Error(TurnIdempotencyErrorReplay),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnIdempotencyErrorReplay {
    pub category: TurnErrorCategory,
    pub adapter_status_code: u16,
}

impl TurnIdempotencyErrorReplay {
    pub fn from_error(error: &TurnError) -> Self {
        Self {
            category: error.category(),
            adapter_status_code: error.adapter_status_code(),
        }
    }

    fn to_error(&self) -> TurnError {
        match self.category {
            TurnErrorCategory::ScopeNotFound => TurnError::ScopeNotFound,
            TurnErrorCategory::Unauthorized => TurnError::Unauthorized,
            TurnErrorCategory::InvalidRequest => TurnError::InvalidRequest {
                reason: "replayed invalid request".to_string(),
            },
            TurnErrorCategory::Unavailable => TurnError::Unavailable {
                reason: "replayed unavailable".to_string(),
            },
            TurnErrorCategory::Conflict => TurnError::Conflict {
                reason: "replayed conflict".to_string(),
            },
            TurnErrorCategory::ThreadBusy => TurnError::Conflict {
                reason: "replayed malformed thread-busy idempotency record".to_string(),
            },
            TurnErrorCategory::AdmissionRejected => TurnError::Conflict {
                reason: "replayed malformed admission idempotency record".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnIdempotencyRecord {
    pub scope: TurnScope,
    pub operation: TurnIdempotencyOperationKind,
    pub key: IdempotencyKey,
    pub turn_id: Option<TurnId>,
    pub run_id: Option<TurnRunId>,
    pub outcome: TurnIdempotencyOutcomeKind,
    pub replay: TurnIdempotencyReplay,
    pub created_at: TurnTimestamp,
    pub expires_at: Option<TurnTimestamp>,
}

impl TurnIdempotencyRecord {
    pub fn replay_submit(&self) -> Option<Result<SubmitTurnResponse, TurnError>> {
        if self.operation != TurnIdempotencyOperationKind::Submit {
            return None;
        }
        match &self.replay {
            TurnIdempotencyReplay::SubmitAccepted(response) => Some(Ok(response.clone())),
            TurnIdempotencyReplay::SubmitThreadBusy(busy) => {
                Some(Err(TurnError::ThreadBusy(busy.clone())))
            }
            TurnIdempotencyReplay::SubmitAdmissionRejected(rejection) => {
                Some(Err(TurnError::AdmissionRejected(rejection.clone())))
            }
            TurnIdempotencyReplay::Error(error)
                if self.operation == TurnIdempotencyOperationKind::Submit =>
            {
                Some(Err(error.to_error()))
            }
            _ => None,
        }
    }

    pub fn replay_resume(&self) -> Option<Result<ResumeTurnResponse, TurnError>> {
        if self.operation != TurnIdempotencyOperationKind::Resume {
            return None;
        }
        match &self.replay {
            TurnIdempotencyReplay::ResumeSucceeded(response) => Some(Ok(response.clone())),
            TurnIdempotencyReplay::Error(error)
                if self.operation == TurnIdempotencyOperationKind::Resume =>
            {
                Some(Err(error.to_error()))
            }
            _ => None,
        }
    }

    pub fn replay_cancel(&self) -> Option<Result<CancelRunResponse, TurnError>> {
        if self.operation != TurnIdempotencyOperationKind::Cancel {
            return None;
        }
        match &self.replay {
            TurnIdempotencyReplay::CancelRecorded(response) => Some(Ok(response.clone())),
            TurnIdempotencyReplay::Error(error)
                if self.operation == TurnIdempotencyOperationKind::Cancel =>
            {
                Some(Err(error.to_error()))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TurnPersistenceSnapshot {
    pub turns: Vec<TurnRecord>,
    pub runs: Vec<TurnRunRecord>,
    pub active_locks: Vec<TurnActiveLockRecord>,
    pub checkpoints: Vec<TurnCheckpointRecord>,
    pub idempotency_records: Vec<TurnIdempotencyRecord>,
}
