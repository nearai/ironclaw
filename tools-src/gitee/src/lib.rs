//! Gitee Code Hosting WASM Tool for IronClaw.
//!
//! Manage Gitee repositories and issues (码云). Supports listing repos,
//! getting repo details, listing/creating issues, and searching repositories.
//!
//! # Authentication
//!
//! Store your Gitee personal access token:
//! `ironclaw secret set gitee_token <token>`
//!
//! Get a token at: https://gitee.com/personal_access_tokens

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://gitee.com/api/v5";
const MAX_RETRIES: u32 = 3;

struct GiteeTool;

impl exports::near::agent::tool::Guest for GiteeTool {
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
        "Manage Gitee repositories, issues, and pull requests (码云代码托管). \
         List repos, get repo details, list and create issues, search repositories, \
         list and create pull requests. \
         Authentication is handled via the 'gitee_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    owner: Option<String>,
    repo: Option<String>,
    query: Option<String>,
    title: Option<String>,
    body: Option<String>,
    state: Option<String>,
    per_page: Option<u32>,
    head: Option<String>,
    base: Option<String>,
}

// --- Gitee API response types ---

#[derive(Debug, Deserialize)]
struct RepoItem {
    full_name: Option<String>,
    html_url: Option<String>,
    description: Option<String>,
    stargazers_count: Option<u32>,
    forks_count: Option<u32>,
    language: Option<String>,
    updated_at: Option<String>,
    #[serde(rename = "private")]
    is_private: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct IssueItem {
    number: Option<String>,
    title: Option<String>,
    state: Option<String>,
    html_url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    user: Option<UserInfo>,
    labels: Option<Vec<LabelInfo>>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LabelInfo {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateIssueResponse {
    number: Option<String>,
    title: Option<String>,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PullItem {
    number: Option<u32>,
    title: Option<String>,
    state: Option<String>,
    html_url: Option<String>,
    created_at: Option<String>,
    user: Option<UserInfo>,
    head: Option<BranchRef>,
    base: Option<BranchRef>,
}

#[derive(Debug, Deserialize)]
struct BranchRef {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatePullResponse {
    number: Option<u32>,
    title: Option<String>,
    html_url: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("gitee_token") {
        return Err(
            "Gitee token not found in secret store. Set it with: \
             ironclaw secret set gitee_token <token>. \
             Get a token at: https://gitee.com/personal_access_tokens"
                .into(),
        );
    }

    match params.action.as_str() {
        "list_repos" => list_repos(),
        "get_repo" => get_repo(&params),
        "list_issues" => list_issues(&params),
        "create_issue" => create_issue(&params),
        "search_repos" => search_repos(&params),
        "list_pulls" => list_pulls(&params),
        "create_pull" => create_pull(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: list_repos, get_repo, list_issues, create_issue, search_repos, list_pulls, create_pull",
            params.action
        )),
    }
}

fn list_repos() -> Result<String, String> {
    let url = format!("{BASE_URL}/user/repos?sort=updated&per_page=20&page=1");
    let resp_body = gitee_request("GET", &url, None)?;

    let repos: Vec<RepoItem> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let formatted: Vec<serde_json::Value> = repos
        .into_iter()
        .filter_map(|r| {
            let full_name = r.full_name?;
            let mut entry = serde_json::json!({"full_name": full_name});
            if let Some(url) = r.html_url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(desc) = r.description {
                entry["description"] = serde_json::json!(desc);
            }
            if let Some(stars) = r.stargazers_count {
                entry["stars"] = serde_json::json!(stars);
            }
            if let Some(lang) = r.language {
                entry["language"] = serde_json::json!(lang);
            }
            if let Some(private) = r.is_private {
                entry["private"] = serde_json::json!(private);
            }
            if let Some(updated) = r.updated_at {
                entry["updated_at"] = serde_json::json!(updated);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_repos",
        "result_count": formatted.len(),
        "repos": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_repo(params: &Params) -> Result<String, String> {
    let owner = params.owner.as_deref().ok_or("'owner' is required for get_repo")?;
    let repo = params.repo.as_deref().ok_or("'repo' is required for get_repo")?;

    if owner.is_empty() || repo.is_empty() {
        return Err("'owner' and 'repo' must not be empty".into());
    }

    let url = format!("{BASE_URL}/repos/{owner}/{repo}");
    let resp_body = gitee_request("GET", &url, None)?;

    let repo_info: RepoItem =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let output = serde_json::json!({
        "action": "get_repo",
        "full_name": repo_info.full_name,
        "url": repo_info.html_url,
        "description": repo_info.description,
        "stars": repo_info.stargazers_count,
        "forks": repo_info.forks_count,
        "language": repo_info.language,
        "private": repo_info.is_private,
        "updated_at": repo_info.updated_at,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_issues(params: &Params) -> Result<String, String> {
    let owner = params.owner.as_deref().ok_or("'owner' is required for list_issues")?;
    let repo = params.repo.as_deref().ok_or("'repo' is required for list_issues")?;

    if owner.is_empty() || repo.is_empty() {
        return Err("'owner' and 'repo' must not be empty".into());
    }

    let url = format!("{BASE_URL}/repos/{owner}/{repo}/issues?state=open&per_page=20");
    let resp_body = gitee_request("GET", &url, None)?;

    let issues: Vec<IssueItem> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let formatted: Vec<serde_json::Value> = issues
        .into_iter()
        .filter_map(|i| {
            let title = i.title?;
            let mut entry = serde_json::json!({"title": title});
            if let Some(number) = i.number {
                entry["number"] = serde_json::json!(number);
            }
            if let Some(state) = i.state {
                entry["state"] = serde_json::json!(state);
            }
            if let Some(url) = i.html_url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(user) = i.user {
                if let Some(login) = user.login {
                    entry["author"] = serde_json::json!(login);
                }
            }
            if let Some(created) = i.created_at {
                entry["created_at"] = serde_json::json!(created);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_issues",
        "owner": owner,
        "repo": repo,
        "result_count": formatted.len(),
        "issues": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_issue(params: &Params) -> Result<String, String> {
    let owner = params.owner.as_deref().ok_or("'owner' is required for create_issue")?;
    let repo = params.repo.as_deref().ok_or("'repo' is required for create_issue")?;
    let title = params.title.as_deref().ok_or("'title' is required for create_issue")?;

    if owner.is_empty() || repo.is_empty() {
        return Err("'owner' and 'repo' must not be empty".into());
    }
    if title.is_empty() {
        return Err("'title' must not be empty".into());
    }

    let url = format!("{BASE_URL}/repos/{owner}/{repo}/issues");
    let mut body = serde_json::json!({"title": title});
    if let Some(ref issue_body) = params.body {
        body["body"] = serde_json::json!(issue_body);
    }

    let resp_body = gitee_request("POST", &url, Some(&body))?;
    let created: CreateIssueResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let output = serde_json::json!({
        "action": "create_issue",
        "number": created.number,
        "title": created.title,
        "url": created.html_url,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn search_repos(params: &Params) -> Result<String, String> {
    let query = params.query.as_deref().ok_or("'query' is required for search_repos")?;

    if query.is_empty() {
        return Err("'query' must not be empty".into());
    }

    let encoded_query = simple_url_encode(query);
    let url = format!("{BASE_URL}/search/repositories?q={encoded_query}&per_page=20");
    let resp_body = gitee_request("GET", &url, None)?;

    let repos: Vec<RepoItem> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let formatted: Vec<serde_json::Value> = repos
        .into_iter()
        .filter_map(|r| {
            let full_name = r.full_name?;
            let mut entry = serde_json::json!({"full_name": full_name});
            if let Some(url) = r.html_url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(desc) = r.description {
                entry["description"] = serde_json::json!(desc);
            }
            if let Some(stars) = r.stargazers_count {
                entry["stars"] = serde_json::json!(stars);
            }
            if let Some(lang) = r.language {
                entry["language"] = serde_json::json!(lang);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "search_repos",
        "query": query,
        "result_count": formatted.len(),
        "repos": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_pulls(params: &Params) -> Result<String, String> {
    let owner = params.owner.as_deref().ok_or("'owner' is required for list_pulls")?;
    let repo = params.repo.as_deref().ok_or("'repo' is required for list_pulls")?;

    if owner.is_empty() || repo.is_empty() {
        return Err("'owner' and 'repo' must not be empty".into());
    }

    let state = params.state.as_deref().unwrap_or("open");
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);

    let url = format!(
        "{BASE_URL}/repos/{owner}/{repo}/pulls?state={state}&per_page={per_page}"
    );
    let resp_body = gitee_request("GET", &url, None)?;

    let pulls: Vec<PullItem> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let formatted: Vec<serde_json::Value> = pulls
        .into_iter()
        .filter_map(|p| {
            let title = p.title?;
            let mut entry = serde_json::json!({"title": title});
            if let Some(number) = p.number {
                entry["number"] = serde_json::json!(number);
            }
            if let Some(state) = p.state {
                entry["state"] = serde_json::json!(state);
            }
            if let Some(url) = p.html_url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(user) = p.user {
                if let Some(login) = user.login {
                    entry["author"] = serde_json::json!(login);
                }
            }
            if let Some(head) = p.head {
                if let Some(ref_name) = head.ref_name {
                    entry["head"] = serde_json::json!(ref_name);
                }
            }
            if let Some(base) = p.base {
                if let Some(ref_name) = base.ref_name {
                    entry["base"] = serde_json::json!(ref_name);
                }
            }
            if let Some(created) = p.created_at {
                entry["created_at"] = serde_json::json!(created);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_pulls",
        "owner": owner,
        "repo": repo,
        "result_count": formatted.len(),
        "pulls": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_pull(params: &Params) -> Result<String, String> {
    let owner = params.owner.as_deref().ok_or("'owner' is required for create_pull")?;
    let repo = params.repo.as_deref().ok_or("'repo' is required for create_pull")?;
    let title = params.title.as_deref().ok_or("'title' is required for create_pull")?;
    let head = params.head.as_deref().ok_or("'head' is required for create_pull")?;
    let base = params.base.as_deref().ok_or("'base' is required for create_pull")?;

    if owner.is_empty() || repo.is_empty() {
        return Err("'owner' and 'repo' must not be empty".into());
    }
    if title.is_empty() {
        return Err("'title' must not be empty".into());
    }
    if head.is_empty() || base.is_empty() {
        return Err("'head' and 'base' must not be empty".into());
    }

    let url = format!("{BASE_URL}/repos/{owner}/{repo}/pulls");
    let mut body = serde_json::json!({
        "title": title,
        "head": head,
        "base": base,
    });
    if let Some(ref pr_body) = params.body {
        body["body"] = serde_json::json!(pr_body);
    }

    let resp_body = gitee_request("POST", &url, Some(&body))?;
    let created: CreatePullResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let output = serde_json::json!({
        "action": "create_pull",
        "number": created.number,
        "title": created.title,
        "url": created.html_url,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn gitee_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-Gitee-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let body_bytes = body.map(|b| b.to_string().into_bytes());
        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            body_bytes.as_deref(),
            None,
        )
        .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Gitee API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Gitee API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

/// Simple URL encoding for query parameters.
fn simple_url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'list_repos' (列出仓库), 'get_repo' (获取仓库详情), 'list_issues' (列出问题), 'create_issue' (创建问题), 'search_repos' (搜索仓库), 'list_pulls' (列出PR), 'create_pull' (创建PR)",
            "enum": ["list_repos", "get_repo", "list_issues", "create_issue", "search_repos", "list_pulls", "create_pull"]
        },
        "owner": {
            "type": "string",
            "description": "Repository owner (required for get_repo, list_issues, create_issue)"
        },
        "repo": {
            "type": "string",
            "description": "Repository name (required for get_repo, list_issues, create_issue)"
        },
        "query": {
            "type": "string",
            "description": "Search query (required for search_repos)"
        },
        "title": {
            "type": "string",
            "description": "Issue title (required for create_issue)"
        },
        "body": {
            "type": "string",
            "description": "Issue/PR body/description (optional for create_issue, create_pull)"
        },
        "state": {
            "type": "string",
            "description": "PR state filter: 'open', 'closed', 'merged', 'all' (optional for list_pulls, default 'open')",
            "enum": ["open", "closed", "merged", "all"],
            "default": "open"
        },
        "per_page": {
            "type": "integer",
            "description": "Number of results per page (1-100, default 20, for list_pulls)",
            "minimum": 1,
            "maximum": 100,
            "default": 20
        },
        "head": {
            "type": "string",
            "description": "Source branch (required for create_pull)"
        },
        "base": {
            "type": "string",
            "description": "Target branch (required for create_pull)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(GiteeTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_response() {
        let json = r#"[
            {
                "full_name": "user/repo",
                "html_url": "https://gitee.com/user/repo",
                "description": "A test repo",
                "stargazers_count": 42,
                "forks_count": 5,
                "language": "Rust",
                "updated_at": "2025-01-01T00:00:00+08:00",
                "private": false
            }
        ]"#;
        let repos: Vec<RepoItem> = serde_json::from_str(json).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name.as_deref(), Some("user/repo"));
        assert_eq!(repos[0].stargazers_count, Some(42));
        assert_eq!(repos[0].language.as_deref(), Some("Rust"));
        assert_eq!(repos[0].is_private, Some(false));
    }

    #[test]
    fn test_parse_issue_response() {
        let json = r#"[
            {
                "number": "I123",
                "title": "Bug report",
                "state": "open",
                "html_url": "https://gitee.com/user/repo/issues/I123",
                "created_at": "2025-01-01T00:00:00+08:00",
                "updated_at": "2025-01-02T00:00:00+08:00",
                "user": {"login": "testuser"},
                "labels": [{"name": "bug"}]
            }
        ]"#;
        let issues: Vec<IssueItem> = serde_json::from_str(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number.as_deref(), Some("I123"));
        assert_eq!(issues[0].title.as_deref(), Some("Bug report"));
        assert_eq!(issues[0].state.as_deref(), Some("open"));
        assert_eq!(issues[0].user.as_ref().and_then(|u| u.login.as_deref()), Some("testuser"));
    }

    #[test]
    fn test_parse_create_issue_response() {
        let json = r#"{
            "number": "I456",
            "title": "New issue",
            "html_url": "https://gitee.com/user/repo/issues/I456"
        }"#;
        let resp: CreateIssueResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.number.as_deref(), Some("I456"));
        assert_eq!(resp.title.as_deref(), Some("New issue"));
    }

    #[test]
    fn test_parse_empty_repos() {
        let json = "[]";
        let repos: Vec<RepoItem> = serde_json::from_str(json).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn test_parse_pull_response() {
        let json = r#"[
            {
                "number": 42,
                "title": "Add feature X",
                "state": "open",
                "html_url": "https://gitee.com/user/repo/pulls/42",
                "created_at": "2025-01-01T00:00:00+08:00",
                "user": {"login": "testuser"},
                "head": {"ref": "feature-x", "label": "user:feature-x"},
                "base": {"ref": "main", "label": "user:main"}
            }
        ]"#;
        let pulls: Vec<PullItem> = serde_json::from_str(json).unwrap();
        assert_eq!(pulls.len(), 1);
        assert_eq!(pulls[0].number, Some(42));
        assert_eq!(pulls[0].title.as_deref(), Some("Add feature X"));
        assert_eq!(pulls[0].state.as_deref(), Some("open"));
        assert_eq!(pulls[0].head.as_ref().and_then(|h| h.ref_name.as_deref()), Some("feature-x"));
        assert_eq!(pulls[0].base.as_ref().and_then(|b| b.ref_name.as_deref()), Some("main"));
    }

    #[test]
    fn test_parse_create_pull_response() {
        let json = r#"{
            "number": 43,
            "title": "New PR",
            "html_url": "https://gitee.com/user/repo/pulls/43"
        }"#;
        let resp: CreatePullResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.number, Some(43));
        assert_eq!(resp.title.as_deref(), Some("New PR"));
        assert_eq!(resp.html_url.as_deref(), Some("https://gitee.com/user/repo/pulls/43"));
    }

    #[test]
    fn test_simple_url_encode() {
        assert_eq!(simple_url_encode("hello world"), "hello%20world");
        assert_eq!(simple_url_encode("rust-lang"), "rust-lang");
        assert_eq!(simple_url_encode("a&b=c"), "a%26b%3Dc");
    }
}
