use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GithubIssueProviderActionId, GithubIssueProviderBindingId, GithubIssueWorkflowRunId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueProviderBinding {
    pub binding_id: GithubIssueProviderBindingId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub system: String,
    pub resource_type: String,
    pub role: String,
    pub owner: String,
    pub repo: String,
    pub provider_id: String,
    pub provider_url: Option<String>,
    pub created_by_provider_action_id: Option<GithubIssueProviderActionId>,
    pub created_at: DateTime<Utc>,
}
