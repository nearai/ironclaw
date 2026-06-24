//! [`GithubIssueWorkflowPort`] implementation backed by a capability dispatcher.
//!
//! [`IronClawGithubIssueWorkflowPort`] adapts each typed workflow port method
//! into a GitHub capability dispatch and normalizes the provider response back
//! into the workflow's strongly-typed snapshots, mapping dispatch failures onto
//! [`GithubIssueWorkflowError`].

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_github_issue_workflow::{
    CreateDraftPullRequestInput, CreateIssueCommentInput, GetAuthenticatedWorkflowActorInput,
    GetGithubIssueInput, GetPullRequestInput, GithubActorSnapshot, GithubCommentRef,
    GithubIssueCommentSnapshot, GithubIssueProviderSnapshot, GithubIssueSearchHit,
    GithubIssueWorkflowError, GithubIssueWorkflowPort, GithubProviderAccountRef,
    GithubPullRequestCheckSnapshot, GithubPullRequestRef, GithubPullRequestSnapshot,
    GithubReviewCommentSnapshot, ListIssueCommentsInput, ListPullRequestChecksInput,
    ListPullRequestReviewCommentsInput, ListPullRequestsInput, SearchGithubIssuesInput,
};
use ironclaw_host_runtime::RuntimeFailureKind;
use serde_json::{Value as JsonValue, json};

use super::capability_dispatcher::{
    GithubIssueWorkflowCapabilityDispatchError, GithubIssueWorkflowCapabilityDispatchRequest,
    GithubIssueWorkflowCapabilityDispatcher,
};
use super::normalize::{
    normalize_actor_snapshot, normalize_combined_status_checks, normalize_comment_ref,
    normalize_issue_comments, normalize_issue_search_hits, normalize_issue_snapshot,
    normalize_pull_request_ref, normalize_pull_request_snapshot, normalize_pull_request_snapshots,
    normalize_review_comments,
};
use super::{
    GITHUB_COMMENT_ISSUE_CAPABILITY_ID, GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
    GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID, GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
    GITHUB_GET_ISSUE_CAPABILITY_ID, GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID, GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
    GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID, GITHUB_SEARCH_ISSUES_CAPABILITY_ID,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IronClawGithubIssueWorkflowPort<D> {
    dispatcher: Arc<D>,
}

impl<D> IronClawGithubIssueWorkflowPort<D> {
    pub(crate) fn new(dispatcher: Arc<D>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl<D> GithubIssueWorkflowPort for IronClawGithubIssueWorkflowPort<D>
where
    D: GithubIssueWorkflowCapabilityDispatcher,
{
    async fn search_open_bug_issues(
        &self,
        input: SearchGithubIssuesInput,
    ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_SEARCH_ISSUES_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "query": input.query,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_issue_search_hits(&response, &input.owner, &input.repo)
    }

    async fn get_issue(
        &self,
        input: GetGithubIssueInput,
    ) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_ISSUE_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "issue_number": input.number,
                }),
            )
            .await?;
        normalize_issue_snapshot(&response, &input.owner, &input.repo, input.number)
    }

    async fn get_authenticated_workflow_actor(
        &self,
        input: GetAuthenticatedWorkflowActorInput,
    ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({}),
            )
            .await?;
        normalize_actor_snapshot(&response)
    }

    async fn list_issue_comments(
        &self,
        input: ListIssueCommentsInput,
    ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.issue.owner,
                    "repo": input.issue.repo,
                    "issue_number": input.issue.number,
                }),
            )
            .await?;
        normalize_issue_comments(&response, &input.issue)
    }

    async fn create_issue_comment(
        &self,
        input: CreateIssueCommentInput,
    ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_COMMENT_ISSUE_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.issue.owner,
                    "repo": input.issue.repo,
                    "issue_number": input.issue.number,
                    "body": input.body,
                }),
            )
            .await?;
        normalize_comment_ref(
            &response,
            Some(&input.issue),
            GITHUB_COMMENT_ISSUE_CAPABILITY_ID,
        )
    }

    async fn list_pull_requests(
        &self,
        input: ListPullRequestsInput,
    ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner.clone(),
                    "repo": input.repo.clone(),
                    "state": input.state.clone(),
                    "page": 1,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_pull_request_snapshots(&response, &input.owner, &input.repo)
    }

    async fn create_draft_pull_request(
        &self,
        input: CreateDraftPullRequestInput,
    ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "title": input.title,
                    "head": input.head_branch,
                    "base": input.base_branch,
                    "body": input.body,
                    "draft": true,
                }),
            )
            .await?;
        normalize_pull_request_ref(&response, &input.owner, &input.repo)
    }

    async fn get_pull_request(
        &self,
        input: GetPullRequestInput,
    ) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner.clone(),
                    "repo": input.repo.clone(),
                    "pr_number": input.number,
                }),
            )
            .await?;
        normalize_pull_request_snapshot(
            &response,
            &input.owner,
            &input.repo,
            GITHUB_GET_PULL_REQUEST_CAPABILITY_ID,
        )
    }

    async fn list_pull_request_checks(
        &self,
        input: ListPullRequestChecksInput,
    ) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
        let head_ref =
            input
                .head_sha
                .clone()
                .ok_or_else(|| GithubIssueWorkflowError::ProviderRead {
                    reason: format!(
                        "GitHub pull request {}/{}#{} has no head SHA for status lookup",
                        input.owner, input.repo, input.pull_request_number
                    ),
                })?;
        let response = self
            .dispatch_capability(
                GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "ref": head_ref,
                }),
            )
            .await?;
        normalize_combined_status_checks(&response, input.head_sha.as_deref(), input.limit)
    }

    async fn list_pull_request_review_comments(
        &self,
        input: ListPullRequestReviewCommentsInput,
    ) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
        let response = self
            .dispatch_capability(
                GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                input.provider_account_ref.clone(),
                json!({
                    "owner": input.owner,
                    "repo": input.repo,
                    "pr_number": input.pull_request_number,
                    "page": 1,
                    "limit": input.limit,
                }),
            )
            .await?;
        normalize_review_comments(&response)
    }
}

