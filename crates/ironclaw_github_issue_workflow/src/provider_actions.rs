use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::{
    ClaimProviderActionInput, ClaimProviderActionOutcome, CompleteProviderActionInput,
    CompleteProviderActionOutcome, CreateDraftPullRequestInput, CreateIssueCommentInput,
    CreateOrGetProviderActionInput, GetAuthenticatedWorkflowActorInput, GithubCommentRef,
    GithubIssueProviderActionId, GithubIssueProviderBinding, GithubIssueStageRunId,
    GithubIssueWorkflowError, GithubIssueWorkflowPort, GithubIssueWorkflowRepository,
    GithubIssueWorkflowRun, GithubIssueWorkflowRunId, GithubProviderAccountRef,
    GithubProviderBindingRef, GithubProviderRef, GithubPullRequestRef, ListIssueCommentsInput,
    ListPullRequestsInput, UpsertProviderBindingInput, WorkflowIdempotencyKey, WorkflowStepRunId,
    WorkflowWorkerId, claim_comment_binding_ref, primary_pr_binding_ref,
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
#[serde(rename_all = "snake_case")]
pub enum ProviderActionKind {
    ClaimComment,
    IssueComment,
    Branch,
    DraftPullRequest,
    ReviewReply,
}

impl ProviderActionKind {
    pub fn as_name(&self) -> &'static str {
        match self {
            Self::ClaimComment => "claim_comment",
            Self::IssueComment => "issue_comment",
            Self::Branch => "branch",
            Self::DraftPullRequest => "draft_pull_request",
            Self::ReviewReply => "review_reply",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderActionReconciliationStrategy {
    ClaimCommentByMarker,
    IssueCommentByMarker,
    BranchByNameAndHeadSha,
    DraftPullRequestByHeadBranchAndMarker,
    ReviewReplyByParentCommentAndMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueProviderActionRecord {
    pub provider_action_id: GithubIssueProviderActionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub step_run_id: Option<WorkflowStepRunId>,
    pub name: String,
    pub kind: ProviderActionKind,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub input_hash: String,
    pub status: ProviderActionStatus,
    pub provider_ref: Option<GithubProviderRef>,
    pub stable_marker: Option<String>,
    pub reconciliation_strategy: ProviderActionReconciliationStrategy,
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

pub fn stable_claim_marker(run_id: &GithubIssueWorkflowRunId) -> String {
    stable_marker("claim", run_id.as_str())
}

pub fn stable_pr_marker(run_id: &GithubIssueWorkflowRunId) -> String {
    stable_marker("pr", run_id.as_str())
}

pub fn stable_issue_comment_marker(action_id: &GithubIssueProviderActionId) -> String {
    stable_marker("issue-comment", action_id.as_str())
}

fn stable_marker(kind: &str, id: &str) -> String {
    format!("<!-- ironclaw:github-bug-workflow:{kind}:{id} -->")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunClaimCommentProviderActionRequest {
    pub run: GithubIssueWorkflowRun,
    pub provider_account_ref: GithubProviderAccountRef,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDraftPullRequestProviderActionRequest {
    pub run: GithubIssueWorkflowRun,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub title: String,
    pub body: String,
    pub head_branch: String,
    pub base_branch: String,
    pub head_sha: String,
    pub provider_account_ref: GithubProviderAccountRef,
    pub worker_id: WorkflowWorkerId,
    pub now: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum ProviderActionRunOutcome {
    Succeeded {
        action: GithubIssueProviderActionRecord,
        binding: GithubIssueProviderBinding,
    },
    Replayed {
        action: GithubIssueProviderActionRecord,
    },
    NeedsReconciliation {
        action: GithubIssueProviderActionRecord,
    },
    Failed {
        action: GithubIssueProviderActionRecord,
    },
    Busy {
        action: GithubIssueProviderActionRecord,
    },
}

#[derive(Debug)]
pub struct GithubIssueProviderActionRunner<R: ?Sized, P: ?Sized> {
    repository: Arc<R>,
    port: Arc<P>,
}

impl<R, P> GithubIssueProviderActionRunner<R, P>
where
    R: GithubIssueWorkflowRepository + ?Sized,
    P: GithubIssueWorkflowPort + ?Sized,
{
    pub fn new(repository: Arc<R>, port: Arc<P>) -> Self {
        Self { repository, port }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            op = "claim_comment",
            workflow_run_id = %request.run.workflow_run_id,
            owner = %request.run.issue_ref.owner,
            repo = %request.run.issue_ref.repo,
            issue = request.run.issue_ref.number,
        )
    )]
    pub async fn run_claim_comment(
        &self,
        request: RunClaimCommentProviderActionRequest,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let marker = stable_claim_marker(&request.run.workflow_run_id);
        let kind = ProviderActionKind::ClaimComment;
        let action = self
            .repository
            .create_or_get_provider_action(CreateOrGetProviderActionInput {
                workflow_run_id: request.run.workflow_run_id.clone(),
                stage_run_id: None,
                step_run_id: None,
                name: kind.as_name().to_string(),
                kind,
                idempotency_key: claim_comment_idempotency_key(&request.run.workflow_run_id),
                input_hash: claim_comment_input_hash(&request.run, &marker),
                stable_marker: Some(marker.clone()),
                reconciliation_strategy: ProviderActionReconciliationStrategy::ClaimCommentByMarker,
                now: request.now,
            })
            .await?;

        let claimed = self
            .repository
            .claim_provider_action(ClaimProviderActionInput {
                provider_action_id: action.provider_action_id,
                worker_id: request.worker_id.clone(),
                now: request.now,
                lease_expires_at: request.lease_expires_at,
            })
            .await?;

        match claimed {
            ClaimProviderActionOutcome::Claimed { action } => {
                debug!(claim = "claimed", "executing claim-comment provider action");
                self.run_claimed_claim_comment(request, action, marker)
                    .await
            }
            ClaimProviderActionOutcome::AlreadyCompleted { action } => {
                debug!(claim = "already_completed", "claim-comment action replayed");
                Ok(ProviderActionRunOutcome::Replayed { action })
            }
            ClaimProviderActionOutcome::Busy { action } => {
                debug!(claim = "busy", "claim-comment action lease is busy");
                Ok(ProviderActionRunOutcome::Busy { action })
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            op = "draft_pull_request",
            workflow_run_id = %request.run.workflow_run_id,
            owner = %request.run.issue_ref.owner,
            repo = %request.run.issue_ref.repo,
            issue = request.run.issue_ref.number,
            account_id = %request.provider_account_ref.account_id,
        )
    )]
    pub async fn run_draft_pull_request(
        &self,
        request: RunDraftPullRequestProviderActionRequest,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let marker = stable_pr_marker(&request.run.workflow_run_id);
        let kind = ProviderActionKind::DraftPullRequest;
        let action = self
            .repository
            .create_or_get_provider_action(CreateOrGetProviderActionInput {
                workflow_run_id: request.run.workflow_run_id.clone(),
                stage_run_id: request.stage_run_id.clone(),
                step_run_id: None,
                name: kind.as_name().to_string(),
                kind,
                idempotency_key: draft_pr_idempotency_key(&request.run.workflow_run_id),
                input_hash: draft_pr_input_hash(&request, &marker),
                stable_marker: Some(marker.clone()),
                reconciliation_strategy:
                    ProviderActionReconciliationStrategy::DraftPullRequestByHeadBranchAndMarker,
                now: request.now,
            })
            .await?;

        let claimed = self
            .repository
            .claim_provider_action(ClaimProviderActionInput {
                provider_action_id: action.provider_action_id,
                worker_id: request.worker_id.clone(),
                now: request.now,
                lease_expires_at: request.lease_expires_at,
            })
            .await?;

        match claimed {
            ClaimProviderActionOutcome::Claimed { action } => {
                debug!(
                    claim = "claimed",
                    "executing draft-pull-request provider action"
                );
                self.run_claimed_draft_pull_request(request, action, marker)
                    .await
            }
            ClaimProviderActionOutcome::AlreadyCompleted { action } => {
                debug!(
                    claim = "already_completed",
                    "draft-pull-request action replayed"
                );
                Ok(ProviderActionRunOutcome::Replayed { action })
            }
            ClaimProviderActionOutcome::Busy { action } => {
                debug!(claim = "busy", "draft-pull-request action lease is busy");
                Ok(ProviderActionRunOutcome::Busy { action })
            }
        }
    }

    async fn run_claimed_claim_comment(
        &self,
        request: RunClaimCommentProviderActionRequest,
        action: GithubIssueProviderActionRecord,
        marker: String,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let actor = match self
            .port
            .get_authenticated_workflow_actor(GetAuthenticatedWorkflowActorInput {
                provider_account_ref: request.provider_account_ref.clone(),
                owner: request.run.issue_ref.owner.clone(),
                repo: request.run.issue_ref.repo.clone(),
            })
            .await
        {
            Ok(actor) => actor,
            Err(error) => {
                debug!(
                    %error,
                    op = "get_authenticated_workflow_actor",
                    workflow_run_id = %request.run.workflow_run_id,
                    failure_kind = "provider_read_failed",
                    "provider action failed before sanitizing failure"
                );
                let action = self
                    .complete_sanitized_failure(
                        &action,
                        &request.worker_id,
                        "provider_read_failed",
                        request.now,
                    )
                    .await?;
                return Ok(ProviderActionRunOutcome::Failed { action });
            }
        };

        let comments = match self
            .port
            .list_issue_comments(ListIssueCommentsInput {
                provider_account_ref: request.provider_account_ref.clone(),
                issue: request.run.issue_ref.clone(),
            })
            .await
        {
            Ok(comments) => comments,
            Err(error) => {
                debug!(
                    %error,
                    op = "list_issue_comments",
                    workflow_run_id = %request.run.workflow_run_id,
                    failure_kind = "provider_read_failed",
                    "provider action failed before sanitizing failure"
                );
                let action = self
                    .complete_sanitized_failure(
                        &action,
                        &request.worker_id,
                        "provider_read_failed",
                        request.now,
                    )
                    .await?;
                return Ok(ProviderActionRunOutcome::Failed { action });
            }
        };

        let matching_comments: Vec<_> = comments
            .iter()
            .filter(|comment| comment.body.contains(&marker))
            .collect();
        if matching_comments.len() > 1 {
            debug!(
                matching = matching_comments.len(),
                failure_kind = "ambiguous_claim_comment",
                "multiple claim-comment markers found; needs reconciliation"
            );
            let action = self
                .complete_needs_reconciliation(
                    &action,
                    &request.worker_id,
                    "ambiguous_claim_comment",
                    request.now,
                )
                .await?;
            return Ok(ProviderActionRunOutcome::NeedsReconciliation { action });
        }

        if let Some(existing_comment) = matching_comments.first() {
            if existing_comment.author_login == actor.login {
                let binding_ref = claim_comment_success_binding_ref(
                    &request.run,
                    &marker,
                    &existing_comment.comment,
                );
                return self
                    .complete_claim_comment_success(
                        &action,
                        &request.worker_id,
                        ClaimCommentSuccess {
                            marker: marker.clone(),
                            comment: existing_comment.comment.clone(),
                            binding_ref,
                            echo_suppressed: true,
                        },
                        request.now,
                    )
                    .await;
            }

            debug!(
                failure_kind = "claim_comment_marker_author_mismatch",
                "claim-comment marker authored by another actor; needs reconciliation"
            );
            let action = self
                .complete_needs_reconciliation(
                    &action,
                    &request.worker_id,
                    "claim_comment_marker_author_mismatch",
                    request.now,
                )
                .await?;
            return Ok(ProviderActionRunOutcome::NeedsReconciliation { action });
        }

        let body = claim_comment_body(&marker);
        let comment = match self
            .port
            .create_issue_comment(CreateIssueCommentInput {
                provider_account_ref: request.provider_account_ref.clone(),
                issue: request.run.issue_ref.clone(),
                body,
            })
            .await
        {
            Ok(comment) => comment,
            Err(error) => {
                debug!(
                    %error,
                    op = "create_issue_comment",
                    workflow_run_id = %request.run.workflow_run_id,
                    failure_kind = "provider_write_failed",
                    "provider action failed before sanitizing failure"
                );
                let action = self
                    .complete_sanitized_failure(
                        &action,
                        &request.worker_id,
                        "provider_write_failed",
                        request.now,
                    )
                    .await?;
                return Ok(ProviderActionRunOutcome::Failed { action });
            }
        };

        let binding_ref = claim_comment_success_binding_ref(&request.run, &marker, &comment);
        self.complete_claim_comment_success(
            &action,
            &request.worker_id,
            ClaimCommentSuccess {
                marker,
                comment,
                binding_ref,
                echo_suppressed: false,
            },
            request.now,
        )
        .await
    }

    async fn run_claimed_draft_pull_request(
        &self,
        request: RunDraftPullRequestProviderActionRequest,
        action: GithubIssueProviderActionRecord,
        marker: String,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let pull_requests = match self
            .port
            .list_pull_requests(ListPullRequestsInput {
                provider_account_ref: request.provider_account_ref.clone(),
                owner: request.run.issue_ref.owner.clone(),
                repo: request.run.issue_ref.repo.clone(),
                state: "open".to_string(),
                limit: 100,
            })
            .await
        {
            Ok(pull_requests) => pull_requests,
            Err(error) => {
                debug!(
                    %error,
                    op = "list_pull_requests",
                    workflow_run_id = %request.run.workflow_run_id,
                    failure_kind = "provider_read_failed",
                    "provider action failed before sanitizing failure"
                );
                let action = self
                    .complete_sanitized_failure(
                        &action,
                        &request.worker_id,
                        "provider_read_failed",
                        request.now,
                    )
                    .await?;
                return Ok(ProviderActionRunOutcome::Failed { action });
            }
        };

        let matching_pull_requests: Vec<_> = pull_requests
            .iter()
            .filter(|snapshot| {
                snapshot.pull_request.head_branch == request.head_branch
                    && snapshot.body.contains(&marker)
            })
            .collect();
        if matching_pull_requests.len() > 1 {
            debug!(
                matching = matching_pull_requests.len(),
                failure_kind = "ambiguous_draft_pull_request",
                "multiple draft pull requests match marker; needs reconciliation"
            );
            let action = self
                .complete_needs_reconciliation(
                    &action,
                    &request.worker_id,
                    "ambiguous_draft_pull_request",
                    request.now,
                )
                .await?;
            return Ok(ProviderActionRunOutcome::NeedsReconciliation { action });
        }
        if let Some(existing) = matching_pull_requests.first() {
            return self
                .complete_draft_pull_request_success(
                    &action,
                    &request.worker_id,
                    DraftPullRequestSuccess {
                        marker,
                        pull_request: existing.pull_request.clone(),
                        echo_suppressed: true,
                    },
                    request.now,
                )
                .await;
        }

        let body = draft_pr_body(&marker, &request.body);
        let pull_request = match self
            .port
            .create_draft_pull_request(CreateDraftPullRequestInput {
                provider_account_ref: request.provider_account_ref.clone(),
                owner: request.run.issue_ref.owner.clone(),
                repo: request.run.issue_ref.repo.clone(),
                title: request.title,
                body: Some(body),
                head_branch: request.head_branch,
                base_branch: request.base_branch,
            })
            .await
        {
            Ok(pull_request) => pull_request,
            Err(error) => {
                debug!(
                    %error,
                    op = "create_draft_pull_request",
                    workflow_run_id = %request.run.workflow_run_id,
                    failure_kind = "provider_write_failed",
                    "provider action failed before sanitizing failure"
                );
                let action = self
                    .complete_sanitized_failure(
                        &action,
                        &request.worker_id,
                        "provider_write_failed",
                        request.now,
                    )
                    .await?;
                return Ok(ProviderActionRunOutcome::Failed { action });
            }
        };

        self.complete_draft_pull_request_success(
            &action,
            &request.worker_id,
            DraftPullRequestSuccess {
                marker,
                pull_request,
                echo_suppressed: false,
            },
            request.now,
        )
        .await
    }

    async fn complete_claim_comment_success(
        &self,
        action: &GithubIssueProviderActionRecord,
        worker_id: &WorkflowWorkerId,
        success: ClaimCommentSuccess,
        now: DateTime<Utc>,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let completed = self
            .repository
            .complete_provider_action(CompleteProviderActionInput {
                provider_action_id: action.provider_action_id.clone(),
                worker_id: worker_id.clone(),
                status: ProviderActionStatus::Succeeded,
                provider_ref: Some(success.binding_ref.provider_ref.clone()),
                stable_marker: Some(success.marker.clone()),
                result: Some(json!({
                    "comment": success.comment,
                    "stable_marker": success.marker,
                    "echo_suppressed": success.echo_suppressed,
                })),
                redacted_failure_kind: None,
                now,
            })
            .await?;
        let CompleteProviderActionOutcome::Completed { action } = completed else {
            return Err(GithubIssueWorkflowError::Repository {
                reason: "provider action completion lost its lease".to_string(),
            });
        };

        let binding = self
            .repository
            .upsert_provider_binding(UpsertProviderBindingInput {
                workflow_run_id: action.workflow_run_id.clone(),
                provider_ref: success.binding_ref.provider_ref,
                role: success.binding_ref.role,
                created_by_provider_action_id: Some(action.provider_action_id.clone()),
                created_at: now,
            })
            .await?;

        Ok(ProviderActionRunOutcome::Succeeded { action, binding })
    }

    async fn complete_draft_pull_request_success(
        &self,
        action: &GithubIssueProviderActionRecord,
        worker_id: &WorkflowWorkerId,
        success: DraftPullRequestSuccess,
        now: DateTime<Utc>,
    ) -> Result<ProviderActionRunOutcome, GithubIssueWorkflowError> {
        let binding_ref = primary_pr_binding_ref(&success.pull_request);
        let completed = self
            .repository
            .complete_provider_action(CompleteProviderActionInput {
                provider_action_id: action.provider_action_id.clone(),
                worker_id: worker_id.clone(),
                status: ProviderActionStatus::Succeeded,
                provider_ref: Some(binding_ref.provider_ref.clone()),
                stable_marker: Some(success.marker.clone()),
                result: Some(json!({
                    "pull_request": success.pull_request,
                    "stable_marker": success.marker,
                    "echo_suppressed": success.echo_suppressed,
                })),
                redacted_failure_kind: None,
                now,
            })
            .await?;
        let CompleteProviderActionOutcome::Completed { action } = completed else {
            return Err(GithubIssueWorkflowError::Repository {
                reason: "provider action completion lost its lease".to_string(),
            });
        };

        let binding = self
            .repository
            .upsert_provider_binding(UpsertProviderBindingInput {
                workflow_run_id: action.workflow_run_id.clone(),
                provider_ref: binding_ref.provider_ref,
                role: binding_ref.role,
                created_by_provider_action_id: Some(action.provider_action_id.clone()),
                created_at: now,
            })
            .await?;

        Ok(ProviderActionRunOutcome::Succeeded { action, binding })
    }

    async fn complete_needs_reconciliation(
        &self,
        action: &GithubIssueProviderActionRecord,
        worker_id: &WorkflowWorkerId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        match self
            .repository
            .complete_provider_action(CompleteProviderActionInput {
                provider_action_id: action.provider_action_id.clone(),
                worker_id: worker_id.clone(),
                status: ProviderActionStatus::NeedsReconciliation,
                provider_ref: None,
                stable_marker: action.stable_marker.clone(),
                result: Some(json!({ "failure_kind": reason })),
                redacted_failure_kind: Some(reason.to_string()),
                now,
            })
            .await?
        {
            CompleteProviderActionOutcome::Completed { action } => Ok(action),
            CompleteProviderActionOutcome::AlreadyCompleted { action }
            | CompleteProviderActionOutcome::NotLeaseOwner { action } => Ok(action),
        }
    }

    async fn complete_sanitized_failure(
        &self,
        action: &GithubIssueProviderActionRecord,
        worker_id: &WorkflowWorkerId,
        failure_kind: &str,
        now: DateTime<Utc>,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        match self
            .repository
            .complete_provider_action(CompleteProviderActionInput {
                provider_action_id: action.provider_action_id.clone(),
                worker_id: worker_id.clone(),
                status: ProviderActionStatus::Failed,
                provider_ref: None,
                stable_marker: action.stable_marker.clone(),
                result: Some(json!({ "failure_kind": failure_kind })),
                redacted_failure_kind: Some(failure_kind.to_string()),
                now,
            })
            .await?
        {
            CompleteProviderActionOutcome::Completed { action } => Ok(action),
            CompleteProviderActionOutcome::AlreadyCompleted { action }
            | CompleteProviderActionOutcome::NotLeaseOwner { action } => Ok(action),
        }
    }
}

#[derive(Debug)]
struct ClaimCommentSuccess {
    marker: String,
    comment: GithubCommentRef,
    binding_ref: GithubProviderBindingRef,
    echo_suppressed: bool,
}

#[derive(Debug)]
struct DraftPullRequestSuccess {
    marker: String,
    pull_request: GithubPullRequestRef,
    echo_suppressed: bool,
}

fn claim_comment_idempotency_key(
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "provider-action:claim-comment:{workflow_run_id}"
    ))
}

fn claim_comment_input_hash(run: &GithubIssueWorkflowRun, marker: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"claim_comment");
    hasher.update(run.workflow_run_id.as_str().as_bytes());
    hasher.update(run.issue_ref.owner.as_bytes());
    hasher.update(run.issue_ref.repo.as_bytes());
    hasher.update(run.issue_ref.number.to_string().as_bytes());
    hasher.update(marker.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn claim_comment_body(marker: &str) -> String {
    format!(
        "{marker}\nIronClaw is attempting this bug fix. A draft PR will be linked here when ready."
    )
}

fn draft_pr_idempotency_key(workflow_run_id: &GithubIssueWorkflowRunId) -> WorkflowIdempotencyKey {
    WorkflowIdempotencyKey::from_generated(format!(
        "provider-action:draft-pull-request:{workflow_run_id}"
    ))
}

fn draft_pr_input_hash(request: &RunDraftPullRequestProviderActionRequest, marker: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"draft_pull_request");
    hasher.update(request.run.workflow_run_id.as_str().as_bytes());
    hasher.update(request.run.issue_ref.owner.as_bytes());
    hasher.update(request.run.issue_ref.repo.as_bytes());
    hasher.update(request.title.as_bytes());
    hasher.update(request.body.as_bytes());
    hasher.update(request.head_branch.as_bytes());
    hasher.update(request.base_branch.as_bytes());
    hasher.update(request.head_sha.as_bytes());
    hasher.update(marker.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn draft_pr_body(marker: &str, body: &str) -> String {
    if body.contains(marker) {
        body.to_string()
    } else {
        format!("{marker}\n{body}")
    }
}

fn claim_comment_success_binding_ref(
    run: &GithubIssueWorkflowRun,
    marker: &str,
    comment: &GithubCommentRef,
) -> GithubProviderBindingRef {
    let mut binding_ref = claim_comment_binding_ref(&run.issue_ref, marker);
    binding_ref.provider_ref.provider_url = Some(comment.url.clone());
    binding_ref
}
