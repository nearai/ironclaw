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

use serde::{Deserialize, Serialize};

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
    ListRepos { username: String, limit: Option<u32> },
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
         Requires GitHub Personal Access Token with 'repo' and 'workflow' scopes."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: GitHubAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Executing GitHub action: {:?}", action),
    );

    let token = get_github_token()?;

    match action {
        GitHubAction::GetRepo { owner, repo } => get_repo(&token, &owner, &repo),
        GitHubAction::ListIssues {
            owner,
            repo,
            state,
            limit,
        } => list_issues(&token, &owner, &repo, state.as_deref(), limit),
        GitHubAction::CreateIssue {
            owner,
            repo,
            title,
            body,
            labels,
        } => create_issue(&token, &owner, &repo, &title, body.as_deref(), labels),
        GitHubAction::GetIssue {
            owner,
            repo,
            issue_number,
        } => get_issue(&token, &owner, &repo, issue_number),
        GitHubAction::ListPullRequests {
            owner,
            repo,
            state,
            limit,
        } => list_pull_requests(&token, &owner, &repo, state.as_deref(), limit),
        GitHubAction::GetPullRequest {
            owner,
            repo,
            pr_number,
        } => get_pull_request(&token, &owner, &repo, pr_number),
        GitHubAction::GetPullRequestFiles {
            owner,
            repo,
            pr_number,
        } => get_pull_request_files(&token, &owner, &repo, pr_number),
        GitHubAction::CreatePrReview {
            owner,
            repo,
            pr_number,
            body,
            event,
        } => create_pr_review(&token, &owner, &repo, pr_number, &body, &event),
        GitHubAction::ListRepos { username, limit } => list_repos(&token, &username, limit),
        GitHubAction::GetFileContent { owner, repo, path, r#ref } => {
            get_file_content(&token, &owner, &repo, &path, r#ref.as_deref())
        }
        GitHubAction::TriggerWorkflow {
            owner,
            repo,
            workflow_id,
            r#ref,
            inputs,
        } => trigger_workflow(&token, &owner, &repo, &workflow_id, &r#ref, inputs),
        GitHubAction::GetWorkflowRuns {
            owner,
            repo,
            workflow_id,
            limit,
        } => get_workflow_runs(&token, &owner, &repo, workflow_id.as_deref(), limit),
    }
}

fn get_github_token() -> Result<String, String> {
    if let Some(val) = near::agent::host::workspace_read("github/token") {
        let trimmed = val.trim().to_string();
        if trimmed.is_empty() {
            return Err("github/token is empty".into());
        }
        return Ok(trimmed);
    }
    Err("GitHub token not found. Store your Personal Access Token at \
         github/token using memory_write. Token needs 'repo' and 'workflow' scopes."
        .into())
}

fn github_request(
    token: &str,
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<String, String> {
    let url = format!("https://api.github.com{}", path);
    
    let mut headers = vec![
        ("Authorization", format!("Bearer {}", token)),
        ("Accept", "application/vnd.github+json".to_string()),
        ("X-GitHub-Api-Version", "2022-11-28".to_string()),
        ("User-Agent", "IronClaw-GitHub-Tool".to_string()),
    ];

    if body.is_some() {
        headers.push(("Content-Type", "application/json".to_string()));
    }

    let response = if let Some(body) = body {
        near::agent::host::http_request(&url, method, &headers, Some(&body))
    } else {
        near::agent::host::http_request(&url, method, &headers, None)
    };

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

fn get_repo(token: &str, owner: &str, repo: &str) -> Result<String, String> {
    github_request(token, "GET", &format!("/repos/{}/{}", owner, repo), None)
}

fn list_issues(
    token: &str,
    owner: &str,
    repo: &str,
    state: Option<&str>,
    limit: Option<u32>,
) -> Result<String, String> {
    let state = state.unwrap_or("open");
    let limit = limit.unwrap_or(30);
    let path = format!(
        "/repos/{}/{}/issues?state={}&per_page={}",
        owner, repo, state, limit
    );
    github_request(token, "GET", &path, None)
}

fn create_issue(
    token: &str,
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<Vec<String>>,
) -> Result<String, String> {
    let path = format!("/repos/{}/{}/issues", owner, repo);
    let mut req_body = serde_json::json!({
        "title": title,
    });
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    if let Some(labels) = labels {
        req_body["labels"] = serde_json::json!(labels);
    }
    github_request(
        token,
        "POST",
        &path,
        Some(req_body.to_string()),
    )
}

fn get_issue(
    token: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<String, String> {
    github_request(
        token,
        "GET",
        &format!("/repos/{}/{}/issues/{}", owner, repo, issue_number),
        None,
    )
}

fn list_pull_requests(
    token: &str,
    owner: &str,
    repo: &str,
    state: Option<&str>,
    limit: Option<u32>,
) -> Result<String, String> {
    let state = state.unwrap_or("open");
    let limit = limit.unwrap_or(30);
    let path = format!(
        "/repos/{}/{}/pulls?state={}&per_page={}",
        owner, repo, state, limit
    );
    github_request(token, "GET", &path, None)
}

fn get_pull_request(
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u32,
) -> Result<String, String> {
    github_request(
        token,
        "GET",
        &format!("/repos/{}/{}/pulls/{}", owner, repo, pr_number),
        None,
    )
}

fn get_pull_request_files(
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u32,
) -> Result<String, String> {
    github_request(
        token,
        "GET",
        &format!("/repos/{}/{}/pulls/{}/files", owner, repo, pr_number),
        None,
    )
}

fn create_pr_review(
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u32,
    body: &str,
    event: &str,
) -> Result<String, String> {
    let path = format!("/repos/{}/{}/pulls/{}/reviews", owner, repo, pr_number);
    let req_body = serde_json::json!({
        "body": body,
        "event": event,
    });
    github_request(token, "POST", &path, Some(req_body.to_string()))
}

fn list_repos(token: &str, username: &str, limit: Option<u32>) -> Result<String, String> {
    let limit = limit.unwrap_or(30);
    let path = format!("/users/{}/repos?per_page={}", username, limit);
    github_request(token, "GET", &path, None)
}

fn get_file_content(
    token: &str,
    owner: &str,
    repo: &str,
    path: &str,
    r#ref: Option<&str>,
) -> Result<String, String> {
    let url_path = if let Some(r#ref) = r#ref {
        format!("/repos/{}/{}/contents/{}?ref={}", owner, repo, path, r#ref)
    } else {
        format!("/repos/{}/{}/contents/{}", owner, repo, path)
    };
    github_request(token, "GET", &url_path, None)
}

fn trigger_workflow(
    token: &str,
    owner: &str,
    repo: &str,
    workflow_id: &str,
    r#ref: &str,
    inputs: Option<serde_json::Value>,
) -> Result<String, String> {
    let path = format!(
        "/repos/{}/{}/actions/workflows/{}/dispatches",
        owner, repo, workflow_id
    );
    let mut req_body = serde_json::json!({
        "ref": r#ref,
    });
    if let Some(inputs) = inputs {
        req_body["inputs"] = inputs;
    }
    github_request(token, "POST", &path, Some(req_body.to_string()))
}

fn get_workflow_runs(
    token: &str,
    owner: &str,
    repo: &str,
    workflow_id: Option<&str>,
    limit: Option<u32>,
) -> Result<String, String> {
    let limit = limit.unwrap_or(30);
    let path = if let Some(workflow_id) = workflow_id {
        format!(
            "/repos/{}/{}/actions/workflows/{}/runs?per_page={}",
            owner, repo, workflow_id, limit
        )
    } else {
        format!(
            "/repos/{}/{}/actions/runs?per_page={}",
            owner, repo, limit
        )
    };
    github_request(token, "GET", &path, None)
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
