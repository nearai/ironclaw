use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    GithubIssueProviderActionId, GithubIssueStageRunId, GithubIssueWorkflowRunId,
    WorkflowIdempotencyKey, WorkflowStepRunId, WorkflowWorkerId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderActionStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Reconciling,
    NeedsReconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueProviderActionRecord {
    pub provider_action_id: GithubIssueProviderActionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub step_run_id: Option<WorkflowStepRunId>,
    pub name: String,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub input_hash: String,
    pub status: ProviderActionStatus,
    pub provider_ref_kind: Option<String>,
    pub provider_ref: Option<String>,
    pub stable_marker: Option<String>,
    pub reconciliation_strategy: String,
    pub lease_owner: Option<WorkflowWorkerId>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub attempt_count: u32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_reconciled_at: Option<DateTime<Utc>>,
    pub result: Option<JsonValue>,
    pub redacted_failure_kind: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
