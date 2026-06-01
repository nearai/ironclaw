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
const MAX_QUERY_LENGTH: usize = 512;
const MAX_COMMENT_BODY_LENGTH: usize = 65_536;
const MAX_REPOSITORY_SEGMENT_LENGTH: usize = 100;

struct GitHubTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitHubOperation {
    SearchIssues,
    GetIssue,
    CommentIssue,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolContext {
    capability_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchIssuesParams {
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetIssueParams {
    owner: String,
    repo: String,
    issue_number: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
                error: Some(guest_error_payload(&error)),
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
    serde_json::from_str(schema).expect("bundled GitHub schema must be valid JSON")
    // safety: bundled schemas are static assets covered by `validates_static_schema_json`.
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
        _ => Err("unsupported_capability".to_string()),
    }
}

fn search_issues(params: SearchIssuesParams) -> Result<String, String> {
    let query = search_query(&params)?;
    validate_text(&query, "query", MAX_QUERY_LENGTH)?;
    validate_search_page(params.page)?;
    validate_search_limit(params.limit)?;
    validate_search_sort(params.sort.as_deref())?;
    validate_order(params.order.as_deref())?;

    let limit = params.limit.unwrap_or(30);
    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(&query),
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

fn search_query(params: &SearchIssuesParams) -> Result<String, String> {
    let mut parts = Vec::new();
    if params.repository.is_some() && (params.owner.is_some() || params.repo.is_some()) {
        return Err("invalid_repository".to_string());
    }
    if let Some(query) = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        parts.push(query.to_string());
    }
    if let Some(repository) = params.repository.as_deref() {
        let (owner, repo) = repository
            .split_once('/')
            .ok_or_else(|| "invalid_repository".to_string())?;
        validate_repo(owner, repo)?;
        parts.push(format!("repo:{owner}/{repo}"));
    }
    match (params.owner.as_deref(), params.repo.as_deref()) {
        (Some(owner), Some(repo)) => {
            validate_repo(owner, repo)?;
            parts.push(format!("repo:{owner}/{repo}"));
        }
        (None, Some(repo)) => {
            let (owner, repo) = repo
                .split_once('/')
                .ok_or_else(|| "invalid_repository".to_string())?;
            validate_repo(owner, repo)?;
            parts.push(format!("repo:{owner}/{repo}"));
        }
        (None, None) => {}
        (Some(_), None) => return Err("invalid_repository".to_string()),
    }
    push_qualifier(&mut parts, "author", params.author.as_deref())?;
    push_qualifier(&mut parts, "assignee", params.assignee.as_deref())?;
    push_qualifier(&mut parts, "involves", params.involves.as_deref())?;
    if let Some(state) = params.state.as_deref() {
        validate_search_state(state)?;
        parts.push(format!("state:{state}"));
    }
    if let Some(issue_type) = params.issue_type.as_deref() {
        validate_search_type(issue_type)?;
        parts.push(format!("is:{issue_type}"));
    }

    if parts.is_empty() {
        Err("invalid_query_empty".to_string())
    } else {
        Ok(parts.join(" "))
    }
}

fn push_qualifier(
    parts: &mut Vec<String>,
    qualifier: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if validate_search_qualifier_value(value) {
        parts.push(format!("{qualifier}:{value}"));
        Ok(())
    } else {
        Err(format!("invalid_{qualifier}"))
    }
}

fn get_issue(params: GetIssueParams) -> Result<String, String> {
    validate_repo(&params.owner, &params.repo)?;
    validate_issue_number(params.issue_number)?;

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
    validate_issue_number(params.issue_number)?;
    validate_text(&params.body, "body", MAX_COMMENT_BODY_LENGTH)?;

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
            String::from_utf8(response.body).map_err(|_| "host_http_invalid_utf8".to_string())?;
        return Ok(body);
    }

    match response.status {
        401 => Err("AuthRequired".to_string()),
        403 => Err("host_http_forbidden".to_string()),
        422 => Err("invalid_parameters".to_string()),
        429 => Err("host_http_rate_limited".to_string()),
        status => Err(format!("host_http_error_status_{status}")),
    }
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
        return "host_http_timeout".to_string();
    }
    if lower.contains("redirect") {
        return "host_http_redirect_denied".to_string();
    }
    if lower.contains("body") || lower.contains("size") || lower.contains("large") {
        return "host_http_body_limit".to_string();
    }
    if lower.contains("deny")
        || lower.contains("denied")
        || lower.contains("policy")
        || lower.contains("allow")
        || lower.contains("host")
    {
        return "host_http_network_denied".to_string();
    }
    "host_http_request_failed".to_string()
}

fn validate_repo(owner: &str, repo: &str) -> Result<(), String> {
    if validate_path_segment(owner) && validate_path_segment(repo) {
        Ok(())
    } else {
        Err("invalid_repository".to_string())
    }
}

