//! Feishu/Lark Bitable (多维表格) WASM Tool for IronClaw.
//!
//! List, get, and create records in Feishu Bitable tables.
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

struct FeishuSheetTool;

impl exports::near::agent::tool::Guest for FeishuSheetTool {
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
        "Manage Feishu/Lark Bitable records (飞书多维表格). \
         List, get, create, and batch create records in Bitable tables. \
         Authentication is handled via the 'feishu_access_token' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    app_token: Option<String>,
    table_id: Option<String>,
    record_id: Option<String>,
    fields: Option<serde_json::Value>,
    records: Option<Vec<serde_json::Value>>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

// --- Feishu API response types ---

#[derive(Debug, Deserialize)]
struct FeishuResponse<T> {
    code: i32,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ListRecordsData {
    #[serde(default)]
    items: Vec<RecordItem>,
    has_more: Option<bool>,
    page_token: Option<String>,
    total: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RecordItem {
    record_id: Option<String>,
    fields: Option<serde_json::Value>,
    created_by: Option<serde_json::Value>,
    last_modified_by: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SingleRecordData {
    record: Option<RecordItem>,
}

#[derive(Debug, Deserialize)]
struct CreateRecordData {
    record: Option<RecordItem>,
}

#[derive(Debug, Deserialize)]
struct BatchCreateRecordsData {
    #[serde(default)]
    records: Vec<RecordItem>,
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
        "list_records" => list_records(&params),
        "get_record" => get_record(&params),
        "create_record" => create_record(&params),
        "batch_create_records" => batch_create_records(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: list_records, get_record, create_record, batch_create_records",
            params.action
        )),
    }
}

fn require_app_and_table(params: &Params) -> Result<(&str, &str), String> {
    let app_token = params
        .app_token
        .as_deref()
        .ok_or("'app_token' is required")?;
    let table_id = params
        .table_id
        .as_deref()
        .ok_or("'table_id' is required")?;

    if app_token.is_empty() {
        return Err("'app_token' must not be empty".into());
    }
    if table_id.is_empty() {
        return Err("'table_id' must not be empty".into());
    }

    Ok((app_token, table_id))
}

fn list_records(params: &Params) -> Result<String, String> {
    let (app_token, table_id) = require_app_and_table(params)?;
    let page_size = params.page_size.unwrap_or(20).clamp(1, 500);

    let mut url = format!(
        "{BASE_URL}/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records?page_size={page_size}"
    );
    if let Some(ref pt) = params.page_token {
        url.push_str(&format!("&page_token={pt}"));
    }

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<ListRecordsData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(ListRecordsData {
        items: vec![],
        has_more: Some(false),
        page_token: None,
        total: Some(0),
    });

    let records: Vec<serde_json::Value> = data
        .items
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "record_id": r.record_id,
                "fields": r.fields,
            })
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_records",
        "app_token": app_token,
        "table_id": table_id,
        "total": data.total,
        "has_more": data.has_more.unwrap_or(false),
        "page_token": data.page_token,
        "record_count": records.len(),
        "records": records,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_record(params: &Params) -> Result<String, String> {
    let (app_token, table_id) = require_app_and_table(params)?;
    let record_id = params
        .record_id
        .as_deref()
        .ok_or("'record_id' is required for get_record")?;

    if record_id.is_empty() {
        return Err("'record_id' must not be empty".into());
    }

    let url = format!(
        "{BASE_URL}/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}"
    );

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<SingleRecordData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let record = resp.data.and_then(|d| d.record);
    let output = serde_json::json!({
        "action": "get_record",
        "app_token": app_token,
        "table_id": table_id,
        "record_id": record.as_ref().and_then(|r| r.record_id.as_deref()),
        "fields": record.as_ref().and_then(|r| r.fields.clone()),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_record(params: &Params) -> Result<String, String> {
    let (app_token, table_id) = require_app_and_table(params)?;
    let fields = params
        .fields
        .as_ref()
        .ok_or("'fields' is required for create_record")?;

    let url = format!(
        "{BASE_URL}/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records"
    );
    let body = serde_json::json!({"fields": fields});

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<CreateRecordData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let record = resp.data.and_then(|d| d.record);
    let output = serde_json::json!({
        "action": "create_record",
        "app_token": app_token,
        "table_id": table_id,
        "record_id": record.as_ref().and_then(|r| r.record_id.as_deref()),
        "fields": record.as_ref().and_then(|r| r.fields.clone()),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn batch_create_records(params: &Params) -> Result<String, String> {
    let (app_token, table_id) = require_app_and_table(params)?;
    let records = params
        .records
        .as_ref()
        .ok_or("'records' is required for batch_create_records")?;

    if records.is_empty() {
        return Err("'records' must not be empty".into());
    }

    let url = format!(
        "{BASE_URL}/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/batch_create"
    );
    let record_objects: Vec<serde_json::Value> = records
        .iter()
        .map(|fields| serde_json::json!({"fields": fields}))
        .collect();
    let body = serde_json::json!({"records": record_objects});

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<BatchCreateRecordsData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(BatchCreateRecordsData { records: vec![] });
    let created: Vec<serde_json::Value> = data
        .records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "record_id": r.record_id,
                "fields": r.fields,
            })
        })
        .collect();

    let output = serde_json::json!({
        "action": "batch_create_records",
        "app_token": app_token,
        "table_id": table_id,
        "created_count": created.len(),
        "records": created,
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
            "description": "The action to perform: 'list_records' (列出记录), 'get_record' (获取记录), 'create_record' (创建记录), 'batch_create_records' (批量创建记录)",
            "enum": ["list_records", "get_record", "create_record", "batch_create_records"]
        },
        "app_token": {
            "type": "string",
            "description": "Bitable app token (required for all actions)"
        },
        "table_id": {
            "type": "string",
            "description": "Table ID within the Bitable app (required for all actions)"
        },
        "record_id": {
            "type": "string",
            "description": "Record ID (required for get_record)"
        },
        "fields": {
            "type": "object",
            "description": "Field values for the new record (required for create_record)"
        },
        "records": {
            "type": "array",
            "description": "Array of field objects for batch creation (required for batch_create_records)",
            "items": {
                "type": "object",
                "description": "Field values for each record"
            }
        },
        "page_size": {
            "type": "integer",
            "description": "Number of records per page (1-500, default 20, for list_records)",
            "minimum": 1,
            "maximum": 500,
            "default": 20
        },
        "page_token": {
            "type": "string",
            "description": "Pagination token for next page (for list_records)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuSheetTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_list_records_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "items": [
                    {
                        "record_id": "recABC123",
                        "fields": {"名称": "测试记录", "状态": "进行中"},
                        "created_by": {"id": "ou_abc"},
                        "last_modified_by": {"id": "ou_abc"}
                    }
                ],
                "has_more": true,
                "page_token": "recABC123",
                "total": 42
            }
        }"#;
        let resp: FeishuResponse<ListRecordsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].record_id.as_deref(), Some("recABC123"));
        assert_eq!(data.total, Some(42));
        assert_eq!(data.has_more, Some(true));
        assert_eq!(data.page_token.as_deref(), Some("recABC123"));
    }

    #[test]
    fn test_parse_empty_list_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {"items": [], "has_more": false, "total": 0}}"#;
        let resp: FeishuResponse<ListRecordsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert!(data.items.is_empty());
        assert_eq!(data.total, Some(0));
    }

    #[test]
    fn test_parse_single_record_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "record": {
                    "record_id": "recXYZ789",
                    "fields": {"名称": "单条记录"}
                }
            }
        }"#;
        let resp: FeishuResponse<SingleRecordData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let record = resp.data.unwrap().record.unwrap();
        assert_eq!(record.record_id.as_deref(), Some("recXYZ789"));
    }

    #[test]
    fn test_parse_create_record_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "record": {
                    "record_id": "recNEW001",
                    "fields": {"名称": "新记录", "数量": 10}
                }
            }
        }"#;
        let resp: FeishuResponse<CreateRecordData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let record = resp.data.unwrap().record.unwrap();
        assert_eq!(record.record_id.as_deref(), Some("recNEW001"));
    }

    #[test]
    fn test_parse_batch_create_records_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "records": [
                    {
                        "record_id": "recBATCH001",
                        "fields": {"名称": "批量记录1"}
                    },
                    {
                        "record_id": "recBATCH002",
                        "fields": {"名称": "批量记录2"}
                    }
                ]
            }
        }"#;
        let resp: FeishuResponse<BatchCreateRecordsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.records[0].record_id.as_deref(), Some("recBATCH001"));
        assert_eq!(data.records[1].record_id.as_deref(), Some("recBATCH002"));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<ListRecordsData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
        assert!(resp.data.is_none());
    }
}
