//! Yuque (语雀) Knowledge Management WASM Tool for IronClaw.
//!
//! Provides knowledge base listing, document listing, document reading,
//! and document creation via the Yuque API.
//!
//! # Authentication
//!
//! Store your Yuque Token:
//! `ironclaw secret set yuque_token <token>`
//!
//! Get a token at: https://www.yuque.com/settings/tokens

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://www.yuque.com/api/v2";
const MAX_RETRIES: u32 = 3;

struct YuqueTool;

impl exports::near::agent::tool::Guest for YuqueTool {
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
        "语雀 API — 知识库管理、文档列表、文档读写。\
         Authentication is handled via the 'yuque_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "groupLogin")]
    group_login: Option<String>,
    namespace: Option<String>,
    slug: Option<String>,
    title: Option<String>,
    body: Option<String>,
    format: Option<String>,
}

// --- List repos response ---
#[derive(Debug, Deserialize)]
struct ListReposResponse {
    data: Option<Vec<Repo>>,
}

#[derive(Debug, Deserialize)]
struct Repo {
    id: Option<i64>,
    name: Option<String>,
    slug: Option<String>,
    namespace: Option<String>,
    description: Option<String>,
    #[serde(rename = "type")]
    repo_type: Option<String>,
    public: Option<i32>,
    items_count: Option<i32>,
}

// --- List docs response ---
#[derive(Debug, Deserialize)]
struct ListDocsResponse {
    data: Option<Vec<Doc>>,
}

#[derive(Debug, Deserialize)]
struct Doc {
    id: Option<i64>,
    title: Option<String>,
    slug: Option<String>,
    description: Option<String>,
    word_count: Option<i64>,
    created_at: Option<String>,
    updated_at: Option<String>,
    #[serde(rename = "public")]
    is_public: Option<i32>,
}

// --- Get doc response ---
#[derive(Debug, Deserialize)]
struct GetDocResponse {
    data: Option<DocDetail>,
}

#[derive(Debug, Deserialize)]
struct DocDetail {
    id: Option<i64>,
    title: Option<String>,
    slug: Option<String>,
    body: Option<String>,
    body_html: Option<String>,
    word_count: Option<i64>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

// --- Create doc response ---
#[derive(Debug, Deserialize)]
struct CreateDocResponse {
    data: Option<DocDetail>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("yuque_token") {
        return Err(
            "Yuque token not found in secret store. Set it with: \
             ironclaw secret set yuque_token <token>. \
             Get a token at: https://www.yuque.com/settings/tokens"
                .into(),
        );
    }