fn validate_text(value: &str, field: &str, max_length: usize) -> Result<(), String> {
    if value.is_empty() {
        Err(format!("invalid_{field}_empty"))
    } else if value.len() > max_length {
        Err(format!("invalid_{field}_too_large"))
    } else {
        Ok(())
    }
}

fn validate_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REPOSITORY_SEGMENT_LENGTH
        && !value.contains('/')
        && !value.contains("..")
        && !value.contains(':')
        && !value.contains('?')
        && !value.contains('#')
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
}

fn validate_search_qualifier_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REPOSITORY_SEGMENT_LENGTH
        && !value.contains(char::is_whitespace)
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, ':' | '"' | '(' | ')'))
}

fn guest_error_payload(code: &str) -> String {
    serde_json::json!({
        "code": code,
        "kind": guest_error_kind(code),
    })
    .to_string()
}

fn guest_error_kind(code: &str) -> &'static str {
    match code {
        "AuthRequired" => "auth_required",
        "missing_invocation_context"
        | "invalid_invocation_context"
        | "unsupported_capability"
        | "invalid_parameters"
        | "invalid_repository"
        | "invalid_query_empty"
        | "invalid_query_too_large"
        | "invalid_author"
        | "invalid_assignee"
        | "invalid_involves"
        | "invalid_state"
        | "invalid_type"
        | "invalid_sort"
        | "invalid_order"
        | "invalid_page"
        | "invalid_limit"
        | "invalid_issue_number"
        | "invalid_body_empty"
        | "invalid_body_too_large" => "input",
        "host_http_body_limit" => "output_too_large",
        "host_http_timeout" => "executor",
        "host_http_network_denied" => "network_denied",
        "host_http_forbidden" | "host_http_rate_limited" => "client",
        _ => "operation_failed",
    }
}

fn validate_search_state(state: &str) -> Result<(), String> {
    match state {
        "open" | "closed" => Ok(()),
        _ => Err("invalid_state".to_string()),
    }
}

fn validate_search_type(issue_type: &str) -> Result<(), String> {
    match issue_type {
        "issue" | "pr" => Ok(()),
        _ => Err("invalid_type".to_string()),
    }
}

fn validate_search_sort(sort: Option<&str>) -> Result<(), String> {
    match sort {
        None | Some("comments" | "created" | "updated") => Ok(()),
        Some(_) => Err("invalid_sort".to_string()),
    }
}

fn validate_order(order: Option<&str>) -> Result<(), String> {
    match order {
        None | Some("asc" | "desc") => Ok(()),
        Some(_) => Err("invalid_order".to_string()),
    }
}

fn validate_search_page(page: Option<u32>) -> Result<(), String> {
    match page {
        None | Some(1..=100) => Ok(()),
        Some(_) => Err("invalid_page".to_string()),
    }
}

fn validate_search_limit(limit: Option<u32>) -> Result<(), String> {
    match limit {
        None | Some(1..=100) => Ok(()),
        Some(_) => Err("invalid_limit".to_string()),
    }
}

