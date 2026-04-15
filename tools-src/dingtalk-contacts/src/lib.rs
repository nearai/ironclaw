//! DingTalk Contacts WASM Tool for IronClaw.
//!
//! Query DingTalk organizational contacts (钉钉通讯录).
//! List departments, get department details, list users, get user details.
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

const BASE_URL: &str = "https://api.dingtalk.com/v1.0/contact";
const MAX_RETRIES: u32 = 3;

struct DingTalkContactsTool;

impl exports::near::agent::tool::Guest for DingTalkContactsTool {
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
        "Query DingTalk organizational contacts (钉钉通讯录). \
         List departments, get department details, list users in departments, get user details. \
         Authentication is handled via the 'dingtalk_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    dept_id: Option<u64>,
    user_id: Option<String>,
    max_results: Option<u32>,
    next_token: Option<String>,
}

// --- DingTalk Contact API response types ---

#[derive(Debug, Deserialize)]
struct DeptListResponse {
    result: Option<Vec<DeptItem>>,
}

#[derive(Debug, Deserialize)]
struct DeptItem {
    #[serde(rename = "deptId")]
    dept_id: Option<u64>,
    name: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<u64>,
    #[serde(rename = "createDeptGroup")]
    create_dept_group: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DeptDetailResponse {
    result: Option<DeptDetail>,
}

#[derive(Debug, Deserialize)]
struct DeptDetail {
    #[serde(rename = "deptId")]
    dept_id: Option<u64>,
    name: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<u64>,
    #[serde(rename = "deptManagerUseridList")]
    dept_manager_userid_list: Option<Vec<String>>,
    #[serde(rename = "groupContainCount")]
    group_contain_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct UserListResponse {
    result: Option<UserListResult>,
}

#[derive(Debug, Deserialize)]
struct UserListResult {
    #[serde(rename = "hasMore")]
    has_more: Option<bool>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
    list: Option<Vec<UserItem>>,
}

#[derive(Debug, Deserialize)]
struct UserItem {
    #[serde(rename = "userId")]
    user_id: Option<String>,
    name: Option<String>,
    title: Option<String>,
    mobile: Option<String>,
    email: Option<String>,
    active: Option<bool>,
    #[serde(rename = "deptIdList")]
    dept_id_list: Option<Vec<u64>>,
}

#[derive(Debug, Deserialize)]
struct UserDetailResponse {
    result: Option<UserDetail>,
}

#[derive(Debug, Deserialize)]
struct UserDetail {
    #[serde(rename = "userId")]
    user_id: Option<String>,
    name: Option<String>,
    title: Option<String>,
    mobile: Option<String>,
    email: Option<String>,
    active: Option<bool>,
    #[serde(rename = "deptIdList")]
    dept_id_list: Option<Vec<u64>>,
    #[serde(rename = "hiredDate")]
    hired_date: Option<u64>,
    avatar: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("dingtalk_access_token") {
        return Err(
            "DingTalk access token not found in secret store. Set it with: \
             ironclaw secret set dingtalk_access_token <token>. \
             Get a token at: https://open.dingtalk.com/"
                .into(),
        );
    }

