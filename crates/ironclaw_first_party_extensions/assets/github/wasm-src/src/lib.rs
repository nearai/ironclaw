//! First-party Reborn GitHub WASM tool.
//!
//! This component intentionally exposes only the Reborn GitHub slice:
//! `github.search_issues`, `github.get_issue`, and `github.comment_issue`.
//! Authentication is mediated by the host HTTP egress path; the component never
//! reads or constructs a GitHub token.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../../wit/tool.wit",
});

use serde::Deserialize;

const GITHUB_API_ROOT: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2026-03-10";
const HTTP_TIMEOUT_MS: u32 = 10_000;
const MAX_TEXT_LENGTH: usize = 65_536;

struct GitHubTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitHubOperation {
    SearchIssues,
    GetIssue,
    CommentIssue,
}

#[derive(Debug, Deserialize)]
struct ToolContext {
    capability_id: String,
}

#[derive(Debug, Deserialize)]
struct SearchIssuesParams {
    query: String,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetIssueParams {
    owner: String,
    repo: String,
    issue_number: u32,
}

#[derive(Debug, Deserialize)]
struct CommentIssueParams {
    owner: String,
    repo: String,
    issue_number: u32,
    body: String,
}

impl exports::near::agent::tool::Guest for GitHubTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(error),
            },
        }
    }

    fn schema() -> String {
        let search = schema_value(include_str!(
            "../../schemas/github/search_issues.input.v1.json"
        ));
        let get_issue = schema_value(include_str!("../../schemas/github/get_issue.input.v1.json"));
        let comment_issue = schema_value(include_str!(
            "../../schemas/github/comment_issue.input.v1.json"
        ));
        serde_json::json!({
            "type": "object",
            "oneOf": [search, get_issue, comment_issue]
        })
        .to_string()
    }

    fn description() -> String {
        "First-party GitHub Reborn tool for searching issues, fetching one issue, and commenting on an issue. GitHub credentials are injected only by host HTTP egress."
            .to_string()
    }
}

fn schema_value(schema: &str) -> serde_json::Value {
    serde_json::from_str(schema).unwrap_or_else(|_| serde_json::json!({"type": "object"}))
}

fn execute_inner(params: &str, context: Option<&str>) -> Result<String, String> {
    match operation_from_context(context)? {
        GitHubOperation::SearchIssues => search_issues(
            serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?,
        ),
        GitHubOperation::GetIssue => {
            get_issue(serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?)
        }
        GitHubOperation::CommentIssue => comment_issue(
            serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?,
        ),
    }
}

fn operation_from_context(context: Option<&str>) -> Result<GitHubOperation, String> {
    let context = context.ok_or_else(|| "missing_invocation_context".to_string())?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| "invalid_invocation_context".to_string())?;
    match context.capability_id.as_str() {
        "github.search_issues" => Ok(GitHubOperation::SearchIssues),
        "github.get_issue" => Ok(GitHubOperation::GetIssue),
        "github.comment_issue" => Ok(GitHubOperation::CommentIssue),
        _ => Err("unsupported_github_capability".to_string()),
    }
}

fn search_issues(params: SearchIssuesParams) -> Result<String, String> {
    validate_text(&params.query, "query")?;
    validate_search_sort(params.sort.as_deref())?;
    validate_order(params.order.as_deref())?;

    let limit = params.limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(&params.query),
        limit
    );

    if let Some(page) = params.page {
        path.push_str("&page=");
        path.push_str(&page.to_string());
    }
    if let Some(sort) = params.sort {
        path.push_str("&sort=");
        path.push_str(&url_encode_query(&sort));
    }
    if let Some(order) = params.order {
        path.push_str("&order=");
        path.push_str(&order);
    }

    github_request("GET", &path, None)
}

fn get_issue(params: GetIssueParams) -> Result<String, String> {
    validate_repo(&params.owner, &params.repo)?;

    let path = format!(
        "/repos/{}/{}/issues/{}",
        url_encode_path(&params.owner),
        url_encode_path(&params.repo),
        params.issue_number
    );

    github_request("GET", &path, None)
}

