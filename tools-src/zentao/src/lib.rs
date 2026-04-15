//! ZenTao Project Management WASM Tool for IronClaw.
//!
//! Manage ZenTao products, bugs, and tasks (禅道项目管理).
//! ZenTao is self-hosted, so each request requires a `base_url` parameter.
//!
//! # Authentication
//!
//! Store your ZenTao API token:
//! `ironclaw secret set zentao_token <token>`
//!
//! Get a token from: 禅道系统管理 > 二次开发 > 应用

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const MAX_RETRIES: u32 = 3;

struct ZenTaoTool;

impl exports::near::agent::tool::Guest for ZenTaoTool {
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
        "Manage ZenTao projects, bugs, and tasks (禅道项目管理). \
         List products, list and create bugs, list tasks. \
         ZenTao is self-hosted — requires base_url parameter. \
         Authentication is handled via the 'zentao_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    base_url: String,
    product: Option<u64>,
    title: Option<String>,
    severity: Option<u32>,
    steps: Option<String>,
}

// --- ZenTao API response types ---

#[derive(Debug, Deserialize)]
struct ProductListResponse {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    products: Option<Vec<ProductItem>>,
}

#[derive(Debug, Deserialize)]
struct BugListResponse {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    bugs: Option<Vec<BugItem>>,
}

#[derive(Debug, Deserialize)]
struct TaskListResponse {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    tasks: Option<Vec<TaskItem>>,
}

