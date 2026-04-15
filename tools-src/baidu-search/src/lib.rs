//! Baidu Search (via SerpAPI) WASM Tool for IronClaw.
//!
//! Searches the web using Baidu via SerpAPI and returns structured results.
//! Optimized for Chinese-language content.
//!
//! # Authentication
//!
//! Store your SerpAPI key:
//! `ironclaw secret set serpapi_key <key>`
//!
//! Get a key at: https://serpapi.com/manage-api-key

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const SERPAPI_ENDPOINT: &str = "https://serpapi.com/search";
const MAX_COUNT: u32 = 100;
const DEFAULT_COUNT: u32 = 10;
const MAX_RETRIES: u32 = 3;

struct BaiduSearchTool;

impl exports::near::agent::tool::Guest for BaiduSearchTool {
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
        "百度搜索 (via SerpAPI) — 使用 SerpAPI 接口搜索百度，返回标题、链接、摘要等结构化结果。\
         Authentication is handled via the 'serpapi_key' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    action: String,
    query: Option<String>,
    count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SerpApiResponse {
    organic_results: Option<Vec<OrganicResult>>,
    search_metadata: Option<SearchMetadata>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrganicResult {
    position: Option<u32>,
    title: Option<String>,
    link: Option<String>,
    snippet: Option<String>,
    displayed_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchMetadata {
    total_time_taken: Option<f64>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: SearchParams =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match params.action.as_str() {
        "search" => action_search(&params),
        other => Err(format!(
            "Unknown action '{other}'. Valid actions: search"
        )),
    }
}

fn action_search(params: &SearchParams) -> Result<String, String> {
    let query = params
        .query
        .as_deref()
        .ok_or("'query' is required for search action")?;
    if query.is_empty() {
        return Err("'query' must not be empty".into());
    }
    if query.len() > 2000 {
        return Err("'query' exceeds maximum length of 2000 characters".into());
    }

    if !near::agent::host::secret_exists("serpapi_key") {
        return Err(
            "SerpAPI key not found in secret store. Set it with: \
             ironclaw secret set serpapi_key <key>. \
             Get a key at: https://serpapi.com/manage-api-key"
                .into(),
        );
    }

    let count = params.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);
    let encoded_query = url_encode(query);
    let url = format!(
        "{SERPAPI_ENDPOINT}?engine=baidu&q={encoded_query}&num={count}"
    );

    let headers = serde_json::json!({
        "User-Agent": "IronClaw-BaiduSearch-Tool/0.1",
        "Accept": "application/json"
    });

    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let resp =
                near::agent::host::http_request("GET", &url, &headers.to_string(), None, None)
                    .map_err(|e| format!("HTTP request failed: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "SerpAPI error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!("SerpAPI error (HTTP {}): {}", resp.status, body));
        }
    };

    let body_str =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let serpapi_response: SerpApiResponse = serde_json::from_str(&body_str)
        .map_err(|e| format!("Failed to parse SerpAPI response: {e}"))?;

    if let Some(error) = serpapi_response.error {
        return Err(format!("SerpAPI error: {error}"));
    }

    let results = serpapi_response.organic_results.unwrap_or_default();

    let formatted: Vec<serde_json::Value> = results
        .into_iter()
        .filter_map(|r| {
            let title = r.title?;
            let link = r.link?;

            let mut entry = serde_json::json!({
                "title": title,
                "link": link,
            });

            if let Some(snippet) = r.snippet {
                entry["snippet"] = serde_json::json!(snippet);
            }
            if let Some(displayed_link) = r.displayed_link {
                entry["displayed_link"] = serde_json::json!(displayed_link);
            }
            if let Some(position) = r.position {
                entry["position"] = serde_json::json!(position);
            }

            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "query": query,
        "result_count": formatted.len(),
        "results": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform",
            "enum": ["search"]
        },
        "query": {
            "type": "string",
            "description": "The search query (搜索关键词)"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-100, default 10)",
            "minimum": 1,
            "maximum": 100,
            "default": 10
        }
    },
    "required": ["action", "query"],
    "additionalProperties": false
}"#;

export!(BaiduSearchTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("人工智能"), "%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD");
    }

    #[test]
    fn test_parse_serpapi_response() {
        let json = r#"{
            "organic_results": [
                {
                    "position": 1,
                    "title": "人工智能 - 百度百科",
                    "link": "https://baike.baidu.com/item/人工智能",
                    "snippet": "人工智能是计算机科学的一个分支...",
                    "displayed_link": "baike.baidu.com"
                }
            ],
            "search_metadata": {
                "total_time_taken": 1.23
            }
        }"#;
        let resp: SerpApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.is_none());
        let results = resp.organic_results.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title.as_deref(), Some("人工智能 - 百度百科"));
        assert_eq!(results[0].position, Some(1));
        assert_eq!(results[0].displayed_link.as_deref(), Some("baike.baidu.com"));
    }

    #[test]
    fn test_parse_serpapi_empty() {
        let json = r#"{"organic_results": [], "search_metadata": {"total_time_taken": 0.5}}"#;
        let resp: SerpApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.organic_results.unwrap().is_empty());
    }

    #[test]
    fn test_parse_serpapi_error() {
        let json = r#"{"error": "Invalid API key"}"#;
        let resp: SerpApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.as_deref(), Some("Invalid API key"));
    }

    #[test]
    fn test_parse_serpapi_no_results_field() {
        let json = r#"{"search_metadata": {"total_time_taken": 0.1}}"#;
        let resp: SerpApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.organic_results.is_none());
    }
}
