//! Atlas Brain Tool for IronClaw.
//! Read-only Notion API integration for querying the Atlas Second Brain.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "action")]
enum AtlasBrainAction {
    #[serde(rename = "query_database")]
    QueryDatabase {
        database_id: String,
        filter: Option<serde_json::Value>,
        sorts: Option<serde_json::Value>,
        start_cursor: Option<String>,
        page_size: Option<u32>,
    },
    #[serde(rename = "get_page")]
    GetPage {
        page_id: String,
    },
    #[serde(rename = "search")]
    Search {
        query: Option<String>,
        start_cursor: Option<String>,
        page_size: Option<u32>,
    },
}

struct AtlasBrainTool;

impl exports::near::agent::tool::Guest for AtlasBrainTool {
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
                    "enum": ["query_database", "get_page", "search"],
                    "description": "The read-only Notion operation to perform"
                },
                "database_id": {
                    "type": "string",
                    "description": "Notion Database ID for query_database"
                },
                "page_id": {
                    "type": "string",
                    "description": "Notion Page ID for get_page"
                },
                "filter": {
                    "type": "object",
                    "description": "Notion filter object"
                },
                "sorts": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Notion sorts array"
                },
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "start_cursor": {
                    "type": "string",
                    "description": "Pagination cursor"
                },
                "page_size": {
                    "type": "integer",
                    "default": 25
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Read-only Notion API integration for Atlas Second Brain. Answer questions like 'What are my urgent tasks?' \
         by querying Notion databases."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("notion_token") {
        return Err("notion_token secret not configured.".to_string());
    }

    let action: AtlasBrainAction = serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    match action {
        AtlasBrainAction::QueryDatabase { database_id, filter, sorts, start_cursor, page_size } => {
            let mut body = serde_json::json!({
                "page_size": page_size.unwrap_or(25)
            });
            if let Some(f) = filter { body["filter"] = f; }
            if let Some(s) = sorts { body["sorts"] = s; }
            if let Some(c) = start_cursor { body["start_cursor"] = serde_json::Value::String(c); }
            
            api_call("POST", &format!("databases/{}/query", database_id), Some(&body.to_string()))
        }
        AtlasBrainAction::GetPage { page_id } => {
            api_call("GET", &format!("pages/{}", page_id), None)
        }
        AtlasBrainAction::Search { query, start_cursor, page_size } => {
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

export!(AtlasBrainTool);
