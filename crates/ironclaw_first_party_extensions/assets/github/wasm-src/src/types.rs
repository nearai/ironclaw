use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GitCommitIdentity {
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub(crate) enum GitHubAction {
    #[serde(rename = "get_repo")]
    GetRepo { owner: String, repo: String },
    #[serde(rename = "create_repo")]
    CreateRepo {
        name: String,
        description: Option<String>,
        private: Option<bool>,
        auto_init: Option<bool>,
        gitignore_template: Option<String>,
        license_template: Option<String>,
        org: Option<String>,
    },
    #[serde(rename = "list_issues")]
    ListIssues {
        owner: String,
        repo: String,
        state: Option<String>,
        labels: Option<Vec<String>>,
        assignee: Option<String>,
        milestone: Option<String>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_issue")]
    CreateIssue {
        owner: String,
        repo: String,
        title: String,
        body: Option<String>,
        labels: Option<Vec<String>>,
        assignees: Option<Vec<String>>,
    },
    #[serde(rename = "get_issue")]
    GetIssue {
        owner: String,
        repo: String,
        issue_number: u32,
    },
    #[serde(rename = "list_issue_comments")]
    ListIssueComments {
        owner: String,
        repo: String,
        issue_number: u32,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_issue_comment")]
    CreateIssueComment {
        owner: String,
        repo: String,
        issue_number: u32,
        body: String,
    },
    #[serde(rename = "list_pull_requests")]
    ListPullRequests {
        owner: String,
        repo: String,
        state: Option<String>,
        head: Option<String>,
        base: Option<String>,
        sort: Option<String>,
        direction: Option<String>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_pull_request")]
    CreatePullRequest {
        owner: String,
        repo: String,
        title: String,
        head: String,
        base: String,
        body: Option<String>,
        draft: Option<bool>,
    },
    #[serde(rename = "get_pull_request")]
    GetPullRequest {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
    },
    #[serde(rename = "get_pull_request_files")]
    GetPullRequestFiles {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_pr_review")]
    CreatePrReview {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        body: String,
        event: PrReviewEvent,
    },
    #[serde(rename = "list_pull_request_comments")]
    ListPullRequestComments {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "reply_pull_request_comment")]
    ReplyPullRequestComment {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        comment_id: u64,
        body: String,
    },
    #[serde(rename = "get_pull_request_reviews")]
    GetPullRequestReviews {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "get_combined_status")]
    GetCombinedStatus {
        owner: String,
        repo: String,
        r#ref: String,
    },
    #[serde(rename = "merge_pull_request")]
    MergePullRequest {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        commit_title: Option<String>,
        commit_message: Option<String>,
        merge_method: Option<MergeMethod>,
        sha: Option<String>,
    },
    #[serde(rename = "get_authenticated_user")]
    GetAuthenticatedUser {},
    #[serde(rename = "list_repos")]
    ListRepos {
        username: Option<String>,
        #[serde(rename = "type")]
        repo_type: Option<RepoListType>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "search_repositories")]
    SearchRepositories {
        query: String,
        page: Option<u32>,
        limit: Option<u32>,
        sort: Option<String>,
        order: Option<String>,
    },
    #[serde(rename = "search_code")]
    SearchCode {
        query: String,
        page: Option<u32>,
        limit: Option<u32>,
        sort: Option<String>,
        order: Option<String>,
    },
    #[serde(rename = "search_issues_pull_requests")]
    SearchIssuesPullRequests {
        query: Option<String>,
        repository: Option<String>,
        owner: Option<String>,
        repo: Option<String>,
        author: Option<String>,
        assignee: Option<String>,
        involves: Option<String>,
        state: Option<String>,
        #[serde(rename = "type")]
        issue_type: Option<String>,
        page: Option<u32>,
        limit: Option<u32>,
        sort: Option<String>,
        order: Option<String>,
    },
    #[serde(rename = "list_branches")]
    ListBranches {
        owner: String,
        repo: String,
        protected: Option<bool>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_branch")]
    CreateBranch {
        owner: String,
        repo: String,
        branch: String,
        from_ref: String,
    },
    #[serde(rename = "get_file_content")]
    GetFileContent {
        owner: String,
        repo: String,
        path: String,
        r#ref: Option<String>,
    },
    #[serde(rename = "create_or_update_file")]
    CreateOrUpdateFile {
        owner: String,
        repo: String,
        path: String,
        message: String,
        content: String,
        sha: Option<String>,
        branch: Option<String>,
        committer: Option<GitCommitIdentity>,
        author: Option<GitCommitIdentity>,
    },
    #[serde(rename = "delete_file")]
    DeleteFile {
        owner: String,
        repo: String,
        path: String,
        message: String,
        sha: String,
        branch: Option<String>,
        committer: Option<GitCommitIdentity>,
        author: Option<GitCommitIdentity>,
    },
    #[serde(rename = "list_releases")]
    ListReleases {
        owner: String,
        repo: String,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "create_release")]
    CreateRelease {
        owner: String,
        repo: String,
        tag_name: String,
        target_commitish: Option<String>,
        name: Option<String>,
        body: Option<String>,
        draft: Option<bool>,
        prerelease: Option<bool>,
        generate_release_notes: Option<bool>,
    },
    #[serde(rename = "trigger_workflow")]
    TriggerWorkflow {
        owner: String,
        repo: String,
        workflow_id: String,
        r#ref: String,
        inputs: Option<serde_json::Value>,
    },
    #[serde(rename = "get_workflow_runs")]
    GetWorkflowRuns {
        owner: String,
        repo: String,
        workflow_id: Option<String>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "fork_repo")]
    ForkRepo {
        owner: String,
        repo: String,
        organization: Option<String>,
        name: Option<String>,
        default_branch_only: Option<bool>,
    },
    #[serde(rename = "handle_webhook")]
    HandleWebhook { webhook: GitHubWebhookRequest },
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum PrReviewEvent {
    #[serde(rename = "APPROVE")]
    Approve,
    #[serde(rename = "REQUEST_CHANGES")]
    RequestChanges,
    #[serde(rename = "COMMENT")]
    Comment,
}

impl PrReviewEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Comment => "COMMENT",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum MergeMethod {
    #[serde(rename = "merge")]
    Merge,
    #[serde(rename = "squash")]
    Squash,
    #[serde(rename = "rebase")]
    Rebase,
}

impl MergeMethod {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum RepoListType {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "owner")]
    Owner,
    #[serde(rename = "public")]
    Public,
    #[serde(rename = "private")]
    Private,
    #[serde(rename = "member")]
    Member,
}

impl RepoListType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Owner => "owner",
            Self::Public => "public",
            Self::Private => "private",
            Self::Member => "member",
        }
    }

    pub(crate) fn is_supported_for_named_user(self) -> bool {
        matches!(self, Self::All | Self::Owner | Self::Member)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubWebhookRequest {
    #[serde(default)]
    pub(crate) headers: HashMap<String, String>,
    #[serde(default)]
    pub(crate) body_json: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolWebhookResponse {
    pub(crate) accepted: bool,
    pub(crate) emit_events: Vec<SystemEventIntent>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SystemEventIntent {
    pub(crate) source: String,
    pub(crate) event_type: String,
    pub(crate) payload: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolContext {
    pub(crate) capability_id: String,
}