    match params.action.as_str() {
        "list_repos" => action_list_repos(&params),
        "list_docs" => action_list_docs(&params),
        "get_doc" => action_get_doc(&params),
        "create_doc" => action_create_doc(&params),
        other => Err(format!(
            "Unknown action '{other}'. Valid actions: list_repos, list_docs, get_doc, create_doc"
        )),
    }
}

fn action_list_repos(params: &Params) -> Result<String, String> {
    let url = if let Some(ref group_login) = params.group_login {
        if group_login.is_empty() {
            return Err("'groupLogin' must not be empty".into());
        }
        let encoded = url_encode(group_login);
        format!("{BASE_URL}/groups/{encoded}/repos")
    } else if let Some(ref user_id) = params.user_id {
        if user_id.is_empty() {
            return Err("'userId' must not be empty".into());
        }
        let encoded = url_encode(user_id);
        format!("{BASE_URL}/users/{encoded}/repos")
    } else {
        return Err("Either 'userId' or 'groupLogin' is required for list_repos action".into());
    };

    let body = do_get(&url)?;
    let resp: ListReposResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse repos response: {e}"))?;

    let repos = resp.data.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = repos
        .into_iter()
        .filter_map(|r| {
            let name = r.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(id) = r.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(slug) = r.slug {
                entry["slug"] = serde_json::json!(slug);
            }
            if let Some(ns) = r.namespace {
                entry["namespace"] = serde_json::json!(ns);
            }
            if let Some(desc) = r.description {
                entry["description"] = serde_json::json!(desc);
            }
            if let Some(t) = r.repo_type {
                entry["type"] = serde_json::json!(t);
            }
            if let Some(p) = r.public {
                entry["public"] = serde_json::json!(p == 1);
            }
            if let Some(c) = r.items_count {
                entry["items_count"] = serde_json::json!(c);
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

fn action_list_docs(params: &Params) -> Result<String, String> {
    let namespace = params
        .namespace
        .as_deref()
        .ok_or("'namespace' is required for list_docs action")?;
    if namespace.is_empty() {
        return Err("'namespace' must not be empty".into());
    }

    let encoded = url_encode(namespace);
    let url = format!("{BASE_URL}/repos/{encoded}/docs");

    let body = do_get(&url)?;
    let resp: ListDocsResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse docs response: {e}"))?;

    let docs = resp.data.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = docs
        .into_iter()
        .filter_map(|d| {
            let title = d.title?;
            let mut entry = serde_json::json!({"title": title});
            if let Some(id) = d.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(slug) = d.slug {
                entry["slug"] = serde_json::json!(slug);
            }
            if let Some(desc) = d.description {
                entry["description"] = serde_json::json!(desc);
            }
            if let Some(wc) = d.word_count {
                entry["word_count"] = serde_json::json!(wc);
            }
            if let Some(created) = d.created_at {
                entry["created_at"] = serde_json::json!(created);
            }
            if let Some(updated) = d.updated_at {
                entry["updated_at"] = serde_json::json!(updated);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_docs",
        "namespace": namespace,
        "result_count": formatted.len(),
        "docs": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_get_doc(params: &Params) -> Result<String, String> {
    let namespace = params
        .namespace
        .as_deref()
        .ok_or("'namespace' is required for get_doc action")?;
    let slug = params
        .slug
        .as_deref()
        .ok_or("'slug' is required for get_doc action")?;
    if namespace.is_empty() {
        return Err("'namespace' must not be empty".into());
    }
    if slug.is_empty() {
        return Err("'slug' must not be empty".into());
    }

    let encoded_ns = url_encode(namespace);
    let encoded_slug = url_encode(slug);
    let url = format!("{BASE_URL}/repos/{encoded_ns}/docs/{encoded_slug}");

    let body = do_get(&url)?;
    let resp: GetDocResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse doc response: {e}"))?;

    let doc = resp.data.ok_or("No document data returned")?;

    let mut output = serde_json::json!({
        "action": "get_doc",
        "namespace": namespace,
        "slug": slug,
    });

    if let Some(title) = doc.title {
        output["title"] = serde_json::json!(title);
    }
    if let Some(body) = doc.body {
        output["body"] = serde_json::json!(body);
    }
    if let Some(body_html) = doc.body_html {
        output["body_html"] = serde_json::json!(body_html);
    }
    if let Some(wc) = doc.word_count {
        output["word_count"] = serde_json::json!(wc);
    }
    if let Some(created) = doc.created_at {
        output["created_at"] = serde_json::json!(created);
    }
    if let Some(updated) = doc.updated_at {
        output["updated_at"] = serde_json::json!(updated);
    }

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_create_doc(params: &Params) -> Result<String, String> {
    let namespace = params
        .namespace
        .as_deref()
        .ok_or("'namespace' is required for create_doc action")?;
    let title = params
        .title
        .as_deref()
        .ok_or("'title' is required for create_doc action")?;
    let body_content = params
        .body
        .as_deref()
        .ok_or("'body' is required for create_doc action")?;
    if namespace.is_empty() {
        return Err("'namespace' must not be empty".into());
    }
    if title.is_empty() {
        return Err("'title' must not be empty".into());
    }

    let format = params.format.as_deref().unwrap_or("markdown");
    if !matches!(format, "markdown" | "html" | "lake") {
        return Err(format!(
            "Invalid 'format': expected 'markdown', 'html', or 'lake', got '{format}'"
        ));
    }

    let encoded_ns = url_encode(namespace);
    let url = format!("{BASE_URL}/repos/{encoded_ns}/docs");

    let request_body = serde_json::json!({
        "title": title,
        "body": body_content,
        "format": format,
    });

    let body = do_post(&url, &request_body)?;
    let resp: CreateDocResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse create doc response: {e}"))?;

    let doc = resp.data.ok_or("No document data returned after creation")?;

    let mut output = serde_json::json!({
        "action": "create_doc",
        "namespace": namespace,
        "title": title,
    });

    if let Some(id) = doc.id {
        output["id"] = serde_json::json!(id);
    }
    if let Some(slug) = doc.slug {
        output["slug"] = serde_json::json!(slug);
    }
    if let Some(created) = doc.created_at {
        output["created_at"] = serde_json::json!(created);
    }

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_headers() -> String {
    serde_json::json!({
        "User-Agent": "IronClaw-Yuque-Tool/0.1",
        "Content-Type": "application/json",
        "Accept": "application/json"
    })
    .to_string()
}

fn do_get(url: &str) -> Result<String, String> {
    let headers = get_headers();

    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp = near::agent::host::http_request("GET", url, &headers, None, None)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Yuque API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Yuque API error (HTTP {}): {}", resp.status, body));
    }
}

fn do_post(url: &str, body: &serde_json::Value) -> Result<String, String> {
    let headers = get_headers();

    let mut attempt = 0;
    loop {
        attempt += 1;

        let body_bytes = body.to_string().into_bytes();
        let resp =
            near::agent::host::http_request("POST", url, &headers, Some(&body_bytes), None)
                .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Yuque API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Yuque API error (HTTP {}): {}", resp.status, body));
    }
}

fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            // Allow '/' for namespace paths like "user/repo"
            b'/' => {
                result.push('/');
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform",
            "enum": ["list_repos", "list_docs", "get_doc", "create_doc"]
        },
        "userId": {
            "type": "string",
            "description": "User ID or login (required for 'list_repos' if no groupLogin)"
        },
        "groupLogin": {
            "type": "string",
            "description": "Group login name (required for 'list_repos' if no userId)"
        },
        "namespace": {
            "type": "string",
            "description": "Repository namespace, e.g. 'user/repo' (required for list_docs, get_doc, create_doc)"
        },
        "slug": {
            "type": "string",
            "description": "Document slug (required for 'get_doc')"
        },
        "title": {
            "type": "string",
            "description": "Document title (required for 'create_doc')"
        },
        "body": {
            "type": "string",
            "description": "Document content (required for 'create_doc')"
        },
        "format": {
            "type": "string",
            "description": "Document format: 'markdown', 'html', or 'lake' (default 'markdown')",
            "enum": ["markdown", "html", "lake"],
            "default": "markdown"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(YuqueTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("user/repo"), "user/repo");
        assert_eq!(url_encode("hello world"), "hello%20world");
    }

    #[test]
    fn test_parse_repos_response() {
        let json = r#"{
            "data": [
                {
                    "id": 12345,
                    "name": "测试知识库",
                    "slug": "test-kb",
                    "namespace": "user/test-kb",
                    "description": "A test knowledge base",
                    "type": "Book",
                    "public": 1,
                    "items_count": 10
                }
            ]
        }"#;
        let resp: ListReposResponse = serde_json::from_str(json).unwrap();
        let repos = resp.data.unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name.as_deref(), Some("测试知识库"));
        assert_eq!(repos[0].namespace.as_deref(), Some("user/test-kb"));
        assert_eq!(repos[0].public, Some(1));
    }

    #[test]
    fn test_parse_repos_empty() {
        let json = r#"{"data": []}"#;
        let resp: ListReposResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap().is_empty());
    }

    #[test]
    fn test_parse_docs_response() {
        let json = r#"{
            "data": [
                {
                    "id": 67890,
                    "title": "测试文档",
                    "slug": "test-doc",
                    "description": "A test document",
                    "word_count": 500,
                    "created_at": "2025-01-01T00:00:00Z",
                    "updated_at": "2025-01-02T00:00:00Z",
                    "public": 1
                }
            ]
        }"#;
        let resp: ListDocsResponse = serde_json::from_str(json).unwrap();
        let docs = resp.data.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title.as_deref(), Some("测试文档"));
        assert_eq!(docs[0].slug.as_deref(), Some("test-doc"));
        assert_eq!(docs[0].word_count, Some(500));
    }

    #[test]
    fn test_parse_docs_empty() {
        let json = r#"{"data": []}"#;
        let resp: ListDocsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap().is_empty());
    }

    #[test]
    fn test_parse_doc_detail_response() {
        let json = r##"{
            "data": {
                "id": 67890,
                "title": "测试文档",
                "slug": "test-doc",
                "body": "# Hello",
                "body_html": "<h1>Hello</h1>",
                "word_count": 10,
                "created_at": "2025-01-01T00:00:00Z",
                "updated_at": "2025-01-02T00:00:00Z"
            }
        }"##;
        let resp: GetDocResponse = serde_json::from_str(json).unwrap();
        let doc = resp.data.unwrap();
        assert_eq!(doc.title.as_deref(), Some("\u{6d4b}\u{8bd5}\u{6587}\u{6863}"));
        assert_eq!(doc.body.as_deref(), Some("# Hello"));
        assert!(doc.body_html.is_some());
    }

    #[test]
    fn test_parse_create_doc_response() {
        let json = r#"{
            "data": {
                "id": 99999,
                "title": "新文档",
                "slug": "new-doc",
                "body": "content",
                "body_html": "<p>content</p>",
                "word_count": 1,
                "created_at": "2025-06-01T00:00:00Z",
                "updated_at": "2025-06-01T00:00:00Z"
            }
        }"#;
        let resp: CreateDocResponse = serde_json::from_str(json).unwrap();
        let doc = resp.data.unwrap();
        assert_eq!(doc.id, Some(99999));
        assert_eq!(doc.slug.as_deref(), Some("new-doc"));
    }
}
