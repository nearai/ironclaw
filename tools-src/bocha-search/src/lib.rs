//! Bocha Web Search WASM Tool for IronClaw.
//!
//! Searches the web using the Bocha Search API (博查搜索) and returns structured results.
//! Optimized for Chinese-language content and domestic Chinese internet coverage.
//!
//! # Authentication
//!
//! Store your Bocha API key:
//! `ironclaw secret set bocha_api_key <key>`
//!
//! Get a key at: https://open.bochaai.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BOCHA_SEARCH_ENDPOINT: &str = "https://api.bochaai.com/v1/web-search";
const MAX_COUNT: u32 = 50;
const DEFAULT_COUNT: u32 = 10;
const MAX_RETRIES: u32 = 3;

struct BochaSearchTool;

impl exports::near::agent::tool::Guest for BochaSearchTool {
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
        "Search the web using Bocha Search (博查搜索). Returns titles, URLs, descriptions, \
         site names, and publication dates for matching web pages. Optimized for Chinese-language \
         content and domestic Chinese internet. Supports freshness filtering. Authentication is \
         handled via the 'bocha_api_key' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    query: String,
    count: Option<u32>,
    freshness: Option<String>,
    summary: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BochaSearchResponse {
    #[serde(default)]
    code: i32,
    msg: Option<String>,
    data: Option<BochaData>,
}

#[derive(Debug, Deserialize)]
struct BochaData {
    #[serde(rename = "webPages")]
    web_pages: Option<BochaWebPages>,
}

#[derive(Debug, Deserialize)]
struct BochaWebPages {
    value: Option<Vec<BochaWebResult>>,
}

#[derive(Debug, Deserialize)]
struct BochaWebResult {
    name: Option<String>,
    url: Option<String>,
    snippet: Option<String>,
    summary: Option<String>,
    #[serde(rename = "siteName")]
    site_name: Option<String>,
    #[serde(rename = "siteIcon")]
    site_icon: Option<String>,
    #[serde(rename = "datePublished")]
    date_published: Option<String>,
    #[serde(rename = "dateLastCrawled")]
    date_last_crawled: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: SearchParams =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if params.query.is_empty() {
        return Err("'query' must not be empty".into());
    }
    if params.query.len() > 2000 {
        return Err("'query' exceeds maximum length of 2000 characters".into());
    }

    if let Some(ref freshness) = params.freshness {
        if !is_valid_freshness(freshness) {
            return Err(format!(
                "Invalid 'freshness': expected 'noLimit', 'day', 'week', 'month', or 'year', \
                 got '{freshness}'"
            ));
        }
    }

    // Pre-flight: verify API key is available.
    if !near::agent::host::secret_exists("bocha_api_key") {
        return Err(
            "Bocha API key not found in secret store. Set it with: \
             ironclaw secret set bocha_api_key <key>. \
             Get a key at: https://open.bochaai.com/"
                .into(),
        );
    }

    let count = params.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);
    let summary = params.summary.unwrap_or(false);

