//! DingTalk Todo WASM Tool for IronClaw.
//!
//! Manages todo tasks via the DingTalk Open API (钉钉待办).
//! Supports listing, creating, updating, and deleting tasks.
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

struct DingTalkTodoTool;

impl exports::near::agent::tool::Guest for DingTalkTodoTool {
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
        "Manage DingTalk todo tasks (钉钉待办). List, create, update, and delete tasks. \
         Authentication is handled via the 'dingtalk_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(rename = "unionId")]
    union_id: Option<String>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    #[serde(rename = "dueTime")]
    due_time: Option<u64>,
    done: Option<bool>,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct TaskListResponse {
    #[serde(rename = "todoCards")]
    todo_cards: Option<Vec<TodoTask>>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
    #[serde(rename = "totalCount")]
    total_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TodoTask {
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    #[serde(rename = "dueTime")]
    due_time: Option<u64>,
    done: Option<bool>,
    #[serde(rename = "createdTime")]
    created_time: Option<u64>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<u64>,
    priority: Option<u32>,
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

    let union_id = params.union_id.as_deref();

    match params.action.as_str() {
        "list_tasks" => {
            let uid = union_id.ok_or_else(|| "list_tasks 操作需要 unionId 参数".to_string())?;
            list_tasks(uid)
        }
        "create_task" => {
            let uid = union_id.ok_or_else(|| "create_task 操作需要 unionId 参数".to_string())?;
            let subject = params
                .subject
                .ok_or_else(|| "create_task 操作需要 subject 参数".to_string())?;
            create_task(uid, &subject, params.description.as_deref(), params.due_time)
        }
        "update_task" => {
            let uid = union_id.ok_or_else(|| "update_task 操作需要 unionId 参数".to_string())?;
            let task_id = params
                .task_id
                .ok_or_else(|| "update_task 操作需要 taskId 参数".to_string())?;
            update_task(uid, &task_id, params.subject.as_deref(), params.done)
        }
        "delete_task" => {
            let uid = union_id.ok_or_else(|| "delete_task 操作需要 unionId 参数".to_string())?;
            let task_id = params
                .task_id
                .ok_or_else(|| "delete_task 操作需要 taskId 参数".to_string())?;
            delete_task(uid, &task_id)
        }
        other => Err(format!(
            "未知操作: '{other}'。支持的操作: list_tasks, create_task, update_task, delete_task"
        )),
    }
}

fn list_tasks(union_id: &str) -> Result<String, String> {
    let url = format!("{API_BASE}/todo/users/{union_id}/tasks?maxResults=50");
    let body = do_request("GET", &url, None)?;
    let resp: TaskListResponse =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let tasks = resp.todo_cards.unwrap_or_default();
    let output = serde_json::json!({
        "action": "list_tasks",
        "count": tasks.len(),
        "totalCount": resp.total_count,
        "tasks": tasks.iter().map(|t| serde_json::json!({
            "taskId": t.task_id,
            "subject": t.subject,
            "description": t.description,
            "dueTime": t.due_time,
            "done": t.done,
            "priority": t.priority,
            "createdTime": t.created_time,
            "modifiedTime": t.modified_time,
        })).collect::<Vec<_>>(),
        "nextToken": resp.next_token,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn create_task(
    union_id: &str,
    subject: &str,
    description: Option<&str>,
    due_time: Option<u64>,
) -> Result<String, String> {
    let url = format!("{API_BASE}/todo/users/{union_id}/tasks");
    let mut req_body = serde_json::json!({
        "subject": subject,
    });
    if let Some(desc) = description {
        req_body["description"] = serde_json::json!(desc);
    }
    if let Some(due) = due_time {
        req_body["dueTime"] = serde_json::json!(due);
    }
    let body_bytes = req_body.to_string().into_bytes();
    let body = do_request("POST", &url, Some(&body_bytes))?;
    let resp: TodoTask =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let output = serde_json::json!({
        "action": "create_task",
        "task": {
            "taskId": resp.task_id,
            "subject": resp.subject,
            "description": resp.description,
            "dueTime": resp.due_time,
        },
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn update_task(
    union_id: &str,
    task_id: &str,
    subject: Option<&str>,
    done: Option<bool>,
) -> Result<String, String> {
    let url = format!("{API_BASE}/todo/users/{union_id}/tasks/{task_id}");
    let mut req_body = serde_json::json!({});
    if let Some(subj) = subject {
        req_body["subject"] = serde_json::json!(subj);
    }
    if let Some(d) = done {
        req_body["done"] = serde_json::json!(d);
    }
    let body_bytes = req_body.to_string().into_bytes();
    let body = do_request("PUT", &url, Some(&body_bytes))?;
    let resp: TodoTask =
        serde_json::from_str(&body).map_err(|e| format!("响应解析失败: {e}"))?;

    let output = serde_json::json!({
        "action": "update_task",
        "task": {
            "taskId": resp.task_id,
            "subject": resp.subject,
            "done": resp.done,
        },
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn delete_task(union_id: &str, task_id: &str) -> Result<String, String> {
    let url = format!("{API_BASE}/todo/users/{union_id}/tasks/{task_id}");
    do_request("DELETE", &url, None)?;

    let output = serde_json::json!({
        "action": "delete_task",
        "taskId": task_id,
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn do_request(method: &str, url: &str, body: Option<&[u8]>) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-DingTalkTodo-Tool/0.1"
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
            "description": "操作类型: list_tasks (列出待办), create_task (创建待办), update_task (更新待办), delete_task (删除待办)",
            "enum": ["list_tasks", "create_task", "update_task", "delete_task"]
        },
        "unionId": {
            "type": "string",
            "description": "用户 unionId (所有操作必填)"
        },
        "taskId": {
            "type": "string",
            "description": "待办任务 ID (update_task 和 delete_task 操作必填)"
        },
        "subject": {
            "type": "string",
            "description": "待办标题 (create_task 必填, update_task 可选)"
        },
        "description": {
            "type": "string",
            "description": "待办描述 (create_task 可选)"
        },
        "dueTime": {
            "type": "integer",
            "description": "截止时间，Unix 毫秒时间戳 (create_task 可选)"
        },
        "done": {
            "type": "boolean",
            "description": "是否完成 (update_task 可选)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(DingTalkTodoTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_list_response() {
        let json = r#"{
            "todoCards": [
                {
                    "taskId": "task_001",
                    "subject": "完成需求文档",
                    "description": "编写 Q2 需求文档",
                    "dueTime": 1735689600000,
                    "done": false,
                    "priority": 20,
                    "createdTime": 1735600000000,
                    "modifiedTime": 1735650000000
                }
            ],
            "nextToken": "page2",
            "totalCount": 15
        }"#;
        let resp: TaskListResponse = serde_json::from_str(json).unwrap();
        let tasks = resp.todo_cards.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id.as_deref(), Some("task_001"));
        assert_eq!(tasks[0].subject.as_deref(), Some("完成需求文档"));
        assert_eq!(tasks[0].done, Some(false));
        assert_eq!(tasks[0].priority, Some(20));
        assert_eq!(resp.total_count, Some(15));
    }

    #[test]
    fn test_parse_empty_task_list() {
        let json = r#"{"todoCards": [], "totalCount": 0}"#;
        let resp: TaskListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.todo_cards.unwrap().is_empty());
        assert_eq!(resp.total_count, Some(0));
    }

    #[test]
    fn test_parse_todo_task() {
        let json = r#"{
            "taskId": "task_002",
            "subject": "代码评审",
            "done": true
        }"#;
        let resp: TodoTask = serde_json::from_str(json).unwrap();
        assert_eq!(resp.task_id.as_deref(), Some("task_002"));
        assert_eq!(resp.done, Some(true));
        assert!(resp.description.is_none());
    }

    #[test]
    fn test_parse_task_with_all_fields() {
        let json = r#"{
            "taskId": "task_003",
            "subject": "部署上线",
            "description": "部署 v2.0 到生产环境",
            "dueTime": 1735776000000,
            "done": false,
            "createdTime": 1735600000000,
            "modifiedTime": 1735700000000,
            "priority": 10
        }"#;
        let resp: TodoTask = serde_json::from_str(json).unwrap();
        assert_eq!(resp.subject.as_deref(), Some("部署上线"));
        assert_eq!(resp.due_time, Some(1735776000000));
        assert_eq!(resp.priority, Some(10));
    }
}
