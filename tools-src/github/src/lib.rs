//! GitHub WASM Tool for IronClaw.
//!
//! Provides GitHub integration for reading repos, managing issues,
//! reviewing PRs, and triggering workflows.
//!
//! # Authentication
//!
//! Store your GitHub Personal Access Token:
//! `ironclaw secret set github_token <token>`
//!
//! Token needs these permissions:
//! - repo (for private repos)
//! - workflow (for triggering actions)
//! - read:org (for org repos)

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use std::collections::HashMap;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

const MAX_TEXT_LENGTH: usize = 65536;

/// Validate input length to prevent oversized payloads.
fn validate_input_length(s: &str, field_name: &str) -> Result<(), String> {
    if s.len() > MAX_TEXT_LENGTH {
        return Err(format!(
            "Input '{}' exceeds maximum length of {} characters",
            field_name, MAX_TEXT_LENGTH
        ));
    }
    Ok(())
}

/// Percent-encode a string for safe use in URL path segments.
/// Encodes everything except alphanumeric, hyphen, underscore, and dot.
fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

/// Percent-encode a string for use as a URL query parameter value.
/// Currently identical to `url_encode_path`.
fn url_encode_query(s: &str) -> String {
    url_encode_path(s)
}

/// Validate that a path segment doesn't contain dangerous characters.
/// Returns true if the segment is safe to use.
fn validate_path_segment(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains("..")
        && !s.contains('?')
        && !s.contains('#')
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn validate_repo_path(path: &str) -> Result<(), String> {
    validate_input_length(path, "path")?;
    for segment in path.split('/') {
        if segment == ".." {
            return Err("Invalid path: path traversal not allowed".into());
        }
        if segment.is_empty() {
            return Err("Invalid path: empty segment not allowed".into());
        }
    }
    Ok(())
}

fn encode_repo_path(path: &str) -> String {
    path.split('/')
        .map(url_encode_path)
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_git_ref(ref_name: &str, field_name: &str) -> Result<(), String> {
    if ref_name.is_empty() {
        return Err(format!("Invalid {field_name}: cannot be empty"));
    }
    if ref_name.contains("..")
        || ref_name.contains(':')
        || ref_name.contains('?')
        || ref_name.contains('[')
        || ref_name.contains('\\')
        || ref_name.contains('^')
        || ref_name.contains('~')
        || ref_name.contains("@{")
        || ref_name.contains("//")
        || ref_name.starts_with('/')
        || ref_name.ends_with('/')
        || ref_name.starts_with('.')
        || ref_name.ends_with('.')
        || ref_name.ends_with(".lock")
        || ref_name.chars().any(|c| c.is_control() || c == ' ')
    {
        return Err(format!(
            "Invalid {field_name}: must be a valid branch, tag, or ref name"
        ));
    }
    Ok(())
}

fn normalize_ref_lookup(ref_name: &str) -> Result<String, String> {
    validate_git_ref(ref_name, "from_ref")?;
    if let Some(stripped) = ref_name.strip_prefix("refs/heads/") {
        return Ok(format!("heads/{stripped}"));
    }
    if let Some(stripped) = ref_name.strip_prefix("refs/tags/") {
        return Ok(format!("tags/{stripped}"));
    }
    if ref_name.starts_with("refs/") {
        return Err(
            "Unsupported from_ref: only refs/heads/* and refs/tags/* are supported".to_string(),
        );
    }
    if ref_name.starts_with("heads/") || ref_name.starts_with("tags/") {
        return Ok(ref_name.to_string());
    }
    Ok(format!("heads/{ref_name}"))
}

fn normalize_branch_ref(branch: &str) -> Result<String, String> {
    validate_git_ref(branch, "branch")?;
    if branch.starts_with("refs/heads/") {
        return Ok(branch.to_string());
    }
    if branch.starts_with("refs/") {
        return Err("Invalid branch ref: only refs/heads/* is allowed".to_string());
    }
    let branch = branch.strip_prefix("heads/").unwrap_or(branch);
    if branch.starts_with("tags/") {
        return Err("Invalid branch ref: tags/* is not a branch".to_string());
    }
    Ok(format!("refs/heads/{branch}"))
}

fn append_search_params(
    path: &mut String,
    page: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), String> {
    if let Some(p) = page {
        path.push_str(&format!("&page={p}"));
    }
    if let Some(sort) = sort {
        validate_input_length(sort, "sort")?;
        path.push_str("&sort=");
        path.push_str(&url_encode_query(sort));
    }
    if let Some(order) = order {
        if !matches!(order, "asc" | "desc") {
            return Err("Invalid order: must be 'asc' or 'desc'".into());
        }
        path.push_str("&order=");
        path.push_str(order);
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GitCommitIdentity {
    name: String,
    email: String,
}

fn validate_commit_identity(identity: &GitCommitIdentity, field_name: &str) -> Result<(), String> {
    validate_input_length(&identity.name, &format!("{field_name}.name"))?;
    validate_input_length(&identity.email, &format!("{field_name}.email"))?;
    Ok(())
}

struct GitHubTool;

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum GitHubAction {
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
        pr_number: u32,
    },
    #[serde(rename = "get_pull_request_files")]
    GetPullRequestFiles {
        owner: String,
        repo: String,
        pr_number: u32,
    },
    #[serde(rename = "create_pr_review")]
    CreatePrReview {
        owner: String,
        repo: String,
        pr_number: u32,
        body: String,
        event: String,
    },
    #[serde(rename = "list_pull_request_comments")]
    ListPullRequestComments {
        owner: String,
        repo: String,
        pr_number: u32,
        page: Option<u32>,
        limit: Option<u32>,
    },
    #[serde(rename = "reply_pull_request_comment")]
    ReplyPullRequestComment {
        owner: String,
        repo: String,
        comment_id: u64,
        body: String,
    },
    #[serde(rename = "get_pull_request_reviews")]
    GetPullRequestReviews {
        owner: String,
        repo: String,
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
        pr_number: u32,
        commit_title: Option<String>,
        commit_message: Option<String>,
        merge_method: Option<String>,
    },
    #[serde(rename = "list_repos")]
    ListRepos {
        username: String,
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
        query: String,
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

#[derive(Debug, Deserialize)]
struct GitHubWebhookRequest {
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body_json: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ToolWebhookResponse {
    accepted: bool,
    emit_events: Vec<SystemEventIntent>,
}

#[derive(Debug, Serialize)]
struct SystemEventIntent {
    source: String,
    event_type: String,
    payload: serde_json::Value,
}

impl exports::near::agent::tool::Guest for GitHubTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "GitHub integration for repositories, issues, pull requests, search, \
         branches, file reads and writes, releases, and workflows. \
         Authentication is handled via the 'github_token' secret injected by the host."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: GitHubAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    // Pre-flight check: ensure token exists in secret store.
    // We don't use the returned value because the host injects it into the request.
    let _ = get_github_token()?;

    match action {
        GitHubAction::GetRepo { owner, repo } => get_repo(&owner, &repo),
        GitHubAction::CreateRepo {
            name,
            description,
            private,
            auto_init,
            gitignore_template,
            license_template,
            org,
        } => create_repo(
            &name,
            description.as_deref(),
            private.unwrap_or(false),
            auto_init.unwrap_or(false),
            gitignore_template.as_deref(),
            license_template.as_deref(),
            org.as_deref(),
        ),
        GitHubAction::ListIssues {
            owner,
            repo,
            state,
            page,
            limit,
        } => list_issues(&owner, &repo, state.as_deref(), page, limit),
        GitHubAction::CreateIssue {
            owner,
            repo,
            title,
            body,
            labels,
        } => create_issue(&owner, &repo, &title, body.as_deref(), labels),
        GitHubAction::GetIssue {
            owner,
            repo,
            issue_number,
        } => get_issue(&owner, &repo, issue_number),
        GitHubAction::ListIssueComments {
            owner,
            repo,
            issue_number,
            page,
            limit,
        } => list_issue_comments(&owner, &repo, issue_number, page, limit),
        GitHubAction::CreateIssueComment {
            owner,
            repo,
            issue_number,
            body,
        } => create_issue_comment(&owner, &repo, issue_number, &body),
        GitHubAction::ListPullRequests {
            owner,
            repo,
            state,
            page,
            limit,
        } => list_pull_requests(&owner, &repo, state.as_deref(), page, limit),
        GitHubAction::CreatePullRequest {
            owner,
            repo,
            title,
            head,
            base,
            body,
            draft,
        } => create_pull_request(
            &owner,
            &repo,
            &title,
            &head,
            &base,
            body.as_deref(),
            draft.unwrap_or(false),
        ),
        GitHubAction::GetPullRequest {
            owner,
            repo,
            pr_number,
        } => get_pull_request(&owner, &repo, pr_number),
        GitHubAction::GetPullRequestFiles {
            owner,
            repo,
            pr_number,
        } => get_pull_request_files(&owner, &repo, pr_number),
        GitHubAction::CreatePrReview {
            owner,
            repo,
            pr_number,
            body,
            event,
        } => create_pr_review(&owner, &repo, pr_number, &body, &event),
        GitHubAction::ListPullRequestComments {
            owner,
            repo,
            pr_number,
            page,
            limit,
        } => list_pull_request_comments(&owner, &repo, pr_number, page, limit),
        GitHubAction::ReplyPullRequestComment {
            owner,
            repo,
            comment_id,
            body,
        } => reply_pull_request_comment(&owner, &repo, comment_id, &body),
        GitHubAction::GetPullRequestReviews {
            owner,
            repo,
            pr_number,
            page,
            limit,
        } => get_pull_request_reviews(&owner, &repo, pr_number, page, limit),
        GitHubAction::GetCombinedStatus { owner, repo, r#ref } => {
            get_combined_status(&owner, &repo, &r#ref)
        }
        GitHubAction::MergePullRequest {
            owner,
            repo,
            pr_number,
            commit_title,
            commit_message,
            merge_method,
        } => merge_pull_request(
            &owner,
            &repo,
            pr_number,
            commit_title.as_deref(),
            commit_message.as_deref(),
            merge_method.as_deref(),
        ),
        GitHubAction::ListRepos {
            username,
            page,
            limit,
        } => list_repos(&username, page, limit),
        GitHubAction::SearchRepositories {
            query,
            page,
            limit,
            sort,
            order,
        } => search_repositories(&query, page, limit, sort.as_deref(), order.as_deref()),
        GitHubAction::SearchCode {
            query,
            page,
            limit,
            sort,
            order,
        } => search_code(&query, page, limit, sort.as_deref(), order.as_deref()),
        GitHubAction::SearchIssuesPullRequests {
            query,
            page,
            limit,
            sort,
            order,
        } => search_issues_pull_requests(&query, page, limit, sort.as_deref(), order.as_deref()),
        GitHubAction::ListBranches {
            owner,
            repo,
            protected,
            page,
            limit,
        } => list_branches(&owner, &repo, protected, page, limit),
        GitHubAction::CreateBranch {
            owner,
            repo,
            branch,
            from_ref,
        } => create_branch(&owner, &repo, &branch, &from_ref),
        GitHubAction::GetFileContent {
            owner,
            repo,
            path,
            r#ref,
        } => get_file_content(&owner, &repo, &path, r#ref.as_deref()),
        GitHubAction::CreateOrUpdateFile {
            owner,
            repo,
            path,
            message,
            content,
            sha,
            branch,
            committer,
            author,
        } => create_or_update_file(
            &owner,
            &repo,
            &path,
            &message,
            &content,
            sha.as_deref(),
            branch.as_deref(),
            committer,
            author,
        ),
        GitHubAction::DeleteFile {
            owner,
            repo,
            path,
            message,
            sha,
            branch,
            committer,
            author,
        } => delete_file(
            &owner,
            &repo,
            &path,
            &message,
            &sha,
            branch.as_deref(),
            committer,
            author,
        ),
        GitHubAction::ListReleases {
            owner,
            repo,
            page,
            limit,
        } => list_releases(&owner, &repo, page, limit),
        GitHubAction::CreateRelease {
            owner,
            repo,
            tag_name,
            target_commitish,
            name,
            body,
            draft,
            prerelease,
            generate_release_notes,
        } => create_release(
            &owner,
            &repo,
            &tag_name,
            target_commitish.as_deref(),
            name.as_deref(),
            body.as_deref(),
            draft.unwrap_or(false),
            prerelease.unwrap_or(false),
            generate_release_notes.unwrap_or(false),
        ),
        GitHubAction::TriggerWorkflow {
            owner,
            repo,
            workflow_id,
            r#ref,
            inputs,
        } => trigger_workflow(&owner, &repo, &workflow_id, &r#ref, inputs),
        GitHubAction::GetWorkflowRuns {
            owner,
            repo,
            workflow_id,
            page,
            limit,
        } => get_workflow_runs(&owner, &repo, workflow_id.as_deref(), page, limit),
        GitHubAction::ForkRepo {
            owner,
            repo,
            organization,
            name,
            default_branch_only,
        } => fork_repo(
            &owner,
            &repo,
            organization.as_deref(),
            name.as_deref(),
            default_branch_only,
        ),
        GitHubAction::HandleWebhook { webhook } => handle_webhook(webhook),
    }
}

fn get_github_token() -> Result<String, String> {
    if near::agent::host::secret_exists("github_token") {
        // Return dummy value since we only need to verify existence.
        // The actual token is injected by the host.
        return Ok("present".to_string());
    }

    Err("GitHub token not found in secret store. Set it with: ironclaw secret set github_token <token>. \
         Token needs 'repo', 'workflow', and 'read:org' scopes.".into())
}

fn github_request(method: &str, path: &str, body: Option<String>) -> Result<String, String> {
    let url = format!("https://api.github.com{}", path);

    // Authorization header (Bearer <token>) is injected automatically by the host
    // via the `http-wrapper` proxy based on the `github_token` secret.
    let headers = serde_json::json!({
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2026-03-10",
        "User-Agent": "IronClaw-GitHub-Tool"
    });

    let body_bytes = body.map(|b| b.into_bytes());

    // Simple retry logic for transient errors (max 3 attempts)
    let max_retries = 3;
    let mut attempt = 0;

    loop {
        attempt += 1;

        let response = near::agent::host::http_request(
            method,
            &url,
            &headers.to_string(),
            body_bytes.as_deref(),
            None,
        );

        match response {
            Ok(resp) => {
                // Log warning if rate limit is low
                if let Ok(headers_json) =
                    serde_json::from_str::<serde_json::Value>(&resp.headers_json)
                {
                    // Header keys are often lowercase in http libs, check case-insensitively if needed,
                    // but usually standard is lowercase/case-insensitive. Let's try lowercase.
                    if let Some(remaining) = headers_json
                        .get("x-ratelimit-remaining")
                        .and_then(|v| v.as_str())
                    {
                        if let Ok(count) = remaining.parse::<u32>() {
                            if count < 10 {
                                near::agent::host::log(
                                    near::agent::host::LogLevel::Warn,
                                    &format!("GitHub API rate limit low: {} remaining", count),
                                );
                            }
                        }
                    }
                }

                if resp.status >= 200 && resp.status < 300 {
                    return String::from_utf8(resp.body)
                        .map_err(|e| format!("Invalid UTF-8: {}", e));
                } else if attempt < max_retries && (resp.status == 429 || resp.status >= 500) {
                    near::agent::host::log(
                        near::agent::host::LogLevel::Warn,
                        &format!(
                            "GitHub API error {} (attempt {}/{}). Retrying...",
                            resp.status, attempt, max_retries
                        ),
                    );
                    // Minimal backoff simulation since we can't block easily in WASM without consuming generic budget?
                    // actually std::thread::sleep works in WASMtime if configured, but here we might just spin.
                    // ideally host exposes sleep. For now just retry immediately or rely on host timeout logic?
                    // Let's assume immediate retry for now as simple strategy.
                    continue;
                } else {
                    let body_str = String::from_utf8_lossy(&resp.body);
                    return Err(format!("GitHub API error {}: {}", resp.status, body_str));
                }
            }
            Err(e) => {
                if attempt < max_retries {
                    near::agent::host::log(
                        near::agent::host::LogLevel::Warn,
                        &format!(
                            "HTTP request failed: {} (attempt {}/{}). Retrying...",
                            e, attempt, max_retries
                        ),
                    );
                    continue;
                }
                return Err(format!(
                    "HTTP request failed after {} attempts: {}",
                    max_retries, e
                ));
            }
        }
    }
}

// === API Functions ===

fn get_repo(owner: &str, repo: &str) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!("/repos/{}/{}", encoded_owner, encoded_repo),
        None,
    )
}

fn create_repo(
    name: &str,
    description: Option<&str>,
    private: bool,
    auto_init: bool,
    gitignore_template: Option<&str>,
    license_template: Option<&str>,
    org: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(name) {
        return Err("Invalid repository name".into());
    }
    validate_input_length(name, "name")?;
    if let Some(description) = description {
        validate_input_length(description, "description")?;
    }
    if let Some(template) = gitignore_template {
        validate_input_length(template, "gitignore_template")?;
    }
    if let Some(template) = license_template {
        validate_input_length(template, "license_template")?;
    }
    if let Some(org) = org {
        if !validate_path_segment(org) {
            return Err("Invalid org name".into());
        }
    }

    let path = if let Some(org) = org {
        format!("/orgs/{}/repos", url_encode_path(org))
    } else {
        "/user/repos".to_string()
    };

    let mut req_body = serde_json::json!({
        "name": name,
        "private": private,
        "auto_init": auto_init,
    });
    if let Some(description) = description {
        req_body["description"] = serde_json::json!(description);
    }
    if let Some(template) = gitignore_template {
        req_body["gitignore_template"] = serde_json::json!(template);
    }
    if let Some(template) = license_template {
        req_body["license_template"] = serde_json::json!(template);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

fn fork_repo(
    owner: &str,
    repo: &str,
    organization: Option<&str>,
    name: Option<&str>,
    default_branch_only: Option<bool>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if let Some(org) = organization {
        if !validate_path_segment(org) {
            return Err("Invalid organization name".into());
        }
    }
    if let Some(n) = name {
        validate_input_length(n, "name")?;
        if !validate_path_segment(n) {
            return Err("Invalid fork name".into());
        }
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/forks", encoded_owner, encoded_repo);

    let mut req_body = serde_json::json!({});
    if let Some(org) = organization {
        req_body["organization"] = serde_json::json!(org);
    }
    if let Some(n) = name {
        req_body["name"] = serde_json::json!(n);
    }
    if let Some(only) = default_branch_only {
        req_body["default_branch_only"] = serde_json::json!(only);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

fn list_issues(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let state = state.unwrap_or("open");
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let encoded_state = url_encode_query(state);

    let mut path = format!(
        "/repos/{}/{}/issues?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }

    github_request("GET", &path, None)
}

fn create_issue(
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<Vec<String>>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(title, "title")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/issues", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "title": title,
    });
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    if let Some(labels) = labels {
        req_body["labels"] = serde_json::json!(labels);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

fn get_issue(owner: &str, repo: &str, issue_number: u32) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/issues/{}",
            encoded_owner, encoded_repo, issue_number
        ),
        None,
    )
}

fn list_issue_comments(
    owner: &str,
    repo: &str,
    issue_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/issues/{}/comments?per_page={}",
        encoded_owner, encoded_repo, issue_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn create_issue_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/issues/{}/comments",
        encoded_owner, encoded_repo, issue_number
    );
    let req_body = serde_json::json!({ "body": body });
    github_request("POST", &path, Some(req_body.to_string()))
}

fn list_pull_requests(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let state = state.unwrap_or("open");
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let encoded_state = url_encode_query(state);

    let mut path = format!(
        "/repos/{}/{}/pulls?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }

    github_request("GET", &path, None)
}

fn create_pull_request(
    owner: &str,
    repo: &str,
    title: &str,
    head: &str,
    base: &str,
    body: Option<&str>,
    draft: bool,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(title, "title")?;
    validate_input_length(head, "head")?;
    validate_input_length(base, "base")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/pulls", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "title": title,
        "head": head,
        "base": base,
        "draft": draft,
    });
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

fn get_pull_request(owner: &str, repo: &str, pr_number: u32) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/pulls/{}",
            encoded_owner, encoded_repo, pr_number
        ),
        None,
    )
}

fn get_pull_request_files(owner: &str, repo: &str, pr_number: u32) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/pulls/{}/files",
            encoded_owner, encoded_repo, pr_number
        ),
        None,
    )
}

fn create_pr_review(
    owner: &str,
    repo: &str,
    pr_number: u32,
    body: &str,
    event: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;

    let valid_events = ["APPROVE", "REQUEST_CHANGES", "COMMENT"];
    if !valid_events.contains(&event) {
        return Err(format!(
            "Invalid event: '{}'. Must be one of: {}",
            event,
            valid_events.join(", ")
        ));
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}/reviews",
        encoded_owner, encoded_repo, pr_number
    );
    let req_body = serde_json::json!({
        "body": body,
        "event": event,
    });
    github_request("POST", &path, Some(req_body.to_string()))
}

fn list_pull_request_comments(
    owner: &str,
    repo: &str,
    pr_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/comments?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn reply_pull_request_comment(
    owner: &str,
    repo: &str,
    comment_id: u64,
    body: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/comments/{}/replies",
        encoded_owner, encoded_repo, comment_id
    );
    let req_body = serde_json::json!({ "body": body });
    github_request("POST", &path, Some(req_body.to_string()))
}

fn get_pull_request_reviews(
    owner: &str,
    repo: &str,
    pr_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/reviews?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn get_combined_status(owner: &str, repo: &str, r#ref: &str) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(r#ref, "ref")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_ref = url_encode_path(r#ref);
    let path = format!(
        "/repos/{}/{}/commits/{}/status",
        encoded_owner, encoded_repo, encoded_ref
    );
    github_request("GET", &path, None)
}

fn merge_pull_request(
    owner: &str,
    repo: &str,
    pr_number: u32,
    commit_title: Option<&str>,
    commit_message: Option<&str>,
    merge_method: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if let Some(v) = commit_title {
        validate_input_length(v, "commit_title")?;
    }
    if let Some(v) = commit_message {
        validate_input_length(v, "commit_message")?;
    }
    let method = merge_method.unwrap_or("merge");
    let valid_methods = ["merge", "squash", "rebase"];
    if !valid_methods.contains(&method) {
        return Err(format!(
            "Invalid merge_method: '{}'. Must be one of: {}",
            method,
            valid_methods.join(", ")
        ));
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}/merge",
        encoded_owner, encoded_repo, pr_number
    );
    let mut req_body = serde_json::json!({
        "merge_method": method,
    });
    if let Some(v) = commit_title {
        req_body["commit_title"] = serde_json::json!(v);
    }
    if let Some(v) = commit_message {
        req_body["commit_message"] = serde_json::json!(v);
    }
    github_request("PUT", &path, Some(req_body.to_string()))
}

fn list_repos(username: &str, page: Option<u32>, limit: Option<u32>) -> Result<String, String> {
    if !validate_path_segment(username) {
        return Err("Invalid username".into());
    }
    let encoded_username = url_encode_path(username);
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let mut path = format!("/users/{}/repos?per_page={}", encoded_username, limit);
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn search_repositories(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/repositories?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

fn search_code(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/code?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

fn search_issues_pull_requests(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

fn list_branches(
    owner: &str,
    repo: &str,
    protected: Option<bool>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/branches?per_page={}",
        encoded_owner, encoded_repo, limit
    );
    if let Some(protected) = protected {
        path.push_str("&protected=");
        path.push_str(if protected { "true" } else { "false" });
    }
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

fn create_branch(owner: &str, repo: &str, branch: &str, from_ref: &str) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(branch, "branch")?;
    validate_input_length(from_ref, "from_ref")?;

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let source_ref = normalize_ref_lookup(from_ref)?;
    let source_path = format!(
        "/repos/{}/{}/git/ref/{}",
        encoded_owner,
        encoded_repo,
        encode_repo_path(&source_ref)
    );
    let source_ref_resp = github_request("GET", &source_path, None)?;
    let source_ref_json: serde_json::Value = serde_json::from_str(&source_ref_resp)
        .map_err(|e| format!("Invalid GitHub response for source ref: {e}"))?;
    let sha = source_ref_json
        .pointer("/object/sha")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Source ref response missing object.sha".to_string())?;

    let req_body = serde_json::json!({
        "ref": normalize_branch_ref(branch)?,
        "sha": sha,
    });
    let path = format!("/repos/{}/{}/git/refs", encoded_owner, encoded_repo);
    github_request("POST", &path, Some(req_body.to_string()))
}

fn get_file_content(
    owner: &str,
    repo: &str,
    path: &str,
    r#ref: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    // Validate ref if provided
    if let Some(r#ref) = r#ref {
        validate_git_ref(r#ref, "ref")?;
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);

    let url_path = if let Some(r#ref) = r#ref {
        let encoded_ref = url_encode_query(r#ref);
        format!(
            "/repos/{}/{}/contents/{}?ref={}",
            encoded_owner, encoded_repo, encoded_path, encoded_ref
        )
    } else {
        format!(
            "/repos/{}/{}/contents/{}",
            encoded_owner, encoded_repo, encoded_path
        )
    };
    github_request("GET", &url_path, None)
}

fn create_or_update_file(
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    content: &str,
    sha: Option<&str>,
    branch: Option<&str>,
    committer: Option<GitCommitIdentity>,
    author: Option<GitCommitIdentity>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    validate_input_length(message, "message")?;
    validate_input_length(content, "content")?;
    if let Some(branch) = branch {
        validate_git_ref(branch, "branch")?;
    }
    if let Some(sha) = sha {
        validate_input_length(sha, "sha")?;
    }
    if let Some(committer) = &committer {
        validate_commit_identity(committer, "committer")?;
    }
    if let Some(author) = &author {
        validate_commit_identity(author, "author")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);
    let mut req_body = serde_json::json!({
        "message": message,
        "content": BASE64_STANDARD.encode(content.as_bytes()),
    });
    if let Some(sha) = sha {
        req_body["sha"] = serde_json::json!(sha);
    }
    if let Some(branch) = branch {
        req_body["branch"] = serde_json::json!(branch);
    }
    if let Some(committer) = committer {
        req_body["committer"] =
            serde_json::to_value(committer).map_err(|e| format!("Invalid committer: {e}"))?;
    }
    if let Some(author) = author {
        req_body["author"] =
            serde_json::to_value(author).map_err(|e| format!("Invalid author: {e}"))?;
    }

    let path = format!(
        "/repos/{}/{}/contents/{}",
        encoded_owner, encoded_repo, encoded_path
    );
    github_request("PUT", &path, Some(req_body.to_string()))
}

fn delete_file(
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    sha: &str,
    branch: Option<&str>,
    committer: Option<GitCommitIdentity>,
    author: Option<GitCommitIdentity>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    validate_input_length(message, "message")?;
    validate_input_length(sha, "sha")?;
    if let Some(branch) = branch {
        validate_git_ref(branch, "branch")?;
    }
    if let Some(committer) = &committer {
        validate_commit_identity(committer, "committer")?;
    }
    if let Some(author) = &author {
        validate_commit_identity(author, "author")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);
    let mut req_body = serde_json::json!({
        "message": message,
        "sha": sha,
    });
    if let Some(branch) = branch {
        req_body["branch"] = serde_json::json!(branch);
    }
    if let Some(committer) = committer {
        req_body["committer"] =
            serde_json::to_value(committer).map_err(|e| format!("Invalid committer: {e}"))?;
    }
    if let Some(author) = author {
        req_body["author"] =
            serde_json::to_value(author).map_err(|e| format!("Invalid author: {e}"))?;
    }

    let path = format!(
        "/repos/{}/{}/contents/{}",
        encoded_owner, encoded_repo, encoded_path
    );
    github_request("DELETE", &path, Some(req_body.to_string()))
}

fn list_releases(
    owner: &str,
    repo: &str,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/releases?per_page={}",
        encoded_owner, encoded_repo, limit
    );
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

fn create_release(
    owner: &str,
    repo: &str,
    tag_name: &str,
    target_commitish: Option<&str>,
    name: Option<&str>,
    body: Option<&str>,
    draft: bool,
    prerelease: bool,
    generate_release_notes: bool,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_git_ref(tag_name, "tag_name")?;
    if let Some(target_commitish) = target_commitish {
        validate_git_ref(target_commitish, "target_commitish")?;
    }
    if let Some(name) = name {
        validate_input_length(name, "name")?;
    }
    if let Some(body) = body {
        validate_input_length(body, "body")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/releases", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "tag_name": tag_name,
        "draft": draft,
        "prerelease": prerelease,
        "generate_release_notes": generate_release_notes,
    });
    if let Some(target_commitish) = target_commitish {
        req_body["target_commitish"] = serde_json::json!(target_commitish);
    }
    if let Some(name) = name {
        req_body["name"] = serde_json::json!(name);
    }
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

fn trigger_workflow(
    owner: &str,
    repo: &str,
    workflow_id: &str,
    r#ref: &str,
    inputs: Option<serde_json::Value>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    // Validate inputs size if present
    if let Some(valid_inputs) = &inputs {
        let inputs_str = valid_inputs.to_string();
        validate_input_length(&inputs_str, "inputs")?;
    }

    // Validate workflow_id - must be a safe filename
    if workflow_id.contains('/') || workflow_id.contains("..") || workflow_id.contains(':') {
        return Err("Invalid workflow_id: must be a filename or numeric ID".into());
    }
    // Validate ref - must be a valid git ref
    if r#ref.contains("..") || r#ref.contains(':') {
        return Err("Invalid ref: must be a valid branch, tag, or commit SHA".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_workflow_id = url_encode_path(workflow_id);
    let path = format!(
        "/repos/{}/{}/actions/workflows/{}/dispatches",
        encoded_owner, encoded_repo, encoded_workflow_id
    );
    let mut req_body = serde_json::json!({
        "ref": r#ref,
    });
    if let Some(inputs) = inputs {
        req_body["inputs"] = inputs;
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

fn get_workflow_runs(
    owner: &str,
    repo: &str,
    workflow_id: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    // Validate workflow_id if provided
    if let Some(wid) = workflow_id {
        if wid.contains('/') || wid.contains("..") || wid.contains(':') {
            return Err("Invalid workflow_id: must be a filename or numeric ID".into());
        }
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let mut path = if let Some(workflow_id) = workflow_id {
        let encoded_workflow_id = url_encode_path(workflow_id);
        format!(
            "/repos/{}/{}/actions/workflows/{}/runs?per_page={}",
            encoded_owner, encoded_repo, encoded_workflow_id, limit
        )
    } else {
        format!(
            "/repos/{}/{}/actions/runs?per_page={}",
            encoded_owner, encoded_repo, limit
        )
    };
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn header_value<'a>(headers: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    let lower = key.to_ascii_lowercase();
    headers
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == lower)
        .map(|(_, v)| v.as_str())
}

fn handle_webhook(webhook: GitHubWebhookRequest) -> Result<String, String> {
    let event = header_value(&webhook.headers, "x-github-event")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "Missing X-GitHub-Event header".to_string())?;

    let payload = webhook
        .body_json
        .ok_or_else(|| "Missing webhook.body_json".to_string())?;

    let event_type = github_event_type(event, &payload);
    let enriched_payload = github_enriched_payload(event, &webhook.headers, &payload, &event_type);

    let resp = ToolWebhookResponse {
        accepted: true,
        emit_events: vec![SystemEventIntent {
            source: "github".to_string(),
            event_type,
            payload: enriched_payload,
        }],
    };
    serde_json::to_string(&resp).map_err(|e| format!("Failed to encode webhook response: {e}"))
}

fn github_event_type(event: &str, payload: &serde_json::Value) -> String {
    let base = match event {
        "issues" => "issue",
        "pull_request" => "pr",
        "issue_comment" => {
            if payload.pointer("/issue/pull_request").is_some() {
                "pr.comment"
            } else {
                "issue.comment"
            }
        }
        "pull_request_review" => "pr.review",
        "pull_request_review_comment" => "pr.review_comment",
        "pull_request_review_thread" => "pr.review_thread",
        "check_suite" => "ci.check_suite",
        "check_run" => "ci.check_run",
        "status" => "ci.status",
        other => other,
    };

    if let Some(action) = payload.get("action").and_then(|v| v.as_str()) {
        if !action.is_empty() {
            return format!("{base}.{action}");
        }
    }

    base.to_string()
}

fn github_enriched_payload(
    raw_event: &str,
    headers: &HashMap<String, String>,
    payload: &serde_json::Value,
    event_type: &str,
) -> serde_json::Value {
    fn put_if_missing(
        obj: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        val: Option<serde_json::Value>,
    ) {
        if !obj.contains_key(key) {
            if let Some(v) = val {
                obj.insert(key.to_string(), v);
            }
        }
    }

    let mut obj = payload
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);

    put_if_missing(
        &mut obj,
        "event",
        Some(serde_json::Value::String(raw_event.to_string())),
    );
    put_if_missing(
        &mut obj,
        "event_type",
        Some(serde_json::Value::String(event_type.to_string())),
    );
    put_if_missing(
        &mut obj,
        "delivery_id",
        header_value(headers, "x-github-delivery")
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "action",
        payload
            .get("action")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "repository_name",
        payload
            .pointer("/repository/full_name")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "repository_owner",
        payload
            .pointer("/repository/owner/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "sender_login",
        payload
            .pointer("/sender/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "issue_number",
        payload.pointer("/issue/number").cloned(),
    );
    // For `issue_comment` webhooks on PRs, `/pull_request/number` is absent but
    // `/issue/number` is present and `/issue/pull_request` exists. Fall back to
    // `/issue/number` so PR-comment events carry `pr_number`.
    let pr_number = payload
        .pointer("/pull_request/number")
        .cloned()
        .or_else(|| {
            if payload.pointer("/issue/pull_request").is_some() {
                payload.pointer("/issue/number").cloned()
            } else {
                None
            }
        });
    put_if_missing(&mut obj, "pr_number", pr_number);
    put_if_missing(
        &mut obj,
        "comment_author",
        payload
            .pointer("/comment/user/login")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "comment_body",
        payload
            .pointer("/comment/body")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "review_state",
        payload
            .pointer("/review/state")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "pr_state",
        payload
            .pointer("/pull_request/state")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "pr_merged",
        payload.pointer("/pull_request/merged").cloned(),
    );
    put_if_missing(
        &mut obj,
        "pr_draft",
        payload.pointer("/pull_request/draft").cloned(),
    );
    put_if_missing(
        &mut obj,
        "base_branch",
        payload
            .pointer("/pull_request/base/ref")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "head_branch",
        payload
            .pointer("/pull_request/head/ref")
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "ci_status",
        payload
            .pointer("/check_run/status")
            .or_else(|| payload.pointer("/check_suite/status"))
            .or_else(|| payload.pointer("/status"))
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );
    put_if_missing(
        &mut obj,
        "ci_conclusion",
        payload
            .pointer("/check_run/conclusion")
            .or_else(|| payload.pointer("/check_suite/conclusion"))
            .or_else(|| payload.pointer("/state"))
            .and_then(|v| v.as_str())
            .map(|s| serde_json::Value::String(s.to_string())),
    );

    serde_json::Value::Object(obj)
}

const SCHEMA: &str = r#"{
    "type": "object",
    "required": ["action"],
    "oneOf": [
        {
            "properties": {
                "action": { "const": "get_repo" },
                "owner": { "type": "string", "description": "Repository owner (user or org)" },
                "repo": { "type": "string", "description": "Repository name" }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "create_repo" },
                "name": { "type": "string", "description": "New repository name" },
                "description": { "type": "string" },
                "private": { "type": "boolean", "default": false },
                "auto_init": { "type": "boolean", "default": false },
                "gitignore_template": { "type": "string" },
                "license_template": { "type": "string" },
                "org": { "type": "string", "description": "Optional organization name; omit to create under the authenticated user" }
            },
            "required": ["action", "name"]
        },
        {
            "properties": {
                "action": { "const": "list_issues" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "state": { "type": "string", "enum": ["open", "closed", "all"], "default": "open" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "create_issue" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "title": { "type": "string" },
                "body": { "type": "string" },
                "labels": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["action", "owner", "repo", "title"]
        },
        {
            "properties": {
                "action": { "const": "get_issue" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "issue_number": { "type": "integer" }
            },
            "required": ["action", "owner", "repo", "issue_number"]
        },
        {
            "properties": {
                "action": { "const": "list_issue_comments" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "issue_number": { "type": "integer" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo", "issue_number"]
        },
        {
            "properties": {
                "action": { "const": "create_issue_comment" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "issue_number": { "type": "integer" },
                "body": { "type": "string" }
            },
            "required": ["action", "owner", "repo", "issue_number", "body"]
        },
        {
            "properties": {
                "action": { "const": "list_pull_requests" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "state": { "type": "string", "enum": ["open", "closed", "all"], "default": "open" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "create_pull_request" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "title": { "type": "string" },
                "head": { "type": "string" },
                "base": { "type": "string" },
                "body": { "type": "string" },
                "draft": { "type": "boolean", "default": false }
            },
            "required": ["action", "owner", "repo", "title", "head", "base"]
        },
        {
            "properties": {
                "action": { "const": "get_pull_request" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" }
            },
            "required": ["action", "owner", "repo", "pr_number"]
        },
        {
            "properties": {
                "action": { "const": "get_pull_request_files" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" }
            },
            "required": ["action", "owner", "repo", "pr_number"]
        },
        {
            "properties": {
                "action": { "const": "create_pr_review" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" },
                "body": { "type": "string", "description": "Review comment" },
                "event": { "type": "string", "enum": ["APPROVE", "REQUEST_CHANGES", "COMMENT"] }
            },
            "required": ["action", "owner", "repo", "pr_number", "body", "event"]
        },
        {
            "properties": {
                "action": { "const": "list_pull_request_comments" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo", "pr_number"]
        },
        {
            "properties": {
                "action": { "const": "reply_pull_request_comment" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "comment_id": { "type": "integer" },
                "body": { "type": "string" }
            },
            "required": ["action", "owner", "repo", "comment_id", "body"]
        },
        {
            "properties": {
                "action": { "const": "get_pull_request_reviews" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo", "pr_number"]
        },
        {
            "properties": {
                "action": { "const": "get_combined_status" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "ref": { "type": "string" }
            },
            "required": ["action", "owner", "repo", "ref"]
        },
        {
            "properties": {
                "action": { "const": "merge_pull_request" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" },
                "commit_title": { "type": "string" },
                "commit_message": { "type": "string" },
                "merge_method": { "type": "string", "enum": ["merge", "squash", "rebase"], "default": "merge" }
            },
            "required": ["action", "owner", "repo", "pr_number"]
        },
        {
            "properties": {
                "action": { "const": "list_repos" },
                "username": { "type": "string" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "username"]
        },
        {
            "properties": {
                "action": { "const": "search_repositories" },
                "query": { "type": "string", "description": "GitHub repository search query" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 },
                "sort": { "type": "string" },
                "order": { "type": "string", "enum": ["asc", "desc"] }
            },
            "required": ["action", "query"]
        },
        {
            "properties": {
                "action": { "const": "search_code" },
                "query": { "type": "string", "description": "GitHub code search query" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 },
                "sort": { "type": "string" },
                "order": { "type": "string", "enum": ["asc", "desc"] }
            },
            "required": ["action", "query"]
        },
        {
            "properties": {
                "action": { "const": "search_issues_pull_requests" },
                "query": { "type": "string", "description": "GitHub issue/PR search query" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 },
                "sort": { "type": "string" },
                "order": { "type": "string", "enum": ["asc", "desc"] }
            },
            "required": ["action", "query"]
        },
        {
            "properties": {
                "action": { "const": "list_branches" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "protected": { "type": "boolean" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "create_branch" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "branch": { "type": "string", "description": "New branch name" },
                "from_ref": { "type": "string", "description": "Source branch or tag to branch from" }
            },
            "required": ["action", "owner", "repo", "branch", "from_ref"]
        },
        {
            "properties": {
                "action": { "const": "get_file_content" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "path": { "type": "string", "description": "File path in repo" },
                "ref": { "type": "string", "description": "Branch/commit (default: default branch)" }
            },
            "required": ["action", "owner", "repo", "path"]
        },
        {
            "properties": {
                "action": { "const": "create_or_update_file" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "path": { "type": "string", "description": "File path in repo" },
                "message": { "type": "string", "description": "Commit message" },
                "content": { "type": "string", "description": "Raw UTF-8 file content; the tool base64-encodes it for GitHub" },
                "sha": { "type": "string", "description": "Required when updating an existing file" },
                "branch": { "type": "string" },
                "committer": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "email": { "type": "string" }
                    },
                    "required": ["name", "email"]
                },
                "author": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "email": { "type": "string" }
                    },
                    "required": ["name", "email"]
                }
            },
            "required": ["action", "owner", "repo", "path", "message", "content"]
        },
        {
            "properties": {
                "action": { "const": "delete_file" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "path": { "type": "string", "description": "File path in repo" },
                "message": { "type": "string", "description": "Commit message" },
                "sha": { "type": "string", "description": "Blob SHA of the file to delete" },
                "branch": { "type": "string" },
                "committer": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "email": { "type": "string" }
                    },
                    "required": ["name", "email"]
                },
                "author": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "email": { "type": "string" }
                    },
                    "required": ["name", "email"]
                }
            },
            "required": ["action", "owner", "repo", "path", "message", "sha"]
        },
        {
            "properties": {
                "action": { "const": "list_releases" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "page": { "type": "integer" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "create_release" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "tag_name": { "type": "string" },
                "target_commitish": { "type": "string" },
                "name": { "type": "string" },
                "body": { "type": "string" },
                "draft": { "type": "boolean", "default": false },
                "prerelease": { "type": "boolean", "default": false },
                "generate_release_notes": { "type": "boolean", "default": false }
            },
            "required": ["action", "owner", "repo", "tag_name"]
        },
        {
            "properties": {
                "action": { "const": "trigger_workflow" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "workflow_id": { "type": "string", "description": "Workflow filename or ID" },
                "ref": { "type": "string", "description": "Branch to run on" },
                "inputs": { "type": "object" }
            },
            "required": ["action", "owner", "repo", "workflow_id", "ref"]
        },
        {
            "properties": {
                "action": { "const": "get_workflow_runs" },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "workflow_id": { "type": "string" },
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "fork_repo" },
                "owner": { "type": "string", "description": "Repository owner (user or org) to fork from" },
                "repo": { "type": "string", "description": "Repository name to fork" },
                "organization": { "type": "string", "description": "Optional organization to fork into; omit to fork into the authenticated user's account" },
                "name": { "type": "string", "description": "Optional name for the fork; defaults to the original repo name" },
                "default_branch_only": { "type": "boolean", "default": false, "description": "When true, only the default branch is copied into the fork" }
            },
            "required": ["action", "owner", "repo"]
        },
        {
            "properties": {
                "action": { "const": "handle_webhook" },
                "webhook": {
                    "type": "object",
                    "properties": {
                        "headers": {
                            "type": "object",
                            "additionalProperties": { "type": "string" }
                        },
                        "body_json": {
                            "type": "object",
                            "description": "Parsed GitHub webhook JSON payload"
                        }
                    },
                    "required": ["headers", "body_json"]
                }
            },
            "required": ["action", "webhook"]
        }
    ]
}"#;

export!(GitHubTool);

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_actions() -> std::collections::HashSet<String> {
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA).expect("schema should be valid JSON");
        schema["oneOf"]
            .as_array()
            .expect("schema.oneOf should be an array")
            .iter()
            .filter_map(|variant| {
                variant
                    .pointer("/properties/action/const")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect()
    }

    fn supported_actions() -> std::collections::HashSet<String> {
        [
            "get_repo",
            "create_repo",
            "list_issues",
            "create_issue",
            "get_issue",
            "list_issue_comments",
            "create_issue_comment",
            "list_pull_requests",
            "create_pull_request",
            "get_pull_request",
            "get_pull_request_files",
            "create_pr_review",
            "list_pull_request_comments",
            "reply_pull_request_comment",
            "get_pull_request_reviews",
            "get_combined_status",
            "merge_pull_request",
            "list_repos",
            "search_repositories",
            "search_code",
            "search_issues_pull_requests",
            "list_branches",
            "create_branch",
            "get_file_content",
            "create_or_update_file",
            "delete_file",
            "list_releases",
            "create_release",
            "trigger_workflow",
            "get_workflow_runs",
            "fork_repo",
            "handle_webhook",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn test_url_encode_path() {
        assert_eq!(url_encode_path("foo-bar_123.baz"), "foo-bar_123.baz");
        assert_eq!(url_encode_path("foo bar"), "foo%20bar");
        assert_eq!(url_encode_path("foo/bar"), "foo%2Fbar");
    }

    #[test]
    fn test_validate_path_segment() {
        assert!(validate_path_segment("foo"));
        assert!(validate_path_segment("foo-bar_123.baz"));
        assert!(!validate_path_segment(""));
        assert!(!validate_path_segment("foo/bar"));
        assert!(!validate_path_segment(".."));
        assert!(!validate_path_segment("foo bar"));
        assert!(!validate_path_segment("foo\nbar"));
    }

    #[test]
    fn test_header_value_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("X-Github-Event".to_string(), "push".to_string());
        assert_eq!(header_value(&headers, "x-github-event"), Some("push"));
        assert_eq!(header_value(&headers, "X-GITHUB-EVENT"), Some("push"));
        assert_eq!(header_value(&headers, "X-Github-Event"), Some("push"));
        assert_eq!(header_value(&headers, "x-nonexistent"), None);
    }

    #[test]
    fn test_input_length_validation() {
        assert!(validate_input_length("short", "test").is_ok());

        let long = "a".repeat(MAX_TEXT_LENGTH + 1);
        assert!(validate_input_length(&long, "test").is_err());
    }

    #[test]
    fn test_github_event_type_normalization() {
        assert_eq!(
            github_event_type("issues", &serde_json::json!({"action": "opened"})),
            "issue.opened"
        );
        assert_eq!(
            github_event_type(
                "pull_request",
                &serde_json::json!({"action": "synchronize"})
            ),
            "pr.synchronize"
        );
        assert_eq!(
            github_event_type(
                "issue_comment",
                &serde_json::json!({
                    "action": "created",
                    "issue": { "pull_request": { "url": "https://api.github.com/repos/org/repo/pulls/1" } }
                })
            ),
            "pr.comment.created"
        );
    }

    #[test]
    fn test_github_enriched_payload_extracts_common_fields() {
        let headers = HashMap::new();
        let payload = serde_json::json!({
            "action": "created",
            "repository": {
                "full_name": "nearai/ironclaw",
                "owner": { "login": "nearai" }
            },
            "sender": { "login": "maintainer1" },
            "issue": { "number": 77 },
            "comment": {
                "body": "Please update the implementation plan",
                "user": { "login": "maintainer1" }
            }
        });

        let enriched =
            github_enriched_payload("issue_comment", &headers, &payload, "issue.comment.created");
        assert_eq!(
            enriched.get("repository_name").and_then(|v| v.as_str()),
            Some("nearai/ironclaw")
        );
        // Original repository object is preserved
        assert!(enriched
            .get("repository")
            .and_then(|v| v.as_object())
            .is_some());
        assert_eq!(
            enriched.get("issue_number").and_then(|v| v.as_i64()),
            Some(77)
        );
        assert_eq!(
            enriched.get("comment_body").and_then(|v| v.as_str()),
            Some("Please update the implementation plan")
        );
    }

    #[test]
    fn test_enriched_payload_pr_number_from_issue_comment() {
        let headers = HashMap::new();
        let payload = serde_json::json!({
            "action": "created",
            "issue": {
                "number": 42,
                "pull_request": { "url": "https://api.github.com/repos/nearai/ironclaw/pulls/42" }
            },
            "comment": { "body": "LGTM", "user": { "login": "reviewer" } },
            "repository": { "full_name": "nearai/ironclaw", "owner": { "login": "nearai" } },
            "sender": { "login": "reviewer" }
        });

        let enriched =
            github_enriched_payload("issue_comment", &headers, &payload, "pr.comment.created");
        // pr_number should fall back to issue.number when issue.pull_request exists
        assert_eq!(
            enriched.get("pr_number").and_then(|v| v.as_i64()),
            Some(42),
            "pr_number should be set from issue.number for issue_comment on a PR"
        );
    }

    #[test]
    fn test_handle_webhook_requires_event_header() {
        let err = handle_webhook(GitHubWebhookRequest {
            headers: HashMap::new(),
            body_json: Some(serde_json::json!({"action":"opened"})),
        })
        .expect_err("expected header validation error");
        assert!(err.contains("X-GitHub-Event"));
    }

    #[test]
    fn test_handle_webhook_emits_event_intent() {
        let mut headers = HashMap::new();
        headers.insert("x-github-event".to_string(), "issues".to_string());
        headers.insert("x-github-delivery".to_string(), "abc-123".to_string());

        let out = handle_webhook(GitHubWebhookRequest {
            headers,
            body_json: Some(serde_json::json!({
                "action":"opened",
                "issue":{"number":42},
                "repository":{"full_name":"nearai/ironclaw"},
                "sender":{"login":"maintainer1"}
            })),
        })
        .expect("webhook handled");

        let json: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            json.pointer("/emit_events/0/source")
                .and_then(|v| v.as_str()),
            Some("github")
        );
        assert_eq!(
            json.pointer("/emit_events/0/event_type")
                .and_then(|v| v.as_str()),
            Some("issue.opened")
        );
        assert_eq!(
            json.pointer("/emit_events/0/payload/issue_number")
                .and_then(|v| v.as_i64()),
            Some(42)
        );
    }

    #[test]
    fn test_validate_git_ref_rejects_bad_names() {
        assert!(validate_git_ref("feature/test", "branch").is_ok());
        assert!(validate_git_ref("release/v1.2.3", "branch").is_ok());
        assert!(validate_git_ref("bad ref", "branch").is_err());
        assert!(validate_git_ref("../main", "branch").is_err());
        assert!(validate_git_ref("refs/heads/main.lock", "branch").is_err());
    }

    #[test]
    fn test_normalize_ref_lookup_and_branch_ref() {
        assert_eq!(
            normalize_ref_lookup("main").expect("main should normalize"),
            "heads/main"
        );
        assert_eq!(
            normalize_ref_lookup("refs/tags/v1.0.0").expect("tag ref should normalize"),
            "tags/v1.0.0"
        );
        assert_eq!(
            normalize_branch_ref("feature/github-tool-audit").expect("branch ref should normalize"),
            "refs/heads/feature/github-tool-audit"
        );
        assert_eq!(
            normalize_branch_ref("refs/heads/main").expect("qualified branch ref should pass"),
            "refs/heads/main"
        );
        assert!(normalize_ref_lookup("refs/pull/123/head").is_err());
        assert!(normalize_branch_ref("refs/tags/v1.0.0").is_err());
        assert!(normalize_branch_ref("tags/v1.0.0").is_err());
    }

    #[test]
    fn test_validate_repo_path_enforces_length_limit() {
        let long_path = format!("dir/{}", "a".repeat(MAX_TEXT_LENGTH));
        assert!(validate_repo_path("docs/readme.md").is_ok());
        assert!(validate_repo_path(&long_path).is_err());
    }

    #[test]
    fn test_validate_commit_identity_enforces_length_limit() {
        let identity = GitCommitIdentity {
            name: "IronClaw Bot".to_string(),
            email: "bot@example.com".to_string(),
        };
        assert!(validate_commit_identity(&identity, "committer").is_ok());

        let too_long = GitCommitIdentity {
            name: "a".repeat(MAX_TEXT_LENGTH + 1),
            email: "bot@example.com".to_string(),
        };
        assert!(validate_commit_identity(&too_long, "committer").is_err());
    }

    #[test]
    fn test_schema_includes_new_core_actions() {
        let actions = schema_actions();
        for action in [
            "create_repo",
            "search_repositories",
            "search_code",
            "search_issues_pull_requests",
            "list_branches",
            "create_branch",
            "create_or_update_file",
            "delete_file",
            "list_releases",
            "create_release",
        ] {
            assert!(
                actions.contains(action),
                "schema should include action {action}"
            );
        }
    }

    #[test]
    fn test_schema_matches_supported_action_set() {
        assert_eq!(schema_actions(), supported_actions());
    }

    #[test]
    fn test_readme_examples_only_reference_supported_actions() {
        let actions = schema_actions();
        let readme = include_str!("../README.md");
        let mut referenced = Vec::new();

        for line in readme.lines() {
            let Some((_, rhs)) = line.split_once("\"action\":") else {
                continue;
            };
            let rhs = rhs.trim();
            let Some(rest) = rhs.strip_prefix('"') else {
                continue;
            };
            let Some(action) = rest.split('"').next() else {
                continue;
            };
            referenced.push(action.to_string());
        }

        assert!(
            !referenced.is_empty(),
            "README should contain action examples"
        );
        for action in referenced {
            assert!(
                actions.contains(&action),
                "README references unsupported action {action}"
            );
        }
    }

    #[test]
    fn test_registry_description_claims_are_supported() {
        let actions = schema_actions();
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../registry/tools/github.json"))
                .expect("registry manifest should parse");
        let description = manifest["description"]
            .as_str()
            .expect("registry manifest should have a description")
            .to_ascii_lowercase();

        if description.contains("search") {
            assert!(
                actions.contains("search_repositories")
                    && actions.contains("search_code")
                    && actions.contains("search_issues_pull_requests"),
                "registry search claim should map to implemented search actions"
            );
        }
        if description.contains("releases") {
            assert!(
                actions.contains("list_releases") && actions.contains("create_release"),
                "registry releases claim should map to implemented release actions"
            );
        }
        if description.contains("file writes") {
            assert!(
                actions.contains("create_or_update_file") && actions.contains("delete_file"),
                "registry file write claim should map to implemented content actions"
            );
        }
    }
}
