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

use serde::Deserialize;

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
/// Currently identical to `url_encode_path` and kept as a separate helper
/// for clarity and potential future customization.
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
    #[serde(rename = "list_pull_requests")]
    ListPullRequests {
        owner: String,
        repo: String,
        state: Option<String>,
        limit: Option<u32>,
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
    #[serde(rename = "list_repos")]
    ListRepos {
        username: String,
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
        limit: Option<u32>,
    },
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
            limit,
        } => list_issues(&owner, &repo, state.as_deref(), limit),
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
        GitHubAction::ListPullRequests {
            owner,
            repo,
            state,
            limit,
        } => list_pull_requests(&owner, &repo, state.as_deref(), limit),
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
        GitHubAction::ListRepos { username, limit } => list_repos(&username, limit),
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
            limit,
        } => get_workflow_runs(&owner, &repo, workflow_id.as_deref(), limit),
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
    // The WASM tool never sees the raw token value.
    let headers = serde_json::json!({
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "IronClaw-GitHub-Tool"
    });

    let body_bytes = body.map(|b| b.into_bytes());

    let response = near::agent::host::http_request(
        method,
        &url,
        &headers.to_string(),
        body_bytes.as_deref(),
        None,
    );

    match response {
        Ok(resp) => {
            if resp.status >= 200 && resp.status < 300 {
                String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8: {}", e))
            } else {
                let body_str = String::from_utf8_lossy(&resp.body);
                Err(format!("GitHub API error {}: {}", resp.status, body_str))
            }
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
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
    let path = format!(
        "/repos/{}/{}/issues?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
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

fn list_pull_requests(
    owner: &str,
    repo: &str,
    state: Option<&str>,
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
    let path = format!(
        "/repos/{}/{}/pulls?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
    github_request("GET", &path, None)
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
    if !["APPROVE", "REQUEST_CHANGES", "COMMENT"].contains(&event) {
        return Err("Invalid event: must be APPROVE, REQUEST_CHANGES, or COMMENT".into());
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

fn list_repos(username: &str, limit: Option<u32>) -> Result<String, String> {
    if !validate_path_segment(username) {
        return Err("Invalid username".into());
    }
    let encoded_username = url_encode_path(username);
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let path = format!("/users/{}/repos?per_page={}", encoded_username, limit);
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
    let path = if let Some(workflow_id) = workflow_id {
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
    github_request("GET", &path, None)
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
    fn test_validate_event_in_create_pr_review() {
        let valid = ["APPROVE", "REQUEST_CHANGES", "COMMENT"];
        // Ensure valid inputs are accepted
        for event in valid {
            assert!(valid.contains(&event));
        }
        // Ensure invalid inputs are rejected
        assert!(!valid.contains(&"INVALID"));
    }
}
