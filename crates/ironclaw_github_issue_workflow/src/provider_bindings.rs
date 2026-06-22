use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    GithubIssueProviderActionId, GithubIssueProviderBindingId, GithubIssueRef,
    GithubIssueWorkflowRunId, GithubProviderRef, GithubPullRequestRef,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubProviderBindingRef {
    pub provider_ref: GithubProviderRef,
    pub role: String,
}

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

pub fn issue_binding_ref(issue: &GithubIssueRef) -> GithubProviderBindingRef {
    GithubProviderBindingRef {
        provider_ref: GithubProviderRef {
            system: "github".to_string(),
            resource_type: "issue".to_string(),
            owner: issue.owner.clone(),
            repo: issue.repo.clone(),
            provider_id: issue_provider_id(issue),
            provider_url: Some(issue.url.clone()),
        },
        role: "primary".to_string(),
    }
}

pub fn claim_comment_binding_ref(issue: &GithubIssueRef, marker: &str) -> GithubProviderBindingRef {
    GithubProviderBindingRef {
        provider_ref: GithubProviderRef {
            system: "github".to_string(),
            resource_type: "issue_comment".to_string(),
            owner: issue.owner.clone(),
            repo: issue.repo.clone(),
            provider_id: marker.to_string(),
            provider_url: None,
        },
        role: "claim".to_string(),
    }
}

pub fn primary_pr_binding_ref(pr: &GithubPullRequestRef) -> GithubProviderBindingRef {
    GithubProviderBindingRef {
        provider_ref: GithubProviderRef {
            system: "github".to_string(),
            resource_type: "pull_request".to_string(),
            owner: pr.owner.clone(),
            repo: pr.repo.clone(),
            provider_id: pr_provider_id(pr),
            provider_url: Some(pr.url.clone()),
        },
        role: "primary".to_string(),
    }
}

fn issue_provider_id(issue: &GithubIssueRef) -> String {
    issue
        .node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}#{}", issue.owner, issue.repo, issue.number))
}

fn pr_provider_id(pr: &GithubPullRequestRef) -> String {
    pr.node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}#{}", pr.owner, pr.repo, pr.number))
}