#[derive(Debug, Deserialize)]
struct ProductItem {
    id: Option<u64>,
    name: Option<String>,
    status: Option<String>,
    #[serde(rename = "createdDate")]
    created_date: Option<String>,
    desc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BugItem {
    id: Option<u64>,
    title: Option<String>,
    severity: Option<u32>,
    status: Option<String>,
    #[serde(rename = "openedBy")]
    opened_by: Option<String>,
    #[serde(rename = "openedDate")]
    opened_date: Option<String>,
    #[serde(rename = "assignedTo")]
    assigned_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskItem {
    id: Option<u64>,
    name: Option<String>,
    status: Option<String>,
    pri: Option<u32>,
    #[serde(rename = "assignedTo")]
    assigned_to: Option<String>,
    deadline: Option<String>,
    estimate: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CreateBugResponse {
    id: Option<u64>,
    title: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("zentao_token") {
        return Err(
            "ZenTao token not found in secret store. Set it with: \
             ironclaw secret set zentao_token <token>. \
             Get a token from: 禅道系统管理 > 二次开发 > 应用"
                .into(),
        );
    }

    if params.base_url.is_empty() {
        return Err("'base_url' is required and must not be empty".into());
    }

    let base_url = params.base_url.trim_end_matches('/');

    match params.action.as_str() {
        "list_products" => list_products(base_url),
        "list_bugs" => list_bugs(base_url),
        "create_bug" => create_bug(base_url, &params),
        "list_tasks" => list_tasks(base_url),
        _ => Err(format!(
            "Unknown action '{}'. Expected: list_products, list_bugs, create_bug, list_tasks",
            params.action
        )),
    }
}

fn list_products(base_url: &str) -> Result<String, String> {
    let url = format!("{base_url}/api.php/v1/products?limit=50");
    let resp_body = zentao_request("GET", &url, None)?;

    let resp: ProductListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let products = resp.products.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = products
        .into_iter()
        .filter_map(|p| {
            let name = p.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(id) = p.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(status) = p.status {
                entry["status"] = serde_json::json!(status);
            }
            if let Some(desc) = p.desc {
                entry["description"] = serde_json::json!(desc);
            }
            if let Some(date) = p.created_date {
                entry["created_date"] = serde_json::json!(date);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_products",
        "total": resp.total,
        "result_count": formatted.len(),
        "products": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_bugs(base_url: &str) -> Result<String, String> {
    let url = format!("{base_url}/api.php/v1/bugs?limit=50");
    let resp_body = zentao_request("GET", &url, None)?;

    let resp: BugListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let bugs = resp.bugs.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = bugs
        .into_iter()
        .filter_map(|b| {
            let title = b.title?;
            let mut entry = serde_json::json!({"title": title});
            if let Some(id) = b.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(severity) = b.severity {
                entry["severity"] = serde_json::json!(severity);
            }
            if let Some(status) = b.status {
                entry["status"] = serde_json::json!(status);
            }
            if let Some(by) = b.opened_by {
                entry["opened_by"] = serde_json::json!(by);
            }
            if let Some(to) = b.assigned_to {
                entry["assigned_to"] = serde_json::json!(to);
            }
            if let Some(date) = b.opened_date {
                entry["opened_date"] = serde_json::json!(date);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_bugs",
        "total": resp.total,
        "result_count": formatted.len(),
        "bugs": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_bug(base_url: &str, params: &Params) -> Result<String, String> {
    let product = params.product.ok_or("'product' (product ID) is required for create_bug")?;
    let title = params.title.as_deref().ok_or("'title' is required for create_bug")?;

    if title.is_empty() {
        return Err("'title' must not be empty".into());
    }

    let url = format!("{base_url}/api.php/v1/bugs");
    let mut body = serde_json::json!({
        "product": product,
        "title": title,
    });
    if let Some(severity) = params.severity {
        body["severity"] = serde_json::json!(severity);
    }
    if let Some(ref steps) = params.steps {
        body["steps"] = serde_json::json!(steps);
    }

    let resp_body = zentao_request("POST", &url, Some(&body))?;
    let created: CreateBugResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let output = serde_json::json!({
        "action": "create_bug",
        "id": created.id,
        "title": created.title,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_tasks(base_url: &str) -> Result<String, String> {
    let url = format!("{base_url}/api.php/v1/tasks?limit=50");
    let resp_body = zentao_request("GET", &url, None)?;

    let resp: TaskListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let tasks = resp.tasks.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = tasks
        .into_iter()
        .filter_map(|t| {
            let name = t.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(id) = t.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(status) = t.status {
                entry["status"] = serde_json::json!(status);
            }
            if let Some(pri) = t.pri {
                entry["priority"] = serde_json::json!(pri);
            }
            if let Some(to) = t.assigned_to {
                entry["assigned_to"] = serde_json::json!(to);
            }
            if let Some(deadline) = t.deadline {
                entry["deadline"] = serde_json::json!(deadline);
            }
            if let Some(estimate) = t.estimate {
                entry["estimate_hours"] = serde_json::json!(estimate);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_tasks",
        "total": resp.total,
        "result_count": formatted.len(),
        "tasks": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn zentao_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-ZenTao-Tool/0.1"
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
                    "ZenTao API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "ZenTao API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'list_products' (列出产品), 'list_bugs' (列出Bug), 'create_bug' (创建Bug), 'list_tasks' (列出任务)",
            "enum": ["list_products", "list_bugs", "create_bug", "list_tasks"]
        },
        "base_url": {
            "type": "string",
            "description": "ZenTao server base URL, e.g. 'https://zentao.example.com' (禅道服务器地址)"
        },
        "product": {
            "type": "integer",
            "description": "Product ID (required for create_bug)"
        },
        "title": {
            "type": "string",
            "description": "Bug title (required for create_bug)"
        },
        "severity": {
            "type": "integer",
            "description": "Bug severity level 1-4 (optional for create_bug, 1=highest)",
            "minimum": 1,
            "maximum": 4
        },
        "steps": {
            "type": "string",
            "description": "Bug reproduction steps (optional for create_bug)"
        }
    },
    "required": ["action", "base_url"],
    "additionalProperties": false
}"#;

export!(ZenTaoTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_product_list_response() {
        let json = r#"{
            "total": 2,
            "limit": 50,
            "page": 1,
            "products": [
                {
                    "id": 1,
                    "name": "核心产品",
                    "status": "normal",
                    "createdDate": "2025-01-01",
                    "desc": "核心产品描述"
                },
                {
                    "id": 2,
                    "name": "移动端",
                    "status": "normal",
                    "createdDate": "2025-02-01",
                    "desc": ""
                }
            ]
        }"#;
        let resp: ProductListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 2);
        let products = resp.products.unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].name.as_deref(), Some("核心产品"));
        assert_eq!(products[0].id, Some(1));
    }

    #[test]
    fn test_parse_bug_list_response() {
        let json = r#"{
            "total": 1,
            "limit": 50,
            "page": 1,
            "bugs": [
                {
                    "id": 100,
                    "title": "登录页面崩溃",
                    "severity": 2,
                    "status": "active",
                    "openedBy": "zhangsan",
                    "openedDate": "2025-03-01",
                    "assignedTo": "lisi"
                }
            ]
        }"#;
        let resp: BugListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 1);
        let bugs = resp.bugs.unwrap();
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].title.as_deref(), Some("登录页面崩溃"));
        assert_eq!(bugs[0].severity, Some(2));
        assert_eq!(bugs[0].assigned_to.as_deref(), Some("lisi"));
    }

    #[test]
    fn test_parse_task_list_response() {
        let json = r#"{
            "total": 1,
            "limit": 50,
            "page": 1,
            "tasks": [
                {
                    "id": 50,
                    "name": "实现用户登录",
                    "status": "doing",
                    "pri": 1,
                    "assignedTo": "wangwu",
                    "deadline": "2025-04-01",
                    "estimate": 8.0
                }
            ]
        }"#;
        let resp: TaskListResponse = serde_json::from_str(json).unwrap();
        let tasks = resp.tasks.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name.as_deref(), Some("实现用户登录"));
        assert_eq!(tasks[0].pri, Some(1));
        assert_eq!(tasks[0].estimate, Some(8.0));
    }

    #[test]
    fn test_parse_create_bug_response() {
        let json = r#"{"id": 101, "title": "新建的Bug"}"#;
        let resp: CreateBugResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(101));
        assert_eq!(resp.title.as_deref(), Some("新建的Bug"));
    }

    #[test]
    fn test_parse_empty_list_response() {
        let json = r#"{"total": 0, "products": []}"#;
        let resp: ProductListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 0);
        assert!(resp.products.unwrap().is_empty());
    }
}
