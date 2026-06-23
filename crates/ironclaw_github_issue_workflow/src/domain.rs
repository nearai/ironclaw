use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};

use crate::{
    GithubIssueStageRunId, GithubIssueWorkflowRunId, GithubIssueWorkflowRunKey,
    GithubIssueWorkspaceSessionId, GithubProviderAccountRef, GithubRepositorySelector,
    WorkflowWorkerId, WorkflowWorkspaceMountRef,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWorkflowRunStatus {
    Active,
    Blocked,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWorkflowMode {
    New,
    Claimed,
    Triage,
    Planning,
    Implementation,
    PrSynthesis,
    PrOpen,
    CiRepair,
    ReviewResponse,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueStage {
    Triage,
    Planning,
    Implementation,
    PrSynthesis,
    CiRepair,
    ReviewResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPullRequestRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub head_branch: String,
    pub head_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubCommentRef {
    pub node_id: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWorkspaceRef {
    pub thread_id: Option<ThreadId>,
    pub workspace_session_id: Option<GithubIssueWorkspaceSessionId>,
    pub turn_run_id: Option<TurnRunId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkspaceSession {
    pub workspace_session_id: GithubIssueWorkspaceSessionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub repository: GithubRepositorySelector,
    pub base_branch: String,
    pub base_sha: Option<String>,
    pub working_branch: String,
    pub current_head_sha: Option<String>,
    pub workspace_ref: WorkflowWorkspaceRef,
    pub mount_ref: WorkflowWorkspaceMountRef,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueBlockKind {
    WaitingApproval,
    WaitingAuth,
    BlockedHuman,
    RecoveryRequired,
    RateLimited,
    TerminalFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueBlockState {
    pub kind: GithubIssueBlockKind,
    pub reason: String,
    pub blocked_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssuePlanItemStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssuePlanItem {
    pub title: String,
    pub status: GithubIssuePlanItemStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubProviderWatermarks {
    pub issue_updated_at: Option<Timestamp>,
    pub pull_request_updated_at: Option<Timestamp>,
    pub checks_updated_at: Option<Timestamp>,
    pub reviews_updated_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContentSummary {
    pub source_ref: String,
    pub author: Option<String>,
    pub summary: String,
    pub trust: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueProviderSnapshotSummary {
    pub title: String,
    pub state: String,
    pub author_login: Option<String>,
    pub labels: Vec<String>,
    pub updated_at: Option<Timestamp>,
    pub comment_count: usize,
    pub body_present: bool,
    #[serde(default)]
    pub content_summaries: Vec<ProviderContentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowState {
    pub mode: GithubIssueWorkflowMode,
    pub active_block: Option<GithubIssueBlockState>,
    pub plan: Vec<GithubIssuePlanItem>,
    pub primary_pr: Option<GithubPullRequestRef>,
    pub claim_comment: Option<GithubCommentRef>,
    #[serde(default)]
    pub current_workspace_ref: Option<WorkflowWorkspaceRef>,
    #[serde(default)]
    pub current_workspace_mount_ref: Option<WorkflowWorkspaceMountRef>,
    #[serde(default)]
    pub latest_provider_snapshot: Option<GithubIssueProviderSnapshotSummary>,
    pub last_provider_watermarks: GithubProviderWatermarks,
}

impl GithubIssueWorkflowState {
    pub fn new(mode: GithubIssueWorkflowMode) -> Self {
        Self {
            mode,
            active_block: None,
            plan: Vec::new(),
            primary_pr: None,
            claim_comment: None,
            current_workspace_ref: None,
            current_workspace_mount_ref: None,
            latest_provider_snapshot: None,
            last_provider_watermarks: GithubProviderWatermarks::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowRun {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub workflow_run_key: GithubIssueWorkflowRunKey,
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub provider_account_ref: Option<GithubProviderAccountRef>,
    pub issue_ref: GithubIssueRef,
    pub workflow_policy_key: String,
    pub workflow_policy_version: String,
    pub status: GithubIssueWorkflowRunStatus,
    pub workflow_state: GithubIssueWorkflowState,
    pub event_cursor: i64,
    pub workflow_run_version: i64,
    pub lease_owner: Option<WorkflowWorkerId>,
    pub lease_expires_at: Option<Timestamp>,
    pub last_heartbeat_at: Option<Timestamp>,
    pub claim_count: u32,
    pub active_stage_run_id: Option<GithubIssueStageRunId>,
    pub workspace_session_id: Option<GithubIssueWorkspaceSessionId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
