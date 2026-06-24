use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    GithubIssueBlockState, GithubIssueProviderActionId, GithubIssueProviderActionRecord,
    GithubIssueProviderBinding, GithubIssueProviderSnapshotSummary, GithubIssueStage,
    GithubIssueStageRunId, GithubIssueWorkflowError, GithubIssueWorkflowEvent,
    GithubIssueWorkflowEventType, GithubIssueWorkflowMode, GithubIssueWorkflowRun,
    GithubIssueWorkflowRunId, GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession,
    GithubProviderAccountRef, GithubProviderRef, GithubPullRequestRef, GithubRepositorySelector,
    ProviderActionKind, ProviderActionReconciliationStrategy, ProviderActionStatus,
    WorkflowEventEnvelope, WorkflowIdempotencyKey, WorkflowStepRun, WorkflowStepRunId,
    WorkflowStepStatus, WorkflowWorkerId,
};

#[async_trait]
pub trait GithubIssueWorkflowRepository: Send + Sync {
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError>;

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError>;

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError>;

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError>;

    async fn list_active_workflow_runs_for_repository(
        &self,
        input: ListActiveWorkflowRunsForRepositoryInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError>;

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError>;

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError>;

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError>;

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError>;

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError>;

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError>;

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError>;

    /// Read the stage run row by id. Returns `None` when the row is absent
    /// (legacy run before stage rows were persisted, or never created). Used by
    /// the stuck-stage reconciler to read the stage-level staleness clock.
    async fn get_stage_run(
        &self,
        input: GetStageRunInput,
    ) -> Result<Option<StageRunSnapshot>, GithubIssueWorkflowError>;

    /// Mark a stage run row failed (active -> false, failed -> true). Used by
    /// the stuck-stage reconciler before the run is escalated to a
    /// `RecoveryRequired` block. The run-row `active_stage_run_id` pointer is
    /// cleared separately by `block_workflow_run`, not here, so the two writes
    /// stay independent and the sequence is crash-idempotent.
    async fn fail_stage_run(
        &self,
        input: FailStageRunInput,
    ) -> Result<FailStageRunOutcome, GithubIssueWorkflowError>;

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError>;

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError>;

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError>;

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError>;

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError>;

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOrGetWorkflowRunInput {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub provider_account_ref: Option<GithubProviderAccountRef>,
    pub issue_ref: crate::GithubIssueRef,
    pub workflow_policy_key: String,
    pub workflow_policy_version: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum CreateOrGetWorkflowRunOutcome {
    Created { run: GithubIssueWorkflowRun },
    Existing { run: GithubIssueWorkflowRun },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordWorkflowEventInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub workflow_event_type: GithubIssueWorkflowEventType,
    pub envelope: WorkflowEventEnvelope<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordWorkflowEventOutcome {
    Recorded { event: GithubIssueWorkflowEvent },
    Duplicate { existing: GithubIssueWorkflowEvent },
    Superseded { existing: GithubIssueWorkflowEvent },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowEventsAfterInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub after_sequence: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRunnableWorkflowRunsInput {
    pub tenant_id: TenantId,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListActiveWorkflowRunsForRepositoryInput {
    pub tenant_id: TenantId,
    pub repository: GithubRepositorySelector,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewWorkflowRunLeaseInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum LeaseRenewalOutcome {
    Renewed { run: GithubIssueWorkflowRun },
    NotLeaseOwner,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseWorkflowRunLeaseInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum LeaseReleaseOutcome {
    Released { run: GithubIssueWorkflowRun },
    NotLeaseOwner,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWorkflowRunInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub worker_id: WorkflowWorkerId,
    pub active_block: GithubIssueBlockState,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum BlockWorkflowRunOutcome {
    Blocked { run: GithubIssueWorkflowRun },
    NotLeaseOwner,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindLatestWorkflowEventForProviderInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub workflow_event_types: Vec<GithubIssueWorkflowEventType>,
    pub provider: GithubProviderRef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunTransition {
    pub status: Option<GithubIssueWorkflowRunStatus>,
    pub mode: Option<GithubIssueWorkflowMode>,
    pub active_block: Option<GithubIssueBlockState>,
    pub clear_active_block: bool,
    pub latest_provider_snapshot: Option<GithubIssueProviderSnapshotSummary>,
    pub workspace_session: Option<GithubIssueWorkspaceSession>,
    pub primary_pr: Option<GithubPullRequestRef>,
    /// The claim comment posted on the issue when the run was claimed. Captured
    /// from the claim provider action so a later stage can edit that comment to
    /// link the draft PR. `#[serde(default)]` keeps legacy persisted transition
    /// rows (which predate this field) deserializable.
    #[serde(default)]
    pub claim_comment: Option<crate::GithubCommentRef>,
    /// Outcome of the independent verification gate, persisted onto the run's
    /// workflow_state so PrSynthesis (and replays) can surface it.
    #[serde(default)]
    pub last_verification: Option<crate::WorkflowVerificationSummary>,
}

/// Apply every field of a [`WorkflowRunTransition`] onto a run, in the order the
/// backends agreed on. This is the SINGLE place the transition's fields are
/// projected onto `GithubIssueWorkflowRun`/`GithubIssueWorkflowState`: both the
/// in-memory backend and the durable filesystem backend (inside its CAS retry
/// loop) call it, so the two cannot silently diverge — a divergence already
/// shipped once when the durable backend dropped `latest_provider_snapshot`.
///
/// The transition is destructured below so that adding a new field to
/// [`WorkflowRunTransition`] is a COMPILE ERROR here until the new field is
/// handled — that is the mechanism that prevents the next dropped field. The
/// caller is responsible for the event-cursor advance, version bump, timestamp,
/// and terminal-state lease/stage clearing; this helper only projects the
/// transition's own fields.
///
/// Ordering note: `clear_active_block` is honored BEFORE `active_block`, so a
/// transition that both clears and sets ends with the set block. This preserves
/// the original hand-rolled if-let ordering both backends used.
pub fn apply_workflow_run_transition(
    run: &mut GithubIssueWorkflowRun,
    transition: &WorkflowRunTransition,
) {
    let WorkflowRunTransition {
        status,
        mode,
        active_block,
        clear_active_block,
        latest_provider_snapshot,
        workspace_session,
        primary_pr,
        claim_comment,
        last_verification,
    } = transition;

    if let Some(status) = status.clone() {
        run.status = status;
    }
    if let Some(mode) = mode.clone() {
        run.workflow_state.mode = mode;
    }
    if *clear_active_block {
        run.workflow_state.active_block = None;
    }
    if let Some(active_block) = active_block.clone() {
        run.workflow_state.active_block = Some(active_block);
    }
    if let Some(provider_snapshot) = latest_provider_snapshot.clone() {
        run.workflow_state.latest_provider_snapshot = Some(provider_snapshot);
    }
    if let Some(workspace_session) = workspace_session.clone() {
        run.workspace_session_id = Some(workspace_session.workspace_session_id);
        run.workflow_state.current_workspace_ref = Some(workspace_session.workspace_ref);
        run.workflow_state.current_workspace_mount_ref = Some(workspace_session.mount_ref);
    }
    if let Some(primary_pr) = primary_pr.clone() {
        run.workflow_state.primary_pr = Some(primary_pr);
    }
    if let Some(claim_comment) = claim_comment.clone() {
        run.workflow_state.claim_comment = Some(claim_comment);
    }
    if let Some(last_verification) = last_verification.clone() {
        run.workflow_state.last_verification = Some(last_verification);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceWorkflowRunInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub worker_id: WorkflowWorkerId,
    pub expected_workflow_run_version: i64,
    pub expected_event_cursor: i64,
    pub next_event_cursor: i64,
    pub transition: WorkflowRunTransition,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum TransitionOutcome {
    Applied { run: GithubIssueWorkflowRun },
    VersionConflict { current: GithubIssueWorkflowRun },
    NotLeaseOwner,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateStageRunInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage: GithubIssueStage,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum CreateStageRunOutcome {
    Created {
        stage_run_id: GithubIssueStageRunId,
        run: GithubIssueWorkflowRun,
    },
    ActiveStageExists {
        existing_stage_run_id: GithubIssueStageRunId,
        run: GithubIssueWorkflowRun,
    },
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptStageResultInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub result: JsonValue,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum AcceptStageResultOutcome {
    Accepted { run: GithubIssueWorkflowRun },
    NotActiveStage { run: GithubIssueWorkflowRun },
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStageRunInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
}

/// A read-only view of a stage run row, used by the stuck-stage reconciler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRunSnapshot {
    pub stage_run_id: GithubIssueStageRunId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage: GithubIssueStage,
    pub active: bool,
    pub failed: bool,
    pub created_at: DateTime<Utc>,
    /// Stage-level liveness clock, stamped when the stage row is written. This
    /// is the staleness signal the reconciler tests against `stage_stale_after`.
    /// It is deliberately decoupled from the run-level lease (which is renewed
    /// every poller tick and therefore never goes stale on a stuck stage).
    pub last_heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailStageRunInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailStageRunOutcome {
    /// The stage row was flipped to failed/inactive.
    Failed { stage_run_id: GithubIssueStageRunId },
    /// The stage row was already inactive (accepted or previously failed) — the
    /// op is idempotent, so a retried reconcile is a no-op.
    AlreadyInactive,
    /// No stage row exists for this run/id pair.
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOrGetWorkflowStepInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub step_name: String,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub input_hash: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum CreateOrGetWorkflowStepOutcome {
    Created { step: WorkflowStepRun },
    Existing { step: WorkflowStepRun },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteWorkflowStepInput {
    pub step_run_id: WorkflowStepRunId,
    pub status: WorkflowStepStatus,
    pub result: Option<JsonValue>,
    pub error: Option<JsonValue>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum CompleteWorkflowStepOutcome {
    Completed { step: WorkflowStepRun },
    AlreadyCompleted { step: WorkflowStepRun },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOrGetProviderActionInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub step_run_id: Option<WorkflowStepRunId>,
    pub name: String,
    pub kind: ProviderActionKind,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub input_hash: String,
    pub stable_marker: Option<String>,
    pub reconciliation_strategy: ProviderActionReconciliationStrategy,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimProviderActionInput {
    pub provider_action_id: GithubIssueProviderActionId,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum ClaimProviderActionOutcome {
    Claimed {
        action: GithubIssueProviderActionRecord,
    },
    AlreadyCompleted {
        action: GithubIssueProviderActionRecord,
    },
    Busy {
        action: GithubIssueProviderActionRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteProviderActionInput {
    pub provider_action_id: GithubIssueProviderActionId,
    pub worker_id: WorkflowWorkerId,
    pub status: ProviderActionStatus,
    pub provider_ref: Option<GithubProviderRef>,
    pub stable_marker: Option<String>,
    pub result: Option<JsonValue>,
    pub redacted_failure_kind: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum CompleteProviderActionOutcome {
    Completed {
        action: GithubIssueProviderActionRecord,
    },
    AlreadyCompleted {
        action: GithubIssueProviderActionRecord,
    },
    NotLeaseOwner {
        action: GithubIssueProviderActionRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertProviderBindingInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub provider_ref: GithubProviderRef,
    pub role: String,
    pub created_by_provider_action_id: Option<GithubIssueProviderActionId>,
    pub created_at: DateTime<Utc>,
}
