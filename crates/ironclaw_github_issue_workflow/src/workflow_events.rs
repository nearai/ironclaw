use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::{
    GithubCommentRef, GithubIssueBlockKind, GithubIssueProviderActionId, GithubIssueRef,
    GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowError, GithubIssueWorkflowEventId,
    GithubIssueWorkflowRunId, GithubProviderBindingRef, GithubPullRequestRef, ProviderActionStatus,
    WorkflowIdempotencyKey,
    provider_bindings::{issue_binding_ref, primary_pr_binding_ref},
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
pub struct NormalizeGithubWebhookEventInput {
    pub source_delivery_id: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub actor: Option<GithubWebhookActor>,
    pub workflow_actor: Option<GithubWebhookActor>,
    #[serde(default)]
    pub matched_provider_bindings: Vec<GithubProviderBindingRef>,
    pub observation: GithubWebhookObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWebhookActor {
    pub login: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GithubWebhookObservation {
    Issues(GithubIssueWebhookObservation),
    IssueComment(GithubWebhookIssueCommentObservation),
    PullRequestReviewComment(GithubPullRequestReviewCommentWebhookObservation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWebhookObservation {
    pub action: GithubIssueWebhookAction,
    pub issue: GithubIssueWebhookSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWebhookAction {
    Opened,
    Edited,
    Deleted,
    Pinned,
    Unpinned,
    Closed,
    Reopened,
    Assigned,
    Unassigned,
    Labeled,
    Unlabeled,
    Locked,
    Unlocked,
    Transferred,
    Milestoned,
    Demilestoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWebhookSnapshot {
    pub issue: GithubIssueRef,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub comment_count: Option<usize>,
    pub body_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWebhookIssueCommentObservation {
    pub action: GithubWebhookIssueCommentAction,
    pub issue: GithubIssueRef,
    pub pull_request: Option<GithubPullRequestRef>,
    pub comment: GithubCommentRef,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubWebhookIssueCommentAction {
    Created,
    Edited,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPullRequestReviewCommentWebhookObservation {
    pub action: GithubPullRequestReviewCommentWebhookAction,
    pub pull_request: GithubPullRequestRef,
    pub comment: GithubCommentRef,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubPullRequestReviewCommentWebhookAction {
    Created,
    Edited,
    Deleted,
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

pub const GITHUB_ISSUE_CHANGED_PAYLOAD_SCHEMA: &str = "github.issue.changed.v1";
pub const GITHUB_ISSUE_CLOSED_PAYLOAD_SCHEMA: &str = "github.issue.closed.v1";
pub const GITHUB_REVIEW_COMMENT_CREATED_PAYLOAD_SCHEMA: &str = "github.review_comment.created.v1";

pub fn normalize_github_webhook_event(
    input: NormalizeGithubWebhookEventInput,
) -> Result<Vec<WorkflowEventEnvelope<JsonValue>>, GithubIssueWorkflowError> {
    match input.observation.clone() {
        GithubWebhookObservation::Issues(observation) => {
            normalize_issues_webhook_event(&input, observation)
        }
        GithubWebhookObservation::IssueComment(observation) => {
            normalize_issue_comment_webhook_event(&input, observation)
        }
        GithubWebhookObservation::PullRequestReviewComment(observation) => {
            normalize_pull_request_review_comment_webhook_event(&input, observation)
        }
    }
}

fn normalize_issues_webhook_event(
    input: &NormalizeGithubWebhookEventInput,
    observation: GithubIssueWebhookObservation,
) -> Result<Vec<WorkflowEventEnvelope<JsonValue>>, GithubIssueWorkflowError> {
    if matches!(observation.action, GithubIssueWebhookAction::Deleted) {
        return Ok(Vec::new());
    }

    let provider = issue_binding_ref(&observation.issue.issue).provider_ref;
    if !has_provider_binding_for(input, &provider) {
        return Ok(Vec::new());
    }
    if should_suppress_self_authored_echo(input, &provider) {
        return Ok(Vec::new());
    }

    if matches!(observation.action, GithubIssueWebhookAction::Closed) {
        let closed_at = observation.issue.closed_at.or(observation.issue.updated_at);
        return Ok(vec![WorkflowEventEnvelope {
            source_kind: WorkflowEventSourceKind::GithubWebhook,
            source_delivery_id: input.source_delivery_id.clone(),
            provider,
            observed_at: input.observed_at,
            provider_updated_at: observation.issue.updated_at,
            idempotency_key: issue_closed_key(&observation.issue.issue, closed_at),
            payload_schema: GITHUB_ISSUE_CLOSED_PAYLOAD_SCHEMA.to_string(),
            payload: serde_json::to_value(GithubIssueClosedPayload {
                issue: observation.issue.issue,
                closed_at,
            })
            .map_err(webhook_payload_error)?,
        }]);
    }

    Ok(vec![WorkflowEventEnvelope {
        source_kind: WorkflowEventSourceKind::GithubWebhook,
        source_delivery_id: input.source_delivery_id.clone(),
        provider,
        observed_at: input.observed_at,
        provider_updated_at: observation.issue.updated_at,
        idempotency_key: issue_changed_key(&observation.issue.issue, observation.issue.updated_at),
        payload_schema: GITHUB_ISSUE_CHANGED_PAYLOAD_SCHEMA.to_string(),
        payload: github_issue_webhook_payload(observation.issue),
    }])
}

fn normalize_issue_comment_webhook_event(
    input: &NormalizeGithubWebhookEventInput,
    observation: GithubWebhookIssueCommentObservation,
) -> Result<Vec<WorkflowEventEnvelope<JsonValue>>, GithubIssueWorkflowError> {
    if !matches!(observation.action, GithubWebhookIssueCommentAction::Created) {
        return Ok(Vec::new());
    }

    let Some(pull_request) = observation.pull_request else {
        return Ok(Vec::new());
    };
    let provider = issue_comment_provider_ref(&observation.issue, &observation.comment);
    if !has_pull_request_or_comment_binding_for(input, &pull_request, &provider) {
        return Ok(Vec::new());
    }
    if should_suppress_self_authored_echo(input, &provider) {
        return Ok(Vec::new());
    }

    pr_comment_envelope(
        input,
        provider,
        pull_request,
        observation.comment,
        observation.updated_at,
    )
}

fn normalize_pull_request_review_comment_webhook_event(
    input: &NormalizeGithubWebhookEventInput,
    observation: GithubPullRequestReviewCommentWebhookObservation,
) -> Result<Vec<WorkflowEventEnvelope<JsonValue>>, GithubIssueWorkflowError> {
    if !matches!(
        observation.action,
        GithubPullRequestReviewCommentWebhookAction::Created
    ) {
        return Ok(Vec::new());
    }

    let provider = review_comment_provider_ref(&observation.pull_request, &observation.comment);
    if !has_pull_request_or_comment_binding_for(input, &observation.pull_request, &provider) {
        return Ok(Vec::new());
    }
    if should_suppress_self_authored_echo(input, &provider) {
        return Ok(Vec::new());
    }

    pr_comment_envelope(
        input,
        provider,
        observation.pull_request,
        observation.comment,
        observation.updated_at,
    )
}

fn pr_comment_envelope(
    input: &NormalizeGithubWebhookEventInput,
    provider: GithubProviderRef,
    pull_request: GithubPullRequestRef,
    comment: GithubCommentRef,
    provider_updated_at: DateTime<Utc>,
) -> Result<Vec<WorkflowEventEnvelope<JsonValue>>, GithubIssueWorkflowError> {
    let comment_identity = comment_identity(&comment);
    Ok(vec![WorkflowEventEnvelope {
        source_kind: WorkflowEventSourceKind::GithubWebhook,
        source_delivery_id: input.source_delivery_id.clone(),
        provider,
        observed_at: input.observed_at,
        provider_updated_at: Some(provider_updated_at),
        idempotency_key: review_comment_created_key(&comment_identity),
        payload_schema: GITHUB_REVIEW_COMMENT_CREATED_PAYLOAD_SCHEMA.to_string(),
        payload: serde_json::to_value(GithubReviewCommentCreatedPayload {
            pull_request: Some(pull_request),
            comment,
        })
        .map_err(webhook_payload_error)?,
    }])
}

fn github_issue_webhook_payload(snapshot: GithubIssueWebhookSnapshot) -> JsonValue {
    let comment_count = snapshot.comment_count.unwrap_or_default();
    json!({
        "issue": snapshot.issue,
        "provider_snapshot": {
            "title": snapshot.title,
            "state": snapshot.state,
            "labels": snapshot.labels,
            "updated_at": snapshot.updated_at,
            "comment_count": comment_count,
            "body_present": snapshot.body_present,
        }
    })
}

fn has_pull_request_or_comment_binding_for(
    input: &NormalizeGithubWebhookEventInput,
    pull_request: &GithubPullRequestRef,
    comment_provider: &GithubProviderRef,
) -> bool {
    let pull_request_provider = primary_pr_binding_ref(pull_request).provider_ref;
    has_provider_binding_for(input, &pull_request_provider)
        || has_provider_binding_for(input, comment_provider)
}

fn has_provider_binding_for(
    input: &NormalizeGithubWebhookEventInput,
    provider: &GithubProviderRef,
) -> bool {
    input
        .matched_provider_bindings
        .iter()
        .any(|binding| binding_matches_provider(binding, provider))
}

fn should_suppress_self_authored_echo(
    input: &NormalizeGithubWebhookEventInput,
    provider: &GithubProviderRef,
) -> bool {
    input
        .actor
        .as_ref()
        .zip(input.workflow_actor.as_ref())
        .is_some_and(|(actor, workflow_actor)| {
            same_webhook_actor(actor, workflow_actor) && has_provider_binding_for(input, provider)
        })
}

fn same_webhook_actor(actor: &GithubWebhookActor, workflow_actor: &GithubWebhookActor) -> bool {
    match (actor.node_id.as_deref(), workflow_actor.node_id.as_deref()) {
        (Some(actor_node_id), Some(workflow_node_id))
            if !actor_node_id.is_empty() && !workflow_node_id.is_empty() =>
        {
            actor_node_id == workflow_node_id
        }
        _ => actor.login.eq_ignore_ascii_case(&workflow_actor.login),
    }
}

fn binding_matches_provider(
    binding: &GithubProviderBindingRef,
    provider: &GithubProviderRef,
) -> bool {
    binding.provider_ref.system == provider.system
        && binding.provider_ref.resource_type == provider.resource_type
        && binding.provider_ref.owner == provider.owner
        && binding.provider_ref.repo == provider.repo
        && binding.provider_ref.provider_id == provider.provider_id
}

fn issue_comment_provider_ref(
    issue: &GithubIssueRef,
    comment: &GithubCommentRef,
) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "issue_comment".to_string(),
        owner: issue.owner.clone(),
        repo: issue.repo.clone(),
        provider_id: comment_identity(comment),
        provider_url: Some(comment.url.clone()),
    }
}

fn review_comment_provider_ref(
    pull_request: &GithubPullRequestRef,
    comment: &GithubCommentRef,
) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "review_comment".to_string(),
        owner: pull_request.owner.clone(),
        repo: pull_request.repo.clone(),
        provider_id: comment_identity(comment),
        provider_url: Some(comment.url.clone()),
    }
}

fn comment_identity(comment: &GithubCommentRef) -> String {
    comment
        .node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty())
        .unwrap_or(comment.url.as_str())
        .to_string()
}

fn webhook_payload_error(error: serde_json::Error) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("failed to serialize GitHub webhook workflow event payload: {error}"),
    }
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