impl<D> IronClawGithubIssueWorkflowPort<D>
where
    D: GithubIssueWorkflowCapabilityDispatcher,
{
    async fn dispatch_capability(
        &self,
        capability_id: &str,
        provider_account_ref: GithubProviderAccountRef,
        input: JsonValue,
    ) -> Result<JsonValue, GithubIssueWorkflowError> {
        self.dispatcher
            .dispatch(GithubIssueWorkflowCapabilityDispatchRequest {
                capability_id: capability_id.to_string(),
                provider_account_ref,
                input,
            })
            .await
            .map_err(|error| map_dispatch_error(capability_id, error))
    }
}

fn map_dispatch_error(
    capability_id: &str,
    error: GithubIssueWorkflowCapabilityDispatchError,
) -> GithubIssueWorkflowError {
    match error {
        GithubIssueWorkflowCapabilityDispatchError::AuthRequired => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: format!("GitHub capability {capability_id} requires authentication"),
            }
        }
        GithubIssueWorkflowCapabilityDispatchError::ApprovalRequired => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: format!("GitHub capability {capability_id} requires approval"),
            }
        }
        GithubIssueWorkflowCapabilityDispatchError::Backend { kind, .. } => {
            if kind == RuntimeFailureKind::Transient.as_str()
                || kind == RuntimeFailureKind::Resource.as_str()
            {
                GithubIssueWorkflowError::ProviderRateLimited {
                    reason: format!("GitHub capability {capability_id} failed ({kind})"),
                }
            } else {
                GithubIssueWorkflowError::ProviderRead {
                    reason: format!("GitHub capability {capability_id} failed ({kind})"),
                }
            }
        }
    }
}
