//! Feishu/Lark Task WASM Tool for IronClaw.
//!
//! Create, list, and complete tasks using the Feishu Open API.
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

struct FeishuTaskTool;

impl exports::near::agent::tool::Guest for FeishuTaskTool {
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
        "Manage Feishu/Lark tasks (飞书任务). \
         Create, list, and complete tasks. \
         Authentication is handled via the 'feishu_access_token' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    task_id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    due: Option<DueSpec>,
    completed_at: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DueSpec {
    timestamp: Option<String>,
    timezone: Option<String>,
}

// --- Feishu API response types ---

#[derive(Debug, Deserialize)]
struct FeishuResponse<T> {
    code: i32,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct CreateTaskData {
    task: Option<TaskItem>,
}

#[derive(Debug, Deserialize)]
struct ListTasksData {
    #[serde(default)]
    items: Vec<TaskItem>,
    has_more: Option<bool>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompleteTaskData {
    task: Option<TaskItem>,
}

#[derive(Debug, Deserialize)]
struct TaskItem {
    guid: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    completed_at: Option<String>,
    due: Option<TaskDue>,
    creator: Option<serde_json::Value>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskDue {
    timestamp: Option<String>,
    timezone: Option<String>,
    is_all_day: Option<bool>,
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
        "create_task" => create_task(&params),
        "list_tasks" => list_tasks(&params),
        "complete_task" => complete_task(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: create_task, list_tasks, complete_task",
            params.action
        )),
    }
}

fn create_task(params: &Params) -> Result<String, String> {
    let summary = params
        .summary
        .as_deref()
        .ok_or("'summary' is required for create_task")?;

    if summary.is_empty() {
        return Err("'summary' must not be empty".into());
    }

    let url = format!("{BASE_URL}/open-apis/task/v2/tasks");

    let mut body = serde_json::json!({"summary": summary});
    if let Some(ref desc) = params.description {
        body["description"] = serde_json::json!(desc);
    }
    if let Some(ref due) = params.due {
        let mut due_obj = serde_json::json!({});
        if let Some(ref ts) = due.timestamp {
            due_obj["timestamp"] = serde_json::json!(ts);
        }
        if let Some(ref tz) = due.timezone {
            due_obj["timezone"] = serde_json::json!(tz);
        }
        body["due"] = due_obj;
    }

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<CreateTaskData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let task = resp.data.and_then(|d| d.task);
    let output = serde_json::json!({
        "action": "create_task",
        "task_id": task.as_ref().and_then(|t| t.guid.as_deref()),
        "summary": task.as_ref().and_then(|t| t.summary.as_deref()),
        "created_at": task.as_ref().and_then(|t| t.created_at.as_deref()),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_tasks(params: &Params) -> Result<String, String> {
    let page_size = params.page_size.unwrap_or(50).clamp(1, 100);

    let mut url = format!("{BASE_URL}/open-apis/task/v2/tasks?page_size={page_size}");
    if let Some(ref pt) = params.page_token {
        url.push_str(&format!("&page_token={pt}"));
    }

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<ListTasksData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(ListTasksData {
        items: vec![],
        has_more: Some(false),
        page_token: None,
    });

    let tasks: Vec<serde_json::Value> = data
        .items
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "task_id": t.guid,
                "summary": t.summary,
                "description": t.description,
                "completed_at": t.completed_at,
                "due": t.due.map(|d| serde_json::json!({"timestamp": d.timestamp, "timezone": d.timezone, "is_all_day": d.is_all_day})),
                "created_at": t.created_at,
            })
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_tasks",
        "has_more": data.has_more.unwrap_or(false),
        "page_token": data.page_token,
        "task_count": tasks.len(),
        "tasks": tasks,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn complete_task(params: &Params) -> Result<String, String> {
    let task_id = params
        .task_id
        .as_deref()
        .ok_or("'task_id' is required for complete_task")?;

    if task_id.is_empty() {
        return Err("'task_id' must not be empty".into());
    }

    let completed_at = params
        .completed_at
        .as_deref()
        .ok_or("'completed_at' is required for complete_task (Unix timestamp in seconds)")?;

    let url = format!("{BASE_URL}/open-apis/task/v2/tasks/{task_id}");
    let body = serde_json::json!({
        "task": {
            "completed_at": completed_at,
        },
    });

    let resp_body = feishu_request("PATCH", &url, Some(&body))?;
    let resp: FeishuResponse<CompleteTaskData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let task = resp.data.and_then(|d| d.task);
    let output = serde_json::json!({
        "action": "complete_task",
        "task_id": task_id,
        "completed_at": task.as_ref().and_then(|t| t.completed_at.as_deref()).unwrap_or(completed_at),
        "summary": task.as_ref().and_then(|t| t.summary.as_deref()),
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
            "description": "The action to perform: 'create_task' (创建任务), 'list_tasks' (列出任务), 'complete_task' (完成任务)",
            "enum": ["create_task", "list_tasks", "complete_task"]
        },
        "task_id": {
            "type": "string",
            "description": "Task ID / GUID (required for complete_task)"
        },
        "summary": {
            "type": "string",
            "description": "Task title/summary (required for create_task)"
        },
        "description": {
            "type": "string",
            "description": "Task description (optional for create_task)"
        },
        "due": {
            "type": "object",
            "description": "Task due date (optional for create_task)",
            "properties": {
                "timestamp": {
                    "type": "string",
                    "description": "Unix timestamp in seconds"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone (e.g. 'Asia/Shanghai')"
                }
            }
        },
        "completed_at": {
            "type": "string",
            "description": "Unix timestamp marking task completion (required for complete_task)"
        },
        "page_size": {
            "type": "integer",
            "description": "Number of tasks per page (1-100, default 50, for list_tasks)",
            "minimum": 1,
            "maximum": 100,
            "default": 50
        },
        "page_token": {
            "type": "string",
            "description": "Pagination token for next page (for list_tasks)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuTaskTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_create_task_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "task": {
                    "guid": "task_abc123",
                    "summary": "完成周报",
                    "description": "提交本周工作总结",
                    "completed_at": "",
                    "due": {
                        "timestamp": "1700100000",
                        "timezone": "Asia/Shanghai",
                        "is_all_day": false
                    },
                    "created_at": "1700000000",
                    "updated_at": "1700000000"
                }
            }
        }"#;
        let resp: FeishuResponse<CreateTaskData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let task = resp.data.unwrap().task.unwrap();
        assert_eq!(task.guid.as_deref(), Some("task_abc123"));
        assert_eq!(task.summary.as_deref(), Some("完成周报"));
        assert_eq!(task.description.as_deref(), Some("提交本周工作总结"));
        let due = task.due.unwrap();
        assert_eq!(due.timestamp.as_deref(), Some("1700100000"));
        assert_eq!(due.is_all_day, Some(false));
    }

    #[test]
    fn test_parse_list_tasks_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "items": [
                    {
                        "guid": "task_001",
                        "summary": "任务一",
                        "completed_at": ""
                    },
                    {
                        "guid": "task_002",
                        "summary": "任务二",
                        "completed_at": "1700050000"
                    }
                ],
                "has_more": false,
                "page_token": null
            }
        }"#;
        let resp: FeishuResponse<ListTasksData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].guid.as_deref(), Some("task_001"));
        assert_eq!(data.items[1].completed_at.as_deref(), Some("1700050000"));
        assert_eq!(data.has_more, Some(false));
    }

    #[test]
    fn test_parse_empty_list_response() {
        let json = r#"{"code": 0, "msg": "success", "data": {"items": [], "has_more": false}}"#;
        let resp: FeishuResponse<ListTasksData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!(resp.data.unwrap().items.is_empty());
    }

    #[test]
    fn test_parse_complete_task_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "task": {
                    "guid": "task_abc123",
                    "summary": "已完成的任务",
                    "completed_at": "1700099999"
                }
            }
        }"#;
        let resp: FeishuResponse<CompleteTaskData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let task = resp.data.unwrap().task.unwrap();
        assert_eq!(task.guid.as_deref(), Some("task_abc123"));
        assert_eq!(task.completed_at.as_deref(), Some("1700099999"));
    }

    #[test]
    fn test_parse_due_spec() {
        let json = r#"{"timestamp": "1700100000", "timezone": "Asia/Shanghai"}"#;
        let due: DueSpec = serde_json::from_str(json).unwrap();
        assert_eq!(due.timestamp.as_deref(), Some("1700100000"));
        assert_eq!(due.timezone.as_deref(), Some("Asia/Shanghai"));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<CreateTaskData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
        assert!(resp.data.is_none());
    }
}