fn comment_issue(params: CommentIssueParams) -> Result<String, String> {
    validate_repo(&params.owner, &params.repo)?;
    validate_text(&params.body, "body")?;

    let path = format!(
        "/repos/{}/{}/issues/{}/comments",
        url_encode_path(&params.owner),
        url_encode_path(&params.repo),
        params.issue_number
    );
    let body = serde_json::json!({ "body": params.body }).to_string();

    github_request("POST", &path, Some(body))
}

fn github_request(method: &str, path: &str, body: Option<String>) -> Result<String, String> {
    let url = format!("{GITHUB_API_ROOT}{path}");
    let headers = serde_json::json!({
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
        "X-GitHub-Api-Version": GITHUB_API_VERSION,
        "User-Agent": "IronClaw-GitHub-Reborn-WASM"
    });

    let body_bytes = body.map(String::into_bytes);
    let response = near::agent::host::http_request(
        method,
        &url,
        &headers.to_string(),
        body_bytes.as_deref(),
        Some(HTTP_TIMEOUT_MS),
    )
    .map_err(|error| sanitize_host_error(&error))?;

    if (200..300).contains(&response.status) {
        let body =
            String::from_utf8(response.body).map_err(|_| "github_api_invalid_utf8".to_string())?;
        let _: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| "github_api_invalid_json".to_string())?;
        return Ok(body);
    }

    Err(format!("github_api_error_status_{}", response.status))
}

fn sanitize_host_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.contains("token")
    {
        return "AuthRequired".to_string();
    }
    if lower.contains("timeout") || lower.contains("deadline") {
        return "github_api_timeout".to_string();
    }
    if lower.contains("redirect") {
        return "github_api_redirect_denied".to_string();
    }
    if lower.contains("body") || lower.contains("size") || lower.contains("large") {
        return "github_api_body_limit".to_string();
    }
    if lower.contains("deny") || lower.contains("allow") || lower.contains("host") {
        return "github_api_egress_denied".to_string();
    }
    "github_api_request_failed".to_string()
}

fn validate_repo(owner: &str, repo: &str) -> Result<(), String> {
    if validate_path_segment(owner) && validate_path_segment(repo) {
        Ok(())
    } else {
        Err("invalid_repository".to_string())
    }
}

fn validate_text(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_TEXT_LENGTH {
        Err(format!("invalid_{field}_too_large"))
    } else {
        Ok(())
    }
}

fn validate_path_segment(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('/')
        && !value.contains("..")
        && !value.contains('?')
        && !value.contains('#')
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
}

fn validate_search_sort(sort: Option<&str>) -> Result<(), String> {
    match sort {
        None => Ok(()),
        Some(
            "comments"
            | "reactions"
            | "reactions-+1"
            | "reactions--1"
            | "reactions-smile"
            | "reactions-thinking_face"
            | "reactions-heart"
            | "reactions-tada"
            | "interactions"
            | "created"
            | "updated",
        ) => Ok(()),
        Some(_) => Err("invalid_sort".to_string()),
    }
}

fn validate_order(order: Option<&str>) -> Result<(), String> {
    match order {
        None | Some("asc" | "desc") => Ok(()),
        Some(_) => Err("invalid_order".to_string()),
    }
}

fn url_encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(byte & 0x0F) as usize]));
            }
        }
    }
    out
}

fn url_encode_query(value: &str) -> String {
    url_encode_path(value)
}

export!(GitHubTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_comes_from_host_context_not_param_shape() {
        assert_eq!(
            operation_from_context(Some(r#"{"capability_id":"github.get_issue"}"#)).unwrap(),
            GitHubOperation::GetIssue
        );
    }

    #[test]
    fn operation_rejects_missing_or_unknown_context() {
        assert_eq!(
            operation_from_context(None).unwrap_err(),
            "missing_invocation_context"
        );
        assert_eq!(
            operation_from_context(Some(r#"{"capability_id":"github.create_issue"}"#)).unwrap_err(),
            "unsupported_github_capability"
        );
    }
}
