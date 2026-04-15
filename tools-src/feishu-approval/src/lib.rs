//! Feishu/Lark Approval WASM Tool for IronClaw.
//!
//! Create, get, and list approval workflow instances using the Feishu Open API.
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

struct FeishuApprovalTool;

impl exports::near::agent::tool::Guest for FeishuApprovalTool {
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
        "Manage Feishu/Lark approval instances (飞书审批). \
         Create, get, and list approval workflow instances. \
         Authentication is handled via the 'feishu_access_token' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    instance_id: Option<String>,
    approval_code: Option<String>,
    user_id: Option<String>,
    form: Option<String>,
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
struct CreateInstanceData {
    instance_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetInstanceData {
    approval_name: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    user_id: Option<String>,
    status: Option<String>,
    form: Option<String>,
    serial_number: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListInstancesData {
    #[serde(default)]
    instance_code_list: Vec<String>,
    has_more: Option<bool>,
    page_token: Option<String>,
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
        "create_instance" => create_instance(&params),
        "get_instance" => get_instance(&params),
        "list_instances" => list_instances(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: create_instance, get_instance, list_instances",
            params.action
        )),
    }
}

fn create_instance(params: &Params) -> Result<String, String> {
    let approval_code = params
        .approval_code
        .as_deref()
        .ok_or("'approval_code' is required for create_instance")?;
    let user_id = params
        .user_id
        .as_deref()
        .ok_or("'user_id' is required for create_instance")?;
    let form = params
        .form
        .as_deref()
        .ok_or("'form' is required for create_instance")?;

    if approval_code.is_empty() {
        return Err("'approval_code' must not be empty".into());
    }
    if user_id.is_empty() {
        return Err("'user_id' must not be empty".into());
    }

    let url = format!("{BASE_URL}/open-apis/approval/v4/instances");
    let body = serde_json::json!({
        "approval_code": approval_code,
        "user_id": user_id,
        "form": form,
    });

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<CreateInstanceData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let instance_code = resp.data.and_then(|d| d.instance_code);
    let output = serde_json::json!({
        "action": "create_instance",
        "instance_code": instance_code,
        "approval_code": approval_code,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_instance(params: &Params) -> Result<String, String> {
    let instance_id = params
        .instance_id
        .as_deref()
        .ok_or("'instance_id' is required for get_instance")?;

    if instance_id.is_empty() {
        return Err("'instance_id' must not be empty".into());
    }

    let url = format!("{BASE_URL}/open-apis/approval/v4/instances/{instance_id}");

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<GetInstanceData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data;
    let output = serde_json::json!({
        "action": "get_instance",
        "instance_id": instance_id,
        "approval_name": data.as_ref().and_then(|d| d.approval_name.as_deref()),
        "status": data.as_ref().and_then(|d| d.status.as_deref()),
        "user_id": data.as_ref().and_then(|d| d.user_id.as_deref()),
        "serial_number": data.as_ref().and_then(|d| d.serial_number.as_deref()),
        "start_time": data.as_ref().and_then(|d| d.start_time.as_deref()),
        "end_time": data.as_ref().and_then(|d| d.end_time.as_deref()),
        "form": data.as_ref().and_then(|d| d.form.as_deref()),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_instances(params: &Params) -> Result<String, String> {
    let approval_code = params
        .approval_code
        .as_deref()
        .ok_or("'approval_code' is required for list_instances")?;

    if approval_code.is_empty() {
        return Err("'approval_code' must not be empty".into());
    }

    let page_size = params.page_size.unwrap_or(20).clamp(1, 100);
    let mut url = format!(
        "{BASE_URL}/open-apis/approval/v4/instances?approval_code={approval_code}&page_size={page_size}"
    );
    if let Some(ref pt) = params.page_token {
        url.push_str(&format!("&page_token={pt}"));
    }

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<ListInstancesData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(ListInstancesData {
        instance_code_list: vec![],
        has_more: Some(false),
        page_token: None,
    });

    let output = serde_json::json!({
        "action": "list_instances",
        "approval_code": approval_code,
        "instance_count": data.instance_code_list.len(),
        "has_more": data.has_more.unwrap_or(false),
        "page_token": data.page_token,
        "instance_codes": data.instance_code_list,
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
            "description": "The action to perform: 'create_instance' (创建审批实例), 'get_instance' (查询审批实例), 'list_instances' (列出审批实例)",
            "enum": ["create_instance", "get_instance", "list_instances"]
        },
        "instance_id": {
            "type": "string",
            "description": "Approval instance ID (required for get_instance)"
        },
        "approval_code": {
            "type": "string",
            "description": "Approval definition code (required for create_instance and list_instances)"
        },
        "user_id": {
            "type": "string",
            "description": "Initiator user ID (required for create_instance)"
        },
        "form": {
            "type": "string",
            "description": "JSON string of form field values (required for create_instance)"
        },
        "page_size": {
            "type": "integer",
            "description": "Number of instances per page (1-100, default 20, for list_instances)",
            "minimum": 1,
            "maximum": 100,
            "default": 20
        },
        "page_token": {
            "type": "string",
            "description": "Pagination token for next page (for list_instances)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuApprovalTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_create_instance_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "instance_code": "INST-001-ABC"
            }
        }"#;
        let resp: FeishuResponse<CreateInstanceData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(
            resp.data.unwrap().instance_code.as_deref(),
            Some("INST-001-ABC")
        );
    }

    #[test]
    fn test_parse_get_instance_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "approval_name": "请假审批",
                "start_time": "1700000000000",
                "end_time": "1700003600000",
                "user_id": "ou_abc123",
                "status": "APPROVED",
                "form": "[{\"id\":\"widget1\",\"value\":\"3天\"}]",
                "serial_number": "202401010001"
            }
        }"#;
        let resp: FeishuResponse<GetInstanceData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.approval_name.as_deref(), Some("请假审批"));
        assert_eq!(data.status.as_deref(), Some("APPROVED"));
        assert_eq!(data.user_id.as_deref(), Some("ou_abc123"));
        assert_eq!(data.serial_number.as_deref(), Some("202401010001"));
    }

    #[test]
    fn test_parse_list_instances_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "instance_code_list": ["INST-001", "INST-002", "INST-003"],
                "has_more": true,
                "page_token": "INST-003"
            }
        }"#;
        let resp: FeishuResponse<ListInstancesData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.instance_code_list.len(), 3);
        assert_eq!(data.has_more, Some(true));
        assert_eq!(data.page_token.as_deref(), Some("INST-003"));
    }

    #[test]
    fn test_parse_empty_list_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {"instance_code_list": [], "has_more": false}}"#;
        let resp: FeishuResponse<ListInstancesData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!(resp.data.unwrap().instance_code_list.is_empty());
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<CreateInstanceData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
        assert!(resp.data.is_none());
    }
}
