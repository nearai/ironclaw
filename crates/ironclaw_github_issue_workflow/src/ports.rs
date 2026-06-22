use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};

use crate::{
    GithubCommentRef, GithubIssueRef, GithubIssueWorkflowError, GithubIssueWorkflowRunId,
    GithubIssueWorkspaceSessionId, SubmitStageTurnOutcome, SubmitStageTurnRequest,
    WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
};

#[async_trait]
pub trait WorkflowClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[async_trait]
pub trait GithubIssueWorkflowPort: Send + Sync {
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