fn validate_issue_number(issue_number: u32) -> Result<(), String> {
    if issue_number == 0 {
        Err("invalid_issue_number".to_string())
    } else {
        Ok(())
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
    use crate::exports::near::agent::tool::Guest;

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
            "unsupported_capability"
        );
    }

    #[test]
    fn rejects_parameters_that_do_not_match_advertised_schema() {
        assert_eq!(
            search_issues(SearchIssuesParams {
                query: Some(String::new()),
                repository: None,
                owner: None,
                repo: None,
                author: None,
                assignee: None,
                involves: None,
                state: None,
                issue_type: None,
                page: None,
                limit: None,
                sort: None,
                order: None,
            })
            .unwrap_err(),
            "invalid_query_empty"
        );
        assert_eq!(
            search_issues(SearchIssuesParams {
                query: Some("repo:nearai/ironclaw is:issue".to_string()),
                repository: None,
                owner: None,
                repo: None,
                author: None,
                assignee: None,
                involves: None,
                state: None,
                issue_type: None,
                page: Some(0),
                limit: None,
                sort: None,
                order: None,
            })
            .unwrap_err(),
            "invalid_page"
        );
        assert_eq!(
            search_issues(SearchIssuesParams {
                query: Some("repo:nearai/ironclaw is:issue".to_string()),
                repository: None,
                owner: None,
                repo: None,
                author: None,
                assignee: None,
                involves: None,
                state: None,
                issue_type: None,
                page: None,
                limit: Some(0),
                sort: None,
                order: None,
            })
            .unwrap_err(),
            "invalid_limit"
        );
        assert_eq!(
            search_issues(SearchIssuesParams {
                query: Some("repo:nearai/ironclaw is:issue".to_string()),
                repository: None,
                owner: None,
                repo: None,
                author: None,
                assignee: None,
                involves: None,
                state: None,
                issue_type: None,
                page: None,
                limit: None,
                sort: Some("reactions".to_string()),
                order: None,
            })
            .unwrap_err(),
            "invalid_sort"
        );
        assert_eq!(
            comment_issue(CommentIssueParams {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                issue_number: 0,
                body: "comment".to_string(),
            })
            .unwrap_err(),
            "invalid_issue_number"
        );
        assert_eq!(
            comment_issue(CommentIssueParams {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                issue_number: 1,
                body: String::new(),
            })
            .unwrap_err(),
            "invalid_body_empty"
        );
    }

    #[test]
    fn builds_query_from_structured_search_fields() {
        let mut params = empty_search_params();
        params.repo = Some("nearai/ironclaw".to_string());
        params.author = Some("serrrfirat".to_string());
        params.state = Some("open".to_string());
        params.issue_type = Some("issue".to_string());
        let query = search_query(&params).expect("structured fields build search query");

        assert_eq!(
            query,
            "repo:nearai/ironclaw author:serrrfirat state:open is:issue"
        );
    }

    #[test]
    fn search_query_rejects_duplicate_repository_inputs() {
        let mut params = empty_search_params();
        params.repository = Some("nearai/ironclaw".to_string());
        params.owner = Some("nearai".to_string());
        params.repo = Some("ironclaw".to_string());

        assert_eq!(search_query(&params).unwrap_err(), "invalid_repository");
    }

    #[test]
    fn search_query_rejects_owner_without_repo() {
        let mut params = empty_search_params();
        params.owner = Some("nearai".to_string());

        assert_eq!(search_query(&params).unwrap_err(), "invalid_repository");
    }

    #[test]
    fn search_query_rejects_repository_without_slash() {
        let mut params = empty_search_params();
        params.repository = Some("ironclaw".to_string());

        assert_eq!(search_query(&params).unwrap_err(), "invalid_repository");
    }

    #[test]
    fn push_qualifier_validates_structured_qualifier_values() {
        let mut parts = vec!["repo:nearai/ironclaw".to_string()];
        push_qualifier(&mut parts, "author", Some("   ")).unwrap();
        assert_eq!(parts, vec!["repo:nearai/ironclaw"]);

        assert_eq!(
            push_qualifier(&mut parts, "author", Some("bad user")).unwrap_err(),
            "invalid_author"
        );
        assert_eq!(
            push_qualifier(&mut parts, "author", Some("bad\nuser")).unwrap_err(),
            "invalid_author"
        );
        assert_eq!(
            push_qualifier(&mut parts, "author", Some("user:label")).unwrap_err(),
            "invalid_author"
        );
        assert_eq!(
            push_qualifier(&mut parts, "author", Some(r#"user"label"#)).unwrap_err(),
            "invalid_author"
        );
    }

    #[test]
    fn serde_rejects_unknown_fields_before_egress() {
        assert_eq!(
            execute_inner(
                r#"{"query":"repo:nearai/ironclaw","extra":"ignored?"}"#,
                Some(r#"{"capability_id":"github.search_issues"}"#),
            )
            .unwrap_err(),
            "invalid_parameters"
        );
    }

    #[test]
    fn validates_static_schema_json() {
        let schema = GitHubTool::schema();
        let parsed: serde_json::Value =
            serde_json::from_str(&schema).expect("schema should be valid JSON");
        assert_eq!(parsed["type"], "object");
        assert!(parsed["oneOf"]
            .as_array()
            .is_some_and(|schemas| schemas.len() == 3));
    }

    #[test]
    fn sanitizes_host_egress_errors_without_leaking_details() {
        assert_eq!(
            sanitize_host_error("missing token ghp_secret_value"),
            "AuthRequired"
        );
        assert_eq!(
            sanitize_host_error("deadline exceeded"),
            "host_http_timeout"
        );
        assert_eq!(
            sanitize_host_error("redirect blocked"),
            "host_http_redirect_denied"
        );
        assert_eq!(
            sanitize_host_error("response body too large"),
            "host_http_body_limit"
        );
        assert_eq!(
            sanitize_host_error("host not allowed"),
            "host_http_network_denied"
        );
        assert_eq!(
            sanitize_host_error("policy_denied"),
            "host_http_network_denied"
        );
        assert_eq!(
            sanitize_host_error("connection reset with token ghp_secret_value"),
            "AuthRequired"
        );
        assert_eq!(
            sanitize_host_error("connection reset"),
            "host_http_request_failed"
        );
    }

    fn empty_search_params() -> SearchIssuesParams {
        SearchIssuesParams {
            query: None,
            repository: None,
            owner: None,
            repo: None,
            author: None,
            assignee: None,
            involves: None,
            state: None,
            issue_type: None,
            page: None,
            limit: None,
            sort: None,
            order: None,
        }
    }
}
