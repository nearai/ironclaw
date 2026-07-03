use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

fn deserialize_nullable_u32<'de, D>(deserializer: D) -> Result<Option<Option<u32>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u32>::deserialize(deserializer).map(Some)
}

fn deserialize_nullable_string<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

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
        milestone: Option<u32>,
        labels: Option<Vec<String>>,
        assignees: Option<Vec<String>>,
    },
    #[serde(rename = "update_issue")]
    UpdateIssue {
        owner: String,
        repo: String,
        issue_number: u32,
        title: Option<String>,
        #[serde(default, deserialize_with = "deserialize_nullable_string")]
        body: Option<Option<String>>,
        state: Option<IssueState>,
        #[serde(default, deserialize_with = "deserialize_nullable_u32")]
        milestone: Option<Option<u32>>,
        labels: Option<Vec<String>>,
        assignees: Option<Vec<String>>,
    },
    #[serde(rename = "add_issue_labels")]
    AddIssueLabels {
        owner: String,
        repo: String,
        issue_number: u32,
        labels: Vec<String>,
    },
    #[serde(rename = "remove_issue_label")]
    RemoveIssueLabel {
        owner: String,
        repo: String,
        issue_number: u32,
        name: String,
    },
    #[serde(rename = "add_issue_assignees")]
    AddIssueAssignees {
        owner: String,
        repo: String,
        issue_number: u32,
        assignees: Vec<String>,
    },
    #[serde(rename = "remove_issue_assignees")]
    RemoveIssueAssignees {
        owner: String,
        repo: String,
        issue_number: u32,
        assignees: Vec<String>,
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
        title: Option<String>,
        head: String,
        base: String,
        body: Option<String>,
        issue: Option<u32>,
        head_repo: Option<String>,
        maintainer_can_modify: Option<bool>,
        draft: Option<bool>,
    },
    #[serde(rename = "update_pull_request")]
    UpdatePullRequest {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        title: Option<String>,
        body: Option<String>,
        state: Option<PullRequestState>,
        base: Option<String>,
        maintainer_can_modify: Option<bool>,
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
        commit_id: Option<String>,
        comments: Option<Vec<PrReviewCommentInput>>,
    },
    #[serde(rename = "list_pull_request_comments")]
    ListPullRequestComments {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        sort: Option<PullRequestCommentSort>,
        direction: Option<Direction>,
        since: Option<String>,
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
    #[serde(rename = "list_pull_request_review_threads")]
    ListPullRequestReviewThreads {
        owner: String,
        repo: String,
        #[serde(alias = "number", alias = "pull_number")]
        pr_number: u32,
        first: Option<u32>,
        after: Option<String>,
    },
    #[serde(rename = "resolve_review_thread")]
    ResolveReviewThread { thread_id: String },
    #[serde(rename = "unresolve_review_thread")]
    UnresolveReviewThread { thread_id: String },
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
        actor: Option<String>,
        branch: Option<String>,
        event: Option<String>,
        status: Option<WorkflowRunStatus>,
        created: Option<String>,
        exclude_pull_requests: Option<bool>,
        check_suite_id: Option<u64>,
        head_sha: Option<String>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "get_workflow_run_jobs")]
    GetWorkflowRunJobs {
        owner: String,
        repo: String,
        run_id: u64,
        filter: Option<WorkflowJobFilter>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "get_workflow_run_artifacts")]
    GetWorkflowRunArtifacts {
        owner: String,
        repo: String,
        run_id: u64,
        name: Option<String>,
        direction: Option<Direction>,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "rerun_failed_workflow_run_jobs")]
    RerunFailedWorkflowRunJobs {
        owner: String,
        repo: String,
        run_id: u64,
        enable_debug_logging: Option<bool>,
    },
    #[serde(rename = "rerun_workflow_job")]
    RerunWorkflowJob {
        owner: String,
        repo: String,
        job_id: u64,
        enable_debug_logging: Option<bool>,
        enable_debugger: Option<bool>,
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
pub(crate) enum IssueState {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "closed")]
    Closed,
}

impl IssueState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum PullRequestState {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "closed")]
    Closed,
}

impl PullRequestState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum ReviewCommentSide {
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PrReviewCommentInput {
    pub(crate) path: String,
    pub(crate) body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) position: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) side: Option<ReviewCommentSide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) start_side: Option<ReviewCommentSide>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum PullRequestCommentSort {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "updated")]
    Updated,
}

impl PullRequestCommentSort {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum Direction {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

impl Direction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum WorkflowRunStatus {
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "action_required")]
    ActionRequired,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "failure")]
    Failure,
    #[serde(rename = "neutral")]
    Neutral,
    #[serde(rename = "skipped")]
    Skipped,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "timed_out")]
    TimedOut,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "requested")]
    Requested,
    #[serde(rename = "waiting")]
    Waiting,
    #[serde(rename = "pending")]
    Pending,
}

impl WorkflowRunStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::ActionRequired => "action_required",
            Self::Cancelled => "cancelled",
            Self::Failure => "failure",
            Self::Neutral => "neutral",
            Self::Skipped => "skipped",
            Self::Stale => "stale",
            Self::Success => "success",
            Self::TimedOut => "timed_out",
            Self::InProgress => "in_progress",
            Self::Queued => "queued",
            Self::Requested => "requested",
            Self::Waiting => "waiting",
            Self::Pending => "pending",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum WorkflowJobFilter {
    #[serde(rename = "latest")]
    Latest,
    #[serde(rename = "all")]
    All,
}

impl WorkflowJobFilter {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::All => "all",
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
