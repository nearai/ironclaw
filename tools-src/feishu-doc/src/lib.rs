//! Feishu/Lark Document WASM Tool for IronClaw.
//!
//! Search, read, and create documents using the Feishu Open API.
//!
//! # Authentication
//!
//! Store your Feishu tenant_access_token:
//! `ironclaw secret set feishu_access_token <token>`
//!
//! Get a token at: https://open.feishu.cn/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://open.feishu.cn";
const MAX_RETRIES: u32 = 3;

struct FeishuDocTool;

impl exports::near::agent::tool::Guest for FeishuDocTool {
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
        "Search, read, and create Feishu/Lark documents (飞书文档). \
         Supports drive file search, document raw content retrieval, \
         and new document creation. Authentication is handled via the \
         'feishu_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    search_key: Option<String>,
    count: Option<u32>,
    document_id: Option<String>,
    folder_token: Option<String>,
    title: Option<String>,
}

// --- Feishu API response types ---

#[derive(Debug, Deserialize)]
struct FeishuResponse<T> {
    code: i32,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    #[serde(default)]
    files: Vec<FileItem>,
    has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FileItem {
    token: Option<String>,
    name: Option<String>,
    #[serde(rename = "type")]
    file_type: Option<String>,
    url: Option<String>,
    owner_id: Option<String>,
    create_time: Option<String>,
    edit_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DocContentData {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateDocData {
    document: Option<DocumentInfo>,
}

#[derive(Debug, Deserialize)]
struct DocumentInfo {
    document_id: Option<String>,
    title: Option<String>,
    revision_id: Option<i64>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("feishu_access_token") {
        return Err(
            "Feishu access token not found in secret store. Set it with: \
             ironclaw secret set feishu_access_token <token>. \
             Get a token at: https://open.feishu.cn/"
                .into(),
        );
    }

    match params.action.as_str() {
        "search_docs" => search_docs(&params),
        "get_doc" => get_doc(&params),
        "create_doc" => create_doc(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: search_docs, get_doc, create_doc",
            params.action
        )),
    }
}

fn search_docs(params: &Params) -> Result<String, String> {
    let search_key = params
        .search_key
        .as_deref()
        .ok_or("'search_key' is required for search_docs")?;

    if search_key.is_empty() {
        return Err("'search_key' must not be empty".into());
    }

    let count = params.count.unwrap_or(10).clamp(1, 50);
    let url = format!("{BASE_URL}/open-apis/drive/v1/files/search");
    let body = serde_json::json!({
        "search_key": search_key,
        "count": count,
    });

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<SearchData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(SearchData {
        files: vec![],
        has_more: Some(false),
    });

    let files: Vec<serde_json::Value> = data
        .files
        .into_iter()
        .filter_map(|f| {
            let name = f.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(token) = f.token {
                entry["token"] = serde_json::json!(token);
            }
            if let Some(ft) = f.file_type {
                entry["type"] = serde_json::json!(ft);
            }
            if let Some(url) = f.url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(t) = f.edit_time {
                entry["edit_time"] = serde_json::json!(t);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "search_docs",
        "search_key": search_key,
        "result_count": files.len(),
        "has_more": data.has_more.unwrap_or(false),
        "files": files,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_doc(params: &Params) -> Result<String, String> {
    let document_id = params
        .document_id
        .as_deref()
        .ok_or("'document_id' is required for get_doc")?;

    if document_id.is_empty() {
        return Err("'document_id' must not be empty".into());
    }

    let url = format!(
        "{BASE_URL}/open-apis/docx/v1/documents/{document_id}/raw_content"
    );

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<DocContentData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let content = resp
        .data
        .and_then(|d| d.content)
        .unwrap_or_default();

    let output = serde_json::json!({
        "action": "get_doc",
        "document_id": document_id,
        "content": content,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_doc(params: &Params) -> Result<String, String> {
    let title = params
        .title
        .as_deref()
        .ok_or("'title' is required for create_doc")?;

    if title.is_empty() {
        return Err("'title' must not be empty".into());
    }

    let url = format!("{BASE_URL}/open-apis/docx/v1/documents");
    let mut body = serde_json::json!({"title": title});
    if let Some(ref folder_token) = params.folder_token {
        body["folder_token"] = serde_json::json!(folder_token);
    }

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<CreateDocData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let doc = resp.data.and_then(|d| d.document);
    let output = serde_json::json!({
        "action": "create_doc",
        "document_id": doc.as_ref().and_then(|d| d.document_id.as_deref()),
        "title": doc.as_ref().and_then(|d| d.title.as_deref()),
        "revision_id": doc.as_ref().and_then(|d| d.revision_id),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn feishu_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-Feishu-Tool/0.1"
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
                    "Feishu API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Feishu API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'search_docs' (搜索文档), 'get_doc' (获取文档内容), 'create_doc' (创建文档)",
            "enum": ["search_docs", "get_doc", "create_doc"]
        },
        "search_key": {
            "type": "string",
            "description": "Search keyword (required for search_docs)"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-50, default 10, for search_docs)",
            "minimum": 1,
            "maximum": 50,
            "default": 10
        },
        "document_id": {
            "type": "string",
            "description": "Document ID (required for get_doc)"
        },
        "folder_token": {
            "type": "string",
            "description": "Folder token to create document in (optional for create_doc)"
        },
        "title": {
            "type": "string",
            "description": "Document title (required for create_doc)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuDocTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_search_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "files": [
                    {
                        "token": "doccnXYZ123",
                        "name": "项目计划",
                        "type": "docx",
                        "url": "https://example.feishu.cn/docx/doccnXYZ123",
                        "owner_id": "ou_abc",
                        "create_time": "1700000000",
                        "edit_time": "1700001000"
                    }
                ],
                "has_more": false
            }
        }"#;
        let resp: FeishuResponse<SearchData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].name.as_deref(), Some("项目计划"));
        assert_eq!(data.files[0].token.as_deref(), Some("doccnXYZ123"));
        assert_eq!(data.files[0].file_type.as_deref(), Some("docx"));
        assert_eq!(data.has_more, Some(false));
    }

    #[test]
    fn test_parse_empty_search_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {"files": [], "has_more": false}}"#;
        let resp: FeishuResponse<SearchData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!(resp.data.unwrap().files.is_empty());
    }

    #[test]
    fn test_parse_doc_content_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "content": "这是一个文档的内容"
            }
        }"#;
        let resp: FeishuResponse<DocContentData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(
            resp.data.unwrap().content.as_deref(),
            Some("这是一个文档的内容")
        );
    }

    #[test]
    fn test_parse_create_doc_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "document": {
                    "document_id": "doccnNewDoc123",
                    "title": "新文档",
                    "revision_id": 1
                }
            }
        }"#;
        let resp: FeishuResponse<CreateDocData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let doc = resp.data.unwrap().document.unwrap();
        assert_eq!(doc.document_id.as_deref(), Some("doccnNewDoc123"));
        assert_eq!(doc.title.as_deref(), Some("新文档"));
        assert_eq!(doc.revision_id, Some(1));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<SearchData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
        assert!(resp.data.is_none());
    }
}