    match params.action.as_str() {
        "list_departments" => list_departments(&params),
        "get_department" => get_department(&params),
        "list_users" => list_users(&params),
        "get_user" => get_user(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: list_departments, get_department, list_users, get_user",
            params.action
        )),
    }
}

fn list_departments(params: &Params) -> Result<String, String> {
    let parent_dept_id = params.dept_id.unwrap_or(1);
    let url = format!("{BASE_URL}/departments?parentDeptId={parent_dept_id}");
    let resp_body = dingtalk_request("GET", &url)?;

    let resp: DeptListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let depts = resp.result.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = depts
        .into_iter()
        .filter_map(|d| {
            let name = d.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(dept_id) = d.dept_id {
                entry["dept_id"] = serde_json::json!(dept_id);
            }
            if let Some(parent_id) = d.parent_id {
                entry["parent_id"] = serde_json::json!(parent_id);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_departments",
        "parent_dept_id": parent_dept_id,
        "result_count": formatted.len(),
        "departments": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_department(params: &Params) -> Result<String, String> {
    let dept_id = params.dept_id.ok_or("'dept_id' is required for get_department")?;

    let url = format!("{BASE_URL}/departments/{dept_id}");
    let resp_body = dingtalk_request("GET", &url)?;

    let resp: DeptDetailResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let dept = resp.result.ok_or("Department not found")?;

    let output = serde_json::json!({
        "action": "get_department",
        "dept_id": dept.dept_id,
        "name": dept.name,
        "parent_id": dept.parent_id,
        "managers": dept.dept_manager_userid_list,
        "member_count": dept.group_contain_count,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_users(params: &Params) -> Result<String, String> {
    let dept_id = params.dept_id.ok_or("'dept_id' is required for list_users")?;
    let max_results = params.max_results.unwrap_or(100).clamp(1, 100);

    let mut url = format!(
        "{BASE_URL}/departments/{dept_id}/users?maxResults={max_results}"
    );
    if let Some(ref next_token) = params.next_token {
        if !next_token.is_empty() {
            url.push_str(&format!("&nextToken={next_token}"));
        }
    }

    let resp_body = dingtalk_request("GET", &url)?;
    let resp: UserListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let result = resp.result.unwrap_or(UserListResult {
        has_more: Some(false),
        next_token: None,
        list: Some(vec![]),
    });

    let users: Vec<serde_json::Value> = result
        .list
        .unwrap_or_default()
        .into_iter()
        .filter_map(|u| {
            let name = u.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(user_id) = u.user_id {
                entry["user_id"] = serde_json::json!(user_id);
            }
            if let Some(title) = u.title {
                entry["title"] = serde_json::json!(title);
            }
            if let Some(active) = u.active {
                entry["active"] = serde_json::json!(active);
            }
            if let Some(dept_ids) = u.dept_id_list {
                entry["dept_ids"] = serde_json::json!(dept_ids);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_users",
        "dept_id": dept_id,
        "has_more": result.has_more.unwrap_or(false),
        "next_token": result.next_token,
        "result_count": users.len(),
        "users": users,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_user(params: &Params) -> Result<String, String> {
    let user_id = params
        .user_id
        .as_deref()
        .ok_or("'user_id' is required for get_user")?;

    if user_id.is_empty() {
        return Err("'user_id' must not be empty".into());
    }

    let url = format!("{BASE_URL}/users/{user_id}");
    let resp_body = dingtalk_request("GET", &url)?;

    let resp: UserDetailResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let user = resp.result.ok_or("User not found")?;

    let output = serde_json::json!({
        "action": "get_user",
        "user_id": user.user_id,
        "name": user.name,
        "title": user.title,
        "mobile": user.mobile,
        "email": user.email,
        "active": user.active,
        "dept_ids": user.dept_id_list,
        "hired_date": user.hired_date,
        "avatar": user.avatar,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn dingtalk_request(method: &str, url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-DingTalkContacts-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            None,
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
                    "DingTalk API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "DingTalk API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'list_departments' (列出部门), 'get_department' (获取部门详情), 'list_users' (列出部门成员), 'get_user' (获取用户详情)",
            "enum": ["list_departments", "get_department", "list_users", "get_user"]
        },
        "dept_id": {
            "type": "integer",
            "description": "Department ID (default 1 for root, required for get_department and list_users)"
        },
        "user_id": {
            "type": "string",
            "description": "User ID (required for get_user)"
        },
        "max_results": {
            "type": "integer",
            "description": "Max users to return (1-100, default 100, for list_users)",
            "minimum": 1,
            "maximum": 100,
            "default": 100
        },
        "next_token": {
            "type": "string",
            "description": "Pagination token for next page (optional for list_users)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(DingTalkContactsTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dept_list_response() {
        let json = r#"{
            "result": [
                {
                    "deptId": 1,
                    "name": "根部门",
                    "parentId": 0,
                    "createDeptGroup": true
                },
                {
                    "deptId": 100,
                    "name": "研发部",
                    "parentId": 1,
                    "createDeptGroup": false
                }
            ]
        }"#;
        let resp: DeptListResponse = serde_json::from_str(json).unwrap();
        let depts = resp.result.unwrap();
        assert_eq!(depts.len(), 2);
        assert_eq!(depts[0].name.as_deref(), Some("根部门"));
        assert_eq!(depts[0].dept_id, Some(1));
        assert_eq!(depts[1].name.as_deref(), Some("研发部"));
        assert_eq!(depts[1].parent_id, Some(1));
    }

    #[test]
    fn test_parse_dept_detail_response() {
        let json = r#"{
            "result": {
                "deptId": 100,
                "name": "研发部",
                "parentId": 1,
                "deptManagerUseridList": ["manager001", "manager002"],
                "groupContainCount": 15
            }
        }"#;
        let resp: DeptDetailResponse = serde_json::from_str(json).unwrap();
        let dept = resp.result.unwrap();
        assert_eq!(dept.dept_id, Some(100));
        assert_eq!(dept.name.as_deref(), Some("研发部"));
        assert_eq!(dept.dept_manager_userid_list.as_ref().map(|l| l.len()), Some(2));
        assert_eq!(dept.group_contain_count, Some(15));
    }

    #[test]
    fn test_parse_user_list_response() {
        let json = r#"{
            "result": {
                "hasMore": true,
                "nextToken": "next_page_001",
                "list": [
                    {
                        "userId": "user001",
                        "name": "张三",
                        "title": "高级工程师",
                        "mobile": "13800138000",
                        "email": "zhangsan@example.com",
                        "active": true,
                        "deptIdList": [100, 200]
                    }
                ]
            }
        }"#;
        let resp: UserListResponse = serde_json::from_str(json).unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result.has_more, Some(true));
        assert_eq!(result.next_token.as_deref(), Some("next_page_001"));
        let users = result.list.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name.as_deref(), Some("张三"));
        assert_eq!(users[0].title.as_deref(), Some("高级工程师"));
        assert_eq!(users[0].active, Some(true));
    }

    #[test]
    fn test_parse_user_detail_response() {
        let json = r#"{
            "result": {
                "userId": "user001",
                "name": "张三",
                "title": "高级工程师",
                "mobile": "13800138000",
                "email": "zhangsan@example.com",
                "active": true,
                "deptIdList": [100],
                "hiredDate": 1609459200000,
                "avatar": "https://avatar.example.com/user001.png"
            }
        }"#;
        let resp: UserDetailResponse = serde_json::from_str(json).unwrap();
        let user = resp.result.unwrap();
        assert_eq!(user.user_id.as_deref(), Some("user001"));
        assert_eq!(user.name.as_deref(), Some("张三"));
        assert_eq!(user.hired_date, Some(1609459200000));
        assert_eq!(user.avatar.as_deref(), Some("https://avatar.example.com/user001.png"));
    }

    #[test]
    fn test_parse_empty_dept_list() {
        let json = r#"{"result": []}"#;
        let resp: DeptListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_empty_user_list() {
        let json = r#"{"result": {"hasMore": false, "list": []}}"#;
        let resp: UserListResponse = serde_json::from_str(json).unwrap();
        let result = resp.result.unwrap();
        assert!(result.list.unwrap().is_empty());
        assert_eq!(result.has_more, Some(false));
    }
}
