use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

use crate::{
    GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowRunId, WorkflowIdempotencyKey,
    WorkflowWorkspaceRef,
};
use ironclaw_turns::TurnRunId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueStageRun {
    pub stage_run_id: GithubIssueStageRunId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage: GithubIssueStage,
    pub status: StageRunStatus,
    pub stage_turn_identity: StageTurnIdentity,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub prompt_ref: WorkflowPromptContentRef,
    pub capability_profile_id: String,
    pub capability_profile_version: String,
    pub workspace_mount_ref: Option<WorkflowWorkspaceMountRef>,
    pub input_snapshot_hash: String,
    pub result: Option<JsonValue>,
    pub error: Option<JsonValue>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageRunStatus {
    Queued,
    Submitting,
    Running,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowActorScope {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: GithubIssueWorkflowRunId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTurnIdentity {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub stage: GithubIssueStage,
    pub attempt: u32,
    pub workflow_policy_version: String,
}

impl StageTurnIdentity {
    pub fn new(
        workflow_run_id: GithubIssueWorkflowRunId,
        stage_run_id: GithubIssueStageRunId,
        stage: GithubIssueStage,
        attempt: u32,
        workflow_policy_version: String,
    ) -> Self {
        Self {
            workflow_run_id,
            stage_run_id,
            stage,
            attempt,
            workflow_policy_version,
        }
    }

    pub fn thread_id_seed(&self) -> String {
        format!(
            "github-issue-workflow:{}:stage:{}",
            self.workflow_run_id, self.stage_run_id
        )
    }

    pub fn source_binding_ref(&self) -> String {
        format!(
            "github-issue-workflow:{}:stage:{}:source:{}",
            self.workflow_run_id,
            self.stage_run_id,
            stage_slug(&self.stage)
        )
    }

    pub fn reply_target_binding_ref(&self) -> String {
        format!(
            "github-issue-workflow:{}:stage:{}:reply:{}",
            self.workflow_run_id,
            self.stage_run_id,
            stage_slug(&self.stage)
        )
    }

    pub fn turn_idempotency_key(&self) -> WorkflowIdempotencyKey {
        WorkflowIdempotencyKey::from_generated(format!(
            "stage-turn:{}:{}:{}:{}:{}",
            self.workflow_policy_version,
            self.workflow_run_id,
            self.stage_run_id,
            stage_slug(&self.stage),
            self.attempt
        ))
    }

    pub fn completion_nonce(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.workflow_policy_version.as_bytes());
        hasher.update(self.workflow_run_id.as_str().as_bytes());
        hasher.update(self.stage_run_id.as_str().as_bytes());
        hasher.update(stage_slug(&self.stage).as_bytes());
        hasher.update(self.attempt.to_string().as_bytes());
        format!("stage-completion:{:x}", hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPromptContentRef {
    pub prompt_ref: String,
    pub prompt_version: String,
    pub input_snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPromptContent {
    pub content_ref: WorkflowPromptContentRef,
    pub content: String,
    pub content_hash: String,
}

impl WorkflowPromptContent {
    pub fn new(
        content_ref: WorkflowPromptContentRef,
        content: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Self {
        Self {
            content_ref,
            content: content.into(),
            content_hash: content_hash.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWorkspaceMountRef {
    pub mount_id: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitStageTurnRequest {
    pub stage_turn_identity: StageTurnIdentity,
    pub scope: WorkflowActorScope,
    pub prompt: WorkflowPromptContent,
    pub capability_profile_id: String,
    pub workspace_mount_ref: Option<WorkflowWorkspaceMountRef>,
    pub idempotency_key: WorkflowIdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmitStageTurnOutcome {
    Submitted {
        thread_id: ThreadId,
        turn_run_id: TurnRunId,
    },
    Replayed {
        thread_id: ThreadId,
        turn_run_id: TurnRunId,
    },
    Busy {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedWorkflowStageWorkspace {
    pub workspace_ref: WorkflowWorkspaceRef,
    pub mount_ref: WorkflowWorkspaceMountRef,
}

pub(crate) fn stage_slug(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage",
        GithubIssueStage::Planning => "planning",
        GithubIssueStage::Implementation => "implementation",
        GithubIssueStage::PrSynthesis => "pr_synthesis",
        GithubIssueStage::CiRepair => "ci_repair",
        GithubIssueStage::ReviewResponse => "review_response",
    }
}

// Stage-result binding enforcement now lives in the composition sink
// (`GithubWorkflowStageResultSink`), which derives the authoritative stage
// identity from the trusted executing thread's metadata rather than comparing
// two model-supplied copies. The former `StageResultBinding`/
// `evaluate_stage_result_attempt` helpers compared an untrusted attempt against
// an untrusted binding and established no trust, so they were removed.
