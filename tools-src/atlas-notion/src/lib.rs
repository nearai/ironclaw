//! Atlas Notion Tool for IronClaw.
//! Direct Notion API integration for Atlas Second Brain.

mod types;
use types::*;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct AtlasNotionTool;

impl exports::near::agent::tool::Guest for AtlasNotionTool {
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
        r#"{
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_page", "query_database", "update_page", "get_page", "search"],
                    "description": "The Notion operation to perform"
                },
                "database_id": {
                    "type": "string",
                    "description": "Notion Database ID for query_database or create_page"
                },
                "page_id": {
                    "type": "string",
                    "description": "Notion Page ID for update_page or get_page"
                },
                "properties": {
                    "type": "object",
                    "description": "Notion properties (JSON objects) for create/update"
                },
                "filter": {
                    "type": "object",
                    "description": "Notion filter object for query_database"
                },
                "sorts": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Notion sorts array for query_database"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for search action"
                },
                "start_cursor": {
                    "type": "string",
                    "description": "Pagination cursor"
                },
                "page_size": {
                    "type": "integer",
                    "description": "Number of results to return (max 100)",
                    "default": 25
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Direct Notion API integration for Atlas Second Brain. Supports creating pages, querying databases, \
         updating pages, and searching. Requires a notion_token secret."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("notion_token") {
        return Err("notion_token secret not configured. Set it in IronClaw secrets.".to_string());
    }

    let action: AtlasNotionAction = serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    match action {
        AtlasNotionAction::CreatePage { database_id, properties } => {
            let body = serde_json::json!({
                "parent": { "database_id": database_id },
                "properties": properties
            });
            api_call("POST", "pages", Some(&body.to_string()))
        }
        AtlasNotionAction::QueryDatabase { database_id, filter, sorts, start_cursor, page_size } => {
            let mut body = serde_json::json!({
                "page_size": page_size.unwrap_or(25)
            });
            if let Some(f) = filter { body["filter"] = f; }
            if let Some(s) = sorts { body["sorts"] = s; }
            if let Some(c) = start_cursor { body["start_cursor"] = serde_json::Value::String(c); }
            
            api_call("POST", &format!("databases/{}/query", database_id), Some(&body.to_string()))
        }
        AtlasNotionAction::UpdatePage { page_id, properties } => {
            let body = serde_json::json!({ "properties": properties });
            api_call("PATCH", &format!("pages/{}", page_id), Some(&body.to_string()))
        }
        AtlasNotionAction::GetPage { page_id } => {
            api_call("GET", &format!("pages/{}", page_id), None)
        }
        AtlasNotionAction::Search { query, start_cursor, page_size } => {
            let mut body = serde_json::json!({
                "page_size": page_size.unwrap_or(25)
            });
            if let Some(q) = query { body["query"] = serde_json::Value::String(q); }
            if let Some(c) = start_cursor { body["start_cursor"] = serde_json::Value::String(c); }
            
            api_call("POST", "search", Some(&body.to_string()))
        }
    }
}

fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("https://api.notion.com/v1/{}", path);
    
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Notion-Version": "2022-06-28"
    }).to_string();

    let body_bytes = body.map(|b| b.as_bytes().to_vec());
    let response = crate::near::agent::host::http_request(method, &url, &headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!("Notion API returned status {}: {}", response.status, body_text));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

export!(AtlasNotionTool);
