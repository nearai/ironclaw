//! DingTalk Document Management WASM Tool for IronClaw.
//!
//! Manages documents and drive files via the DingTalk Open API (钉钉文档).
//! Supports listing spaces, browsing files, and getting file details.
//!
//! # Authentication
//!
//! Store your DingTalk access token:
//! `ironclaw secret set dingtalk_access_token <token>`
//!
//! Get a token at: https://open.dingtalk.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const API_BASE: &str = "https://api.dingtalk.com/v1.0";
const MAX_RETRIES: u32 = 3;

struct DingTalkDocTool;

impl exports::near::agent::tool::Guest for DingTalkDocTool {
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
        "Manage DingTalk documents and drive files (钉钉文档). List document spaces, browse \
         files in a space, and get file details. Authentication is handled via the \
         'dingtalk_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(rename = "spaceId")]
    space_id: Option<String>,
    #[serde(rename = "fileId")]
    file_id: Option<String>,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct SpaceListResponse {
    spaces: Option<Vec<Space>>,
}

#[derive(Debug, Deserialize)]
struct Space {
    #[serde(rename = "spaceId")]
    space_id: Option<String>,
    #[serde(rename = "spaceName")]
    space_name: Option<String>,
    #[serde(rename = "spaceType")]
    space_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileListResponse {
    files: Option<Vec<FileEntry>>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileEntry {
    #[serde(rename = "fileId")]
    file_id: Option<String>,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    #[serde(rename = "fileType")]
    file_type: Option<String>,
    #[serde(rename = "fileSize")]
    file_size: Option<u64>,
    #[serde(rename = "createTime")]
    create_time: Option<String>,
    #[serde(rename = "modifyTime")]
    modify_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileDetailResponse {
    #[serde(rename = "fileId")]
    file_id: Option<String>,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    #[serde(rename = "fileType")]
    file_type: Option<String>,
    #[serde(rename = "fileSize")]
    file_size: Option<u64>,
    #[serde(rename = "createTime")]
    create_time: Option<String>,
    #[serde(rename = "modifyTime")]
    modify_time: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("参数解析失败: {e}"))?;

    if !near::agent::host::secret_exists("dingtalk_access_token") {
        return Err(
            "未找到钉钉 access_token。请使用 ironclaw secret set dingtalk_access_token <token> 设置。\
             获取方式: https://open.dingtalk.com/"
                .into(),
        );
    }

    match params.action.as_str() {
        "list_spaces" => list_spaces(),
        "list_files" => {
            let space_id = params
                .space_id
                .ok_or_else(|| "list_files 操作需要 spaceId 参数".to_string())?;
            list_files(&space_id)
        }
        "get_file" => {
            let space_id = params
                .space_id
                .ok_or_else(|| "get_file 操作需要 spaceId 参数".to_string())?;
            let file_id = params
                .file_id
                .ok_or_else(|| "get_file 操作需要 fileId 参数".to_string())?;
            get_file(&space_id, &file_id)
        }
        other => Err(format!(
            "未知操作: '{other}'。支持的操作: list_spaces, list_files, get_file"
        )),
    }
}

fn list_spaces() -> Result<String, String> {
    let url = format!("{API_BASE}/drive/spaces");
    let body = do_request("GET", &url, None)?;
    let resp: SpaceListResponse =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let spaces = resp.spaces.unwrap_or_default();
    let output = serde_json::json!({
        "action": "list_spaces",
        "count": spaces.len(),
        "spaces": spaces.iter().map(|s| serde_json::json!({
            "spaceId": s.space_id,
            "spaceName": s.space_name,
            "spaceType": s.space_type,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn list_files(space_id: &str) -> Result<String, String> {
    let url = format!("{API_BASE}/drive/spaces/{space_id}/files?maxResults=50");
    let body = do_request("GET", &url, None)?;
    let resp: FileListResponse =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let files = resp.files.unwrap_or_default();
    let output = serde_json::json!({
        "action": "list_files",
        "spaceId": space_id,
        "count": files.len(),
        "files": files.iter().map(|f| serde_json::json!({
            "fileId": f.file_id,
            "fileName": f.file_name,
            "fileType": f.file_type,
            "fileSize": f.file_size,
            "createTime": f.create_time,
            "modifyTime": f.modify_time,
        })).collect::<Vec<_>>(),
        "nextToken": resp.next_token,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn get_file(space_id: &str, file_id: &str) -> Result<String, String> {
    let url = format!("{API_BASE}/drive/spaces/{space_id}/files/{file_id}");
    let body = do_request("GET", &url, None)?;
    let resp: FileDetailResponse =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let output = serde_json::json!({
        "action": "get_file",
        "file": {
            "fileId": resp.file_id,
            "fileName": resp.file_name,
            "fileType": resp.file_type,
            "fileSize": resp.file_size,
            "createTime": resp.create_time,
            "modifyTime": resp.modify_time,
        },
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn do_request(method: &str, url: &str, body: Option<&[u8]>) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-DingTalkDoc-Tool/0.1"
    });

    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = near::agent::host::http_request(
                method,
                url,
                &headers.to_string(),
                body,
                None,
            )
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "DingTalk API 错误 {} (尝试 {}/{}), 重试中...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body_str = String::from_utf8_lossy(&resp.body);
            return Err(format!("DingTalk API 错误 (HTTP {}): {}", resp.status, body_str));
        }
    };

    String::from_utf8(response.body).map_err(|e| format!("响应编码错误: {e}"))
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "操作类型: list_spaces (列出文档空间), list_files (列出空间文件), get_file (获取文件详情)",
            "enum": ["list_spaces", "list_files", "get_file"]
        },
        "spaceId": {
            "type": "string",
            "description": "文档空间 ID (list_files 和 get_file 操作必填)"
        },
        "fileId": {
            "type": "string",
            "description": "文件 ID (get_file 操作必填)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(DingTalkDocTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_space_list_response() {
        let json = r#"{
            "spaces": [
                {
                    "spaceId": "sp_001",
                    "spaceName": "团队空间",
                    "spaceType": "org"
                }
            ]
        }"#;
        let resp: SpaceListResponse = serde_json::from_str(json).unwrap();
        let spaces = resp.spaces.unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].space_id.as_deref(), Some("sp_001"));
        assert_eq!(spaces[0].space_name.as_deref(), Some("团队空间"));
    }

    #[test]
    fn test_parse_file_list_response() {
        let json = r#"{
            "files": [
                {
                    "fileId": "f_001",
                    "fileName": "设计文档.docx",
                    "fileType": "file",
                    "fileSize": 1024,
                    "createTime": "2025-01-01T00:00:00Z",
                    "modifyTime": "2025-01-02T00:00:00Z"
                }
            ],
            "nextToken": "abc123"
        }"#;
        let resp: FileListResponse = serde_json::from_str(json).unwrap();
        let files = resp.files.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name.as_deref(), Some("设计文档.docx"));
        assert_eq!(files[0].file_size, Some(1024));
        assert_eq!(resp.next_token.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_parse_file_detail_response() {
        let json = r#"{
            "fileId": "f_001",
            "fileName": "README.md",
            "fileType": "file",
            "fileSize": 512
        }"#;
        let resp: FileDetailResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.file_id.as_deref(), Some("f_001"));
        assert_eq!(resp.file_name.as_deref(), Some("README.md"));
    }

    #[test]
    fn test_parse_empty_spaces() {
        let json = r#"{"spaces": []}"#;
        let resp: SpaceListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.spaces.unwrap().is_empty());
    }

    #[test]
    fn test_parse_missing_optional_fields() {
        let json = r#"{"spaces": [{"spaceId": "sp_001"}]}"#;
        let resp: SpaceListResponse = serde_json::from_str(json).unwrap();
        let spaces = resp.spaces.unwrap();
        assert_eq!(spaces[0].space_name, None);
        assert_eq!(spaces[0].space_type, None);
    }
}
