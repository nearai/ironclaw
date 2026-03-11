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
    !s.is_empty() && !s.contains('/') && !s.contains("..") && !s.contains('?') && !s.contains('#')
}

struct GitHubTool;

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum GitHubAction {
    #[serde(rename = "get_repo")]
    GetRepo { owner: String, repo: String },
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
    #[serde(rename = "get_file_content")]
    GetFileContent {
        owner: String,
        repo: String,
        path: String,
        r#ref: Option<String>,
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
        "GitHub integration for managing repositories, issues, pull requests, \
         and workflows. Supports reading repo info, listing/creating issues, \
         reviewing PRs, and triggering GitHub Actions. \
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
        GitHubAction::GetFileContent {
            owner,
            repo,
            path,
            r#ref,
        } => get_file_content(&owner, &repo, &path, r#ref.as_deref()),
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
        "X-GitHub-Api-Version": "2022-11-28",
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

fn get_file_content(
    owner: &str,
    repo: &str,
    path: &str,
    r#ref: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    // Validate path segments - reject path traversal attempts and empty segments
    for segment in path.split('/') {
        if segment == ".." {
            return Err("Invalid path: path traversal not allowed".into());
        }
        if segment.is_empty() {
            return Err("Invalid path: empty segment not allowed".into());
        }
    }
    // Validate ref if provided
    if let Some(r#ref) = r#ref {
        if r#ref.contains("..") || r#ref.contains(':') {
            return Err("Invalid ref: must be a valid branch, tag, or commit SHA".into());
        }
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    // Path can contain slashes, so we encode each segment separately
    let encoded_path = path
        .split('/')
        .map(url_encode_path)
        .collect::<Vec<_>>()
        .join("/");

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
    put_if_missing(
        &mut obj,
        "pr_number",
        payload.pointer("/pull_request/number").cloned(),
    );
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
                "limit": { "type": "integer", "default": 30 }
            },
            "required": ["action", "username"]
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
        }
    ]
}"#;

export!(GitHubTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode_path() {
        assert_eq!(url_encode_path("foo-bar_123.baz"), "foo-bar_123.baz");
        assert_eq!(url_encode_path("foo bar"), "foo%20bar");
        assert_eq!(url_encode_path("foo/bar"), "foo%2Fbar");
    }

    #[test]
    fn test_validate_path_segment() {
        assert!(validate_path_segment("foo"));
        assert!(!validate_path_segment(""));
        assert!(!validate_path_segment("foo/bar"));
        assert!(!validate_path_segment(".."));
        // Empty segments are handled in get_file_content logic, not here
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
        assert!(enriched.get("repository").and_then(|v| v.as_object()).is_some());
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
}
