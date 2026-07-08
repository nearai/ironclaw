use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::{AgentId, ProjectId, RuntimeCredentialAuthRequirement, TenantId};

use crate::{
    AcceptedMessageRef, AdmissionRejection, CancelRunRequest, CancelRunResponse,
    CapabilityActivityId, GateRef, GetRunStateRequest, IdempotencyKey, LoopCheckpointRecord,
    ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest,
    RetryTurnResponse, RunProfileResolver, SourceBindingRef, SubmitChildRunRequest,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActiveRunRefState, TurnActor,
    TurnAdmissionPolicy, TurnAdmissionReservationRecord, TurnCapacityResource, TurnCheckpointId,
    TurnError, TurnErrorCategory, TurnId, TurnLeaseToken, TurnLifecycleEvent, TurnRunId,
    TurnRunProfile, TurnRunState, TurnRunnerId, TurnScope, TurnStatus, TurnTimestamp,
    events::EventCursor,
    run_profile::{LoopCheckpointKind, LoopCheckpointStateRef, LoopModelRouteSnapshot},
};

#[async_trait]
pub trait TurnStateStore: Send + Sync {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError>;

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError>;

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError>;

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError>;

    /// Return the run state when the run exists in the supplied exact scope.
    ///
    /// Missing runs and runs outside the supplied scope must both return
    /// [`TurnError::ScopeNotFound`]. This keeps scoped lookups non-enumerating
    /// and gives higher-level helpers one canonical missing-run shape.
    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError>;

    /// Return run state for live cancellation-handle seeding.
    ///
    /// The default path is the canonical scoped lookup. Stores with an
    /// authoritative hot cache may override this to avoid forcing durable row
    /// materialization on the turn execution hot path.
    async fn get_run_state_for_cancellation(
        &self,
        request: GetRunStateRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.get_run_state(request).await
    }
}

/// Classify an active run reference through the shared turn-state lookup.
///
/// `None` and missing records both map to [`TurnActiveRunRefState::Missing`];
/// only looked-up terminal records map to [`TurnActiveRunRefState::Terminal`].
pub async fn active_run_ref_state<S>(
    store: &S,
    scope: TurnScope,
    active_run_ref: Option<TurnRunId>,
) -> Result<TurnActiveRunRefState, TurnError>
where
    S: TurnStateStore + ?Sized,
{
    let Some(run_id) = active_run_ref else {
        return Ok(TurnActiveRunRefState::Missing);
    };
    match store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
    {
        Ok(state) if state.status.is_terminal() => Ok(TurnActiveRunRefState::Terminal),
        Ok(_) => Ok(TurnActiveRunRefState::Nonterminal),
        Err(TurnError::ScopeNotFound) => Ok(TurnActiveRunRefState::Missing),
        Err(error) => Err(error),
    }
}

#[async_trait]
pub trait TurnSpawnTreeStateStore: TurnStateStore {
    /// Spawn-tree operations are only needed by child-run orchestration.
    /// General turn submission should stay behind `TurnStateStore`.
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError>;
    ///
    /// List child runs only when the parent is visible in the supplied scope.
    ///
    /// Implementations must not leak whether a run exists in another tenant,
    /// agent, project, or thread scope; missing and unauthorized parents should
    /// both produce an empty child list.
    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError>;

    /// Return a run record only when it belongs to the supplied exact scope.
    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError>;

    /// Reserve descendant capacity for a root run after validating root scope.
    ///
    /// Missing roots must return not found and cross-scope roots must return
    /// unauthorized rather than mutating reservation state.
    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError>;

    /// Release descendant capacity for a root run after validating root scope.
    ///
    /// `idempotency_key` dedups the release against a durable per-child
    /// record (`SpawnTreeReservation.released_children`) so a retried call
    /// for the same child (recovery re-driving an edge stuck at
    /// `ReservationReleaseState::Claimed`, or a rollback retry) is a no-op
    /// rather than a second decrement — see
    /// `docs/reborn/subagent-spawn/thread-harness-design.md` §5.5 round-5/6.
    /// Callers pass the child's own `TurnRunId`, which never recurs (ABA-immune).
    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError>;

