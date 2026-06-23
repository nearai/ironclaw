use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    GithubCommentRef, GithubIssueBlockKind, GithubIssueProviderActionId, GithubIssueRef,
    GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowEventId, GithubIssueWorkflowRunId,
    GithubPullRequestRef, ProviderActionStatus, WorkflowIdempotencyKey,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GithubProviderRef {
    pub system: String,
    pub resource_type: String,
    pub owner: String,
    pub repo: String,
    pub provider_id: String,
    pub provider_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEventEnvelope<TPayload> {
    pub source_kind: WorkflowEventSourceKind,
    pub source_delivery_id: Option<String>,
    pub provider: GithubProviderRef,
    pub observed_at: DateTime<Utc>,
    pub provider_updated_at: Option<DateTime<Utc>>,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub payload_schema: String,
    pub payload: TPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventSourceKind {
    Poller,
    GithubWebhook,
    BenchmarkWebhook,
    ManualOperator,
    WorkflowInternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GithubIssueWorkflowEventType {
    #[serde(rename = "github.issue.discovered")]
    GithubIssueDiscovered,
    #[serde(rename = "github.issue.changed")]
    GithubIssueChanged,
    #[serde(rename = "github.issue.closed")]
    GithubIssueClosed,
    #[serde(rename = "github.pr.opened")]
    GithubPullRequestOpened,
    #[serde(rename = "github.pr.updated")]
    GithubPullRequestUpdated,
    #[serde(rename = "github.checks.changed")]
    GithubChecksChanged,
    #[serde(rename = "github.checks.failed")]
    GithubChecksFailed,
    #[serde(rename = "github.checks.succeeded")]
    GithubChecksSucceeded,
    #[serde(rename = "github.review_comment.created")]
    GithubReviewCommentCreated,
    #[serde(rename = "stage.completed")]
    StageCompleted,
    #[serde(rename = "provider_action.changed")]
    ProviderActionChanged,
    #[serde(rename = "workflow_run.blocked")]
    WorkflowRunBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowEvent {
    pub workflow_event_id: GithubIssueWorkflowEventId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub sequence: i64,
    pub workflow_event_type: GithubIssueWorkflowEventType,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub source_kind: WorkflowEventSourceKind,
    pub source_delivery_id: Option<String>,
    pub provider: GithubProviderRef,
    pub provider_updated_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub supersedes_workflow_event_id: Option<GithubIssueWorkflowEventId>,
    pub payload_schema: String,
    pub payload: JsonValue,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueDiscoveredPayload {
    pub issue: GithubIssueRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueChangedPayload {
    pub issue: GithubIssueRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueClosedPayload {
    pub issue: GithubIssueRef,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPullRequestOpenedPayload {
    pub pull_request: GithubPullRequestRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPullRequestUpdatedPayload {
    pub pull_request: GithubPullRequestRef,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubChecksChangedPayload {
    pub pull_request: Option<GithubPullRequestRef>,
    pub head_sha: String,
    pub suite_or_run_id: String,
    pub conclusion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubReviewCommentCreatedPayload {
    pub pull_request: Option<GithubPullRequestRef>,
    pub comment: GithubCommentRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCompletedPayload {
    pub stage_run_id: GithubIssueStageRunId,
    pub stage: GithubIssueStage,
    pub schema_version: String,
    pub result: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderActionChangedPayload {
    pub provider_action_id: GithubIssueProviderActionId,
    pub status: ProviderActionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunBlockedPayload {
    pub kind: GithubIssueBlockKind,
    pub reason: String,
    pub blocked_at: DateTime<Utc>,
}

pub fn issue_discovered_key(issue: &GithubIssueRef) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("issue:{}:discovered", issue_identity(issue)))
}

pub fn issue_changed_key(
    issue: &GithubIssueRef,
    provider_updated_at: Option<DateTime<Utc>>,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "issue:{}:updated:{}",
        issue_identity(issue),
        timestamp_identity(provider_updated_at)
    ))
}

pub fn issue_closed_key(
    issue: &GithubIssueRef,
    closed_at: Option<DateTime<Utc>>,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "issue:{}:closed:{}",
        issue_identity(issue),
        timestamp_identity(closed_at)
    ))
}

pub fn pr_opened_key(pr: &GithubPullRequestRef) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("pr:{}:opened", pr_identity(pr)))
}

pub fn pr_updated_key(
    pr: &GithubPullRequestRef,
    provider_updated_at: Option<DateTime<Utc>>,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "pr:{}:updated:{}",
        pr_identity(pr),
        timestamp_identity(provider_updated_at)
    ))
}

pub fn checks_changed_key(
    head_sha: &str,
    suite_or_run_id: &str,
    conclusion: &str,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "checks:{head_sha}:{suite_or_run_id}:{conclusion}"
    ))
}

pub fn checks_failed_key(head_sha: &str, suite_or_run_id: &str) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("checks:{head_sha}:{suite_or_run_id}:failed"))
}

pub fn checks_succeeded_key(head_sha: &str, suite_or_run_id: &str) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("checks:{head_sha}:{suite_or_run_id}:succeeded"))
}

pub fn review_comment_created_key(comment_node_id: &str) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("review-comment:{comment_node_id}"))
}

pub fn stage_result_reported_key(
    stage_run_id: &GithubIssueStageRunId,
    schema_version: &str,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!("stage-result:{stage_run_id}:{schema_version}"))
}

fn issue_identity(issue: &GithubIssueRef) -> String {
    issue
        .node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}#{}", issue.owner, issue.repo, issue.number))
}

fn pr_identity(pr: &GithubPullRequestRef) -> String {
    pr.node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}#{}", pr.owner, pr.repo, pr.number))
}

fn timestamp_identity(timestamp: Option<DateTime<Utc>>) -> String {
    timestamp
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Nanos, true))
        .unwrap_or_else(|| "unknown".to_string())
}