    let body = serde_json::json!({
        "query": params.query,
        "freshness": params.freshness.as_deref().unwrap_or("noLimit"),
        "summary": summary,
        "count": count,
    });

    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-BochaSearch-Tool/0.1"
    });

    // Retry loop for transient errors (429 rate limit, 5xx server errors).
    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let body_bytes = body.to_string().into_bytes();
            let resp = near::agent::host::http_request(
                "POST",
                BOCHA_SEARCH_ENDPOINT,
                &headers.to_string(),
                Some(&body_bytes),
                None,
            )
            .map_err(|e| format!("HTTP request failed: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "Bocha API error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "Bocha API error (HTTP {}): {}",
                resp.status, body
            ));
        }
    };

    let body_str =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let bocha_response: BochaSearchResponse =
        serde_json::from_str(&body_str).map_err(|e| format!("Failed to parse Bocha response: {e}"))?;

    if bocha_response.code != 200 && bocha_response.code != 0 {
        let msg = bocha_response.msg.unwrap_or_default();
        return Err(format!("Bocha API error (code {}): {}", bocha_response.code, msg));
    }

    let results = bocha_response
        .data
        .and_then(|d| d.web_pages)
        .and_then(|w| w.value)
        .unwrap_or_default();

    let formatted: Vec<serde_json::Value> = results
        .into_iter()
        .filter_map(|r| {
            let name = r.name?;
            let url = r.url?;

            let mut entry = serde_json::json!({
                "title": name,
                "url": url,
            });

            // Prefer summary over snippet when available.
            if let Some(summary) = r.summary.filter(|s| !s.is_empty()) {
                entry["description"] = serde_json::json!(summary);
            } else if let Some(snippet) = r.snippet {
                entry["description"] = serde_json::json!(snippet);
            }

            if let Some(site_name) = r.site_name {
                entry["site_name"] = serde_json::json!(site_name);
            } else if let Some(host) = extract_hostname(&url) {
                entry["site_name"] = serde_json::json!(host);
            }

            if let Some(icon) = r.site_icon {
                entry["site_icon"] = serde_json::json!(icon);
            }
            if let Some(date) = r.date_published {
                entry["published"] = serde_json::json!(date);
            } else if let Some(date) = r.date_last_crawled {
                entry["published"] = serde_json::json!(date);
            }

            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "query": params.query,
        "result_count": formatted.len(),
        "results": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

/// Extract hostname from a URL string without a URL parser.
fn extract_hostname(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next()?; // strip port
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Validate a freshness filter value.
fn is_valid_freshness(s: &str) -> bool {
    matches!(s, "noLimit" | "day" | "week" | "month" | "year")
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "The search query (搜索关键词)"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-50, default 10)",
            "minimum": 1,
            "maximum": 50,
            "default": 10
        },
        "freshness": {
            "type": "string",
            "description": "Filter by time range: 'noLimit' (all time), 'day' (past day), 'week' (past week), 'month' (past month), 'year' (past year)",
            "enum": ["noLimit", "day", "week", "month", "year"],
            "default": "noLimit"
        },
        "summary": {
            "type": "boolean",
            "description": "Whether to return AI-generated summaries for each result (default false)",
            "default": false
        }
    },
    "required": ["query"],
    "additionalProperties": false
}"#;

export!(BochaSearchTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_hostname() {
        assert_eq!(
            extract_hostname("https://example.com/path"),
            Some("example.com".into())
        );
        assert_eq!(
            extract_hostname("https://sub.example.com:8080/path"),
            Some("sub.example.com".into())
        );
        assert_eq!(
            extract_hostname("http://example.com"),
            Some("example.com".into())
        );
        assert_eq!(extract_hostname("not-a-url"), None);
    }

    #[test]
    fn test_extract_hostname_empty() {
        assert_eq!(extract_hostname("https://"), None);
        assert_eq!(extract_hostname("https:///path"), None);
        assert_eq!(extract_hostname(""), None);
    }

    #[test]
    fn test_is_valid_freshness() {
        assert!(is_valid_freshness("noLimit"));
        assert!(is_valid_freshness("day"));
        assert!(is_valid_freshness("week"));
        assert!(is_valid_freshness("month"));
        assert!(is_valid_freshness("year"));
        assert!(!is_valid_freshness("pd"));
        assert!(!is_valid_freshness("invalid"));
        assert!(!is_valid_freshness(""));
    }

    #[test]
    fn test_parse_bocha_response() {
        let json = r#"{
            "code": 200,
            "msg": "success",
            "data": {
                "webPages": {
                    "value": [
                        {
                            "name": "Test Page",
                            "url": "https://example.com",
                            "snippet": "A test snippet",
                            "summary": "A test summary",
                            "siteName": "Example",
                            "siteIcon": "https://example.com/icon.png",
                            "datePublished": "2025-01-01"
                        }
                    ]
                }
            }
        }"#;
        let resp: BochaSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 200);
        let pages = resp.data.unwrap().web_pages.unwrap().value.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].name.as_deref(), Some("Test Page"));
        assert_eq!(pages[0].summary.as_deref(), Some("A test summary"));
    }

    #[test]
    fn test_parse_empty_response() {
        let json = r#"{"code": 200, "msg": "success", "data": {"webPages": {"value": []}}}"#;
        let resp: BochaSearchResponse = serde_json::from_str(json).unwrap();
        let pages = resp.data.unwrap().web_pages.unwrap().value.unwrap();
        assert!(pages.is_empty());
    }
}