    /// Remove one child's dedup entry from `SpawnTreeReservation.released_children`
    /// once its await-edge is about to be deleted (§5.5 round-7) — called
    /// strictly *before* the edge's `delete_if_version`, never after, so a
    /// crash between prune and delete just re-derives the same prune on the
    /// next recovery pass (idempotent: pruning an absent entry is a no-op).
    /// Without this, `released_children` grows for the tree's entire
    /// cumulative lifetime instead of staying bounded by the live descendant
    /// cap. Missing root is benign here (the tree may already be fully
    /// released and its reservation record deleted) — only real backend
    /// failures return `Err`.
    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError>;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    pub failure: Option<crate::SanitizedFailure>,
    pub event_cursor: EventCursor,
    pub runner_id: Option<TurnRunnerId>,
    pub lease_token: Option<TurnLeaseToken>,
    pub lease_expires_at: Option<TurnTimestamp>,
    pub last_heartbeat_at: Option<TurnTimestamp>,
    pub claim_count: u64,
    pub received_at: TurnTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<TurnRunId>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_root_run_id: Option<TurnRunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<crate::ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<crate::GateResumeDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpawnTreeReservationKey {
    pub tenant_id: TenantId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    pub root_run_id: TurnRunId,
}

impl SpawnTreeReservationKey {
    pub fn new(scope: &TurnScope, root_run_id: TurnRunId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            root_run_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnTreeReservation {
    pub scope: TurnScope,
    pub root_run_id: TurnRunId,
    pub descendant_count: u64,
    /// Per-child dedup record for `release_tree_descendants`'s idempotency
    /// key (§5.5 round-5/6) — present only for children whose release has
    /// been recorded but whose await-edge hasn't finished closing yet
    /// (pruned in lockstep with edge deletion, so this stays bounded by the
    /// live descendant cap, never the tree's cumulative lifetime count).
    #[serde(default)]
    pub released_children: BTreeSet<TurnRunId>,
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

/// Serde default for `LoopCheckpointKind` — used when deserializing old
/// persisted data that predates the `kind` field.
fn default_checkpoint_kind() -> LoopCheckpointKind {
    LoopCheckpointKind::BeforeBlock
}

/// Serde default for `LoopCheckpointStateRef` — legacy sentinel used only when
/// deserializing old persisted data that predates the `state_ref` field.
fn default_checkpoint_state_ref() -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::legacy_unknown()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpointRecord {
    pub checkpoint_id: TurnCheckpointId,
    pub run_id: TurnRunId,
    /// Scope of the run that created this checkpoint. `None` for legacy records
    /// persisted before scope was added to checkpoints.
    #[serde(default)]
    pub scope: Option<TurnScope>,
    pub sequence: u64,
    pub status: TurnStatus,
    pub gate_ref: GateRef,
    /// The semantic kind of checkpoint (before model, side-effect, block, final).
    #[serde(default = "default_checkpoint_kind")]
    pub kind: LoopCheckpointKind,
    /// An opaque ref describing the loop state at the time of this checkpoint.
    #[serde(default = "default_checkpoint_state_ref")]
    pub state_ref: LoopCheckpointStateRef,
    pub created_at: TurnTimestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnIdempotencyOperationKind {
    Submit,
    Resume,
    Retry,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnIdempotencyOutcomeKind {
    Accepted,
    ThreadBusy,
    AdmissionRejected,
    Resumed,
    Retried,
    CancelRecorded,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    CapacityExceeded,
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
            TurnError::CapacityExceeded { .. } => Self::CapacityExceeded,
            TurnError::Conflict { .. }
            | TurnError::RunNotRetryable { .. }
            | TurnError::InvalidTransition { .. }
            | TurnError::LeaseMismatch => Self::Conflict,
            TurnError::InvalidRunOriginAdapter => Self::InvalidRequest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnIdempotencyReplay {
    SubmitAccepted(SubmitTurnResponse),
    SubmitThreadBusy(ThreadBusy),
    SubmitAdmissionRejected(AdmissionRejection),
    ResumeSucceeded(ResumeTurnResponse),
    RetrySucceeded(RetryTurnResponse),
    RetryThreadBusy(ThreadBusy),
    CancelRecorded(CancelRunResponse),
    Error(TurnIdempotencyErrorReplay),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnIdempotencyErrorReplay {
    pub category: TurnErrorCategory,
    pub adapter_status_code: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_resource: Option<TurnCapacityResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_cap: Option<u64>,
}

impl TurnIdempotencyErrorReplay {
    pub fn from_error(error: &TurnError) -> Self {
        let (capacity_resource, capacity_cap) = match error {
            TurnError::CapacityExceeded { resource, cap } => (Some(*resource), Some(*cap)),
            _ => (None, None),
        };
        Self {
            category: error.category(),
            adapter_status_code: error.adapter_status_code(),
            capacity_resource,
            capacity_cap,
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
            TurnErrorCategory::CapacityExceeded => TurnError::CapacityExceeded {
                resource: self
                    .capacity_resource
                    .unwrap_or(TurnCapacityResource::Replayed),
                cap: self.capacity_cap.unwrap_or_default(),
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
            // Legacy persisted busy submit records are intentionally not replayable.
            // Same-thread busy is a transient lock state, not an idempotent submit outcome.
            TurnIdempotencyReplay::SubmitThreadBusy(_) => None,
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

    pub fn replay_retry(&self) -> Option<Result<RetryTurnResponse, TurnError>> {
        if self.operation != TurnIdempotencyOperationKind::Retry {
            return None;
        }
        match &self.replay {
            TurnIdempotencyReplay::RetrySucceeded(response) => Some(Ok(response.clone())),
            // Same-thread busy is a transient lock state, not an idempotent retry outcome.
            TurnIdempotencyReplay::RetryThreadBusy(_) => None,
            TurnIdempotencyReplay::Error(error)
                if self.operation == TurnIdempotencyOperationKind::Retry =>
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loop_checkpoints: Vec<LoopCheckpointRecord>,
    pub idempotency_records: Vec<TurnIdempotencyRecord>,
    #[serde(default)]
    pub events: Vec<TurnLifecycleEvent>,
    #[serde(default)]
    pub event_retention_floor: EventCursor,
    #[serde(default)]
    pub admission_reservations: Vec<TurnAdmissionReservationRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawn_tree_reservations: Vec<SpawnTreeReservation>,
}

#[cfg(test)]
mod tests {
    use crate::{
        AcceptedMessageRef, EventCursor, GateResumeDisposition, ReplyTargetBindingRef,
        SourceBindingRef, TurnRunId, TurnRunRecord, TurnScope, TurnStatus,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    fn minimal_turn_run_record() -> TurnRunRecord {
        // Build a TurnRunRecord by serializing a struct-literal then
        // deserializing back so serde fills in all optional defaults.
        // We construct the profile via the same JSON shortcut used elsewhere
        // in test helpers (no ResolvedRunProfile needed).
        let scope = TurnScope::new(
            TenantId::new("tenant-store-test").unwrap(),
            Some(AgentId::new("agent-store-test").unwrap()),
            Some(ProjectId::new("project-store-test").unwrap()),
            ThreadId::new("thread-store-test").unwrap(),
        );
        let profile: crate::TurnRunProfile = serde_json::from_value(serde_json::json!({
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false,
        }))
        .expect("profile deserialization");
        TurnRunRecord {
            run_id: TurnRunId::new(),
            turn_id: crate::TurnId::new(),
            scope,
            accepted_message_ref: AcceptedMessageRef::new("accepted-store-test").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-store-test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-store-test").unwrap(),
            status: TurnStatus::Completed,
            profile,
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: vec![],
            failure: None,
            event_cursor: EventCursor(0),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    #[test]
    fn turn_run_record_resume_disposition_defaults_to_none_when_absent() {
        // (a) Deserialize a real TurnRunRecord JSON with auth_resume_disposition key ABSENT.
        // This proves #[serde(default)] is in place on the field.
        let record = minimal_turn_run_record();
        let mut json_val =
            serde_json::to_value(&record).expect("serialize TurnRunRecord with None disposition");

        // The key must already be absent due to skip_serializing_if = "Option::is_none".
        let obj = json_val
            .as_object_mut()
            .expect("TurnRunRecord must serialize to JSON object");
        assert!(
            !obj.contains_key("auth_resume_disposition"),
            "auth_resume_disposition must be absent when resume_disposition is None"
        );

        // Belt-and-suspenders: forcibly remove the key then deserialize.
        obj.remove("auth_resume_disposition");
        let deserialized: TurnRunRecord =
            serde_json::from_value(json_val).expect("deserialize TurnRunRecord missing key");
        assert_eq!(
            deserialized.resume_disposition, None,
            "resume_disposition must default to None when the JSON key is absent"
        );

        // (b) Deserialize a real TurnRunRecord JSON carrying the LEGACY key
        // "auth_resume_disposition": "denied". This proves the serde rename/back-compat.
        let record2 = minimal_turn_run_record();
        let mut json_val2 =
            serde_json::to_value(&record2).expect("serialize TurnRunRecord for legacy key test");
        let obj2 = json_val2
            .as_object_mut()
            .expect("TurnRunRecord must serialize to JSON object");
        obj2.insert(
            "auth_resume_disposition".to_string(),
            serde_json::json!("denied"),
        );

        let deserialized2: TurnRunRecord =
            serde_json::from_value(json_val2).expect("deserialize TurnRunRecord with legacy key");
        assert_eq!(
            deserialized2.resume_disposition,
            Some(GateResumeDisposition::Denied),
            "resume_disposition must be Some(Denied) when legacy key auth_resume_disposition is present"
        );
    }
}
