use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    GithubIssueBlockState, GithubIssueProviderActionId, GithubIssueProviderActionRecord,
    GithubIssueProviderBinding, GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowError,
    GithubIssueWorkflowEvent, GithubIssueWorkflowEventType, GithubIssueWorkflowMode,
    GithubIssueWorkflowRun, GithubIssueWorkflowRunId, GithubIssueWorkflowRunStatus,
    GithubIssueWorkspaceSessionId, GithubProviderRef, ProviderActionKind,
    ProviderActionReconciliationStrategy, ProviderActionStatus, WorkflowEventEnvelope,
    WorkflowIdempotencyKey, WorkflowStepRun, WorkflowStepRunId, WorkflowStepStatus,
    WorkflowWorkerId,
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
    pub workspace_session_id: Option<GithubIssueWorkspaceSessionId>,
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
