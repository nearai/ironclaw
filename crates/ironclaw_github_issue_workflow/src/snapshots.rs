use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{GithubIssueStage, GithubIssueWorkflowError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeredWorkflowSnapshot {
    pub issue: GithubIssueSnapshot,
    pub workflow: WorkflowStateSnapshot,
    pub repository: RepositorySnapshot,
    pub previous_stage_results: Vec<StageResultSummary>,
    pub workspace: Option<WorkflowWorkspaceSnapshot>,
    pub constraints: StageConstraintSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueSnapshot {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub default_branch: String,
    pub state: String,
    pub labels: Vec<String>,
    pub summary: String,
    pub provider_content_summaries: Vec<ProviderContentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContentSummary {
    pub source_ref: String,
    pub author: Option<String>,
    pub summary: String,
    pub trust: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStateSnapshot {
    pub workflow_run_id: String,
    pub workflow_policy_key: String,
    pub workflow_policy_version: String,
    pub status: String,
    pub mode: String,
    pub active_stage_run_id: Option<String>,
    pub event_cursor: i64,
    pub workflow_run_version: i64,
    pub active_block_summary: Option<String>,
    pub plan: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub base_ref: Option<String>,
    pub base_sha: Option<String>,
    pub working_branch: Option<String>,
    pub head_sha: Option<String>,
    pub primary_pr_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageResultSummary {
    pub stage: GithubIssueStage,
    pub outcome: String,
    pub summary: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWorkspaceSnapshot {
    pub workspace_session_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_run_id: Option<String>,
    pub mount_alias: Option<String>,
    pub virtual_root: String,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageConstraintSnapshot {
    pub stage: GithubIssueStage,
    pub stage_goal: String,
    pub allowed_capabilities: Vec<String>,
    pub disallowed_capabilities: Vec<String>,
    pub result_schema_version: String,
    pub completion_tool: String,
    pub provider_write_policy: String,
}

pub fn snapshot_hash(
    snapshot: &EngineeredWorkflowSnapshot,
) -> Result<String, GithubIssueWorkflowError> {
    let bytes = serde_json::to_vec(snapshot).map_err(snapshot_serde_error)?;
    Ok(sha256_hex_bytes(&bytes))
}

pub(crate) fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn snapshot_serde_error(error: serde_json::Error) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("failed to serialize engineered workflow snapshot: {error}"),
    }
}
