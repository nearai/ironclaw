use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};

use crate::{
    GithubCommentRef, GithubIssueRef, GithubIssueWorkflowConfig, GithubIssueWorkflowError,
    GithubIssueWorkflowRunId, GithubIssueWorkspaceSessionId, GithubProviderAccountRef,
    GithubPullRequestRef, GithubRepositorySelector, SubmitStageTurnOutcome, SubmitStageTurnRequest,
    WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
};

#[async_trait]
pub trait WorkflowClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[async_trait]
pub trait GithubIssueWorkflowConfigSource: Send + Sync {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError>;
}

#[async_trait]
pub trait GithubIssueWorkflowPort: Send + Sync {
    async fn search_open_bug_issues(
        &self,
        _input: SearchGithubIssuesInput,
    ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::ProviderRead {
            reason: "GitHub issue search is not configured".to_string(),
        })
    }

    async fn get_issue(
        &self,
        _input: GetGithubIssueInput,
    ) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::ProviderRead {
            reason: "GitHub issue reads are not configured".to_string(),
        })
    }

    async fn get_authenticated_workflow_actor(
        &self,
        input: GetAuthenticatedWorkflowActorInput,
    ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError>;

    async fn list_issue_comments(
        &self,
        input: ListIssueCommentsInput,
    ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError>;

    async fn create_issue_comment(
        &self,
        input: CreateIssueCommentInput,
    ) -> Result<GithubCommentRef, GithubIssueWorkflowError>;

    async fn create_draft_pull_request(
        &self,
        _input: CreateDraftPullRequestInput,
    ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::ProviderRead {
            reason: "GitHub draft pull request writes are not configured".to_string(),
        })
    }
}

#[async_trait]
pub trait StageTurnSubmitter: Send + Sync {
    async fn submit_stage_turn(
        &self,
        request: SubmitStageTurnRequest,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError>;
}

#[async_trait]
pub trait WorkflowProjectAccess: Send + Sync {
    async fn assert_workflow_config_access(
        &self,
        _request: WorkflowConfigAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "workflow config access is not configured".to_string(),
        })
    }

    async fn assert_workflow_project_access(
        &self,
        request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError>;
}

#[async_trait]
pub trait WorkflowWorkspaceManager: Send + Sync {
    async fn prepare_workspace(
        &self,
        request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetAuthenticatedWorkflowActorInput {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubActorSnapshot {
    pub login: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchGithubIssuesInput {
    pub provider_account_ref: GithubProviderAccountRef,
    pub owner: String,
    pub repo: String,
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueSearchHit {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub default_branch: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl GithubIssueSearchHit {
    pub fn issue_ref(&self) -> GithubIssueRef {
        GithubIssueRef {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            number: self.number,
            node_id: self.node_id.clone(),
            url: self.url.clone(),
            default_branch: self.default_branch.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetGithubIssueInput {
    pub provider_account_ref: GithubProviderAccountRef,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueProviderSnapshot {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub default_branch: String,
    pub title: String,
    pub body: String,
    pub state: String,
    pub labels: Vec<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl GithubIssueProviderSnapshot {
    pub fn issue_ref(&self) -> GithubIssueRef {
        GithubIssueRef {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            number: self.number,
            node_id: self.node_id.clone(),
            url: self.url.clone(),
            default_branch: self.default_branch.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListIssueCommentsInput {
    pub issue: GithubIssueRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueCommentSnapshot {
    pub comment: GithubCommentRef,
    pub body: String,
    pub author_login: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateIssueCommentInput {
    pub issue: GithubIssueRef,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateDraftPullRequestInput {
    pub provider_account_ref: GithubProviderAccountRef,
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: Option<String>,
    pub head_branch: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConfigAccessRequest {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub project_id: ProjectId,
    pub repositories: Vec<GithubRepositorySelector>,
    pub provider_account_ref: GithubProviderAccountRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowProjectAccessRequest {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub issue: GithubIssueRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareWorkflowWorkspaceRequest {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub issue: GithubIssueRef,
    pub base_branch: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareWorkflowWorkspaceOutcome {
    pub workspace_session_id: GithubIssueWorkspaceSessionId,
    pub workspace_ref: WorkflowWorkspaceRef,
    pub mount_ref: WorkflowWorkspaceMountRef,
}
