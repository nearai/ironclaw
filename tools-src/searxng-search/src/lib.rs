//! SearXNG Web Search WASM Tool for IronClaw.
//!
//! Searches the web using a self-hosted SearXNG instance. No API key required.
//! Aggregates results from Google, Bing, DuckDuckGo, Baidu, Sogou, and more.
//!
//! SearXNG must be running as a Docker service on the same network (e.g. `searxng:8080`).

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const SEARXNG_ENDPOINT: &str = "http://searxng:8080/search";
const MAX_COUNT: u32 = 50;
const DEFAULT_COUNT: u32 = 10;
const MAX_RETRIES: u32 = 3;

struct SearxngSearchTool;

impl exports::near::agent::tool::Guest for SearxngSearchTool {
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
        "Search the web using SearXNG (self-hosted metasearch engine). Returns titles, URLs, \
         descriptions, source engines, and publication dates. Aggregates results from Google, \
         Bing, DuckDuckGo, Baidu, and more. No API key required — completely free and unlimited."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    query: String,
    count: Option<u32>,
    language: Option<String>,
    time_range: Option<String>,
    categories: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    results: Option<Vec<SearxngResult>>,
    number_of_results: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    engine: Option<String>,
    #[serde(default)]
    engines: Vec<String>,
    publishedDate: Option<String>,
    img_src: Option<String>,
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

    if let Some(ref time_range) = params.time_range {
        if !is_valid_time_range(time_range) {
            return Err(format!(
                "Invalid 'time_range': expected 'day', 'week', 'month', or 'year', got '{time_range}'"
            ));
        }
    }

    let count = params.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);
    let url = build_search_url(&params, count);

    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-SearxngSearch-Tool/0.1"
    });

    // Retry loop for transient errors (429 rate limit, 5xx server errors).
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
                        "SearXNG error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "SearXNG error (HTTP {}): {}",
                resp.status, body
            ));
        }
    };

    let body =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let searxng_response: SearxngResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse SearXNG response: {e}"))?;

    let results = searxng_response.results.unwrap_or_default();

    // Limit results to requested count.
    let formatted: Vec<serde_json::Value> = results
        .into_iter()
        .take(count as usize)
        .filter_map(|r| {
            let title = r.title.filter(|t| !t.is_empty())?;
            let url = r.url.filter(|u| !u.is_empty())?;

            let mut entry = serde_json::json!({
                "title": title,
                "url": url,
            });

            if let Some(content) = r.content.filter(|c| !c.is_empty()) {
                entry["description"] = serde_json::json!(content);
            }

            // Show which engines returned this result.
            if !r.engines.is_empty() {
                entry["engines"] = serde_json::json!(r.engines);
            } else if let Some(engine) = r.engine {
                entry["engines"] = serde_json::json!([engine]);
            }

            if let Some(host) = extract_hostname(&url) {
                entry["site_name"] = serde_json::json!(host);
            }

            if let Some(date) = r.publishedDate {
                entry["published"] = serde_json::json!(date);
            }

            if let Some(img) = r.img_src {
                entry["thumbnail"] = serde_json::json!(img);
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

fn build_search_url(params: &SearchParams, count: u32) -> String {
    let mut url = format!(
        "{}?q={}&format=json&pageno=1",
        SEARXNG_ENDPOINT,
        url_encode(&params.query),
    );

    // SearXNG doesn't have a direct "count" param — it returns ~10 results per page.
    // We request enough pages to cover the count, capped at page 5.
    let pages_needed = ((count as f32) / 10.0).ceil() as u32;
    if pages_needed > 1 {
        // For simplicity, we request page 1 and filter results client-side.
        // Multi-page fetching would require multiple HTTP calls.
        let _ = pages_needed; // acknowledged but not used for v1
    }

    if let Some(ref lang) = params.language {
        url.push_str(&format!("&language={}", url_encode(lang)));
    }

    if let Some(ref time_range) = params.time_range {
        url.push_str(&format!("&time_range={}", url_encode(time_range)));
    }

    if let Some(ref categories) = params.categories {
        url.push_str(&format!("&categories={}", url_encode(categories)));
    } else {
        url.push_str("&categories=general");
    }

    url
}

/// Percent-encode a string for safe use in URL query parameters.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
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

/// Validate a time range filter value.
fn is_valid_time_range(s: &str) -> bool {
    matches!(s, "day" | "week" | "month" | "year")
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
        "language": {
            "type": "string",
            "description": "Language/locale for search results (e.g. 'zh-CN', 'en-US', 'auto')",
            "default": "auto"
        },
        "time_range": {
            "type": "string",
            "description": "Filter by time range: 'day' (past day), 'week' (past week), 'month' (past month), 'year' (past year)",
            "enum": ["day", "week", "month", "year"]
        },
        "categories": {
            "type": "string",
            "description": "Search category (default 'general'). Options: 'general', 'images', 'news', 'science', 'it'",
            "default": "general"
        }
    },
    "required": ["query"],
    "additionalProperties": false
}"#;

export!(SearxngSearchTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("foo&bar=baz"), "foo%26bar%3Dbaz");
        assert_eq!(url_encode("simple"), "simple");
    }

    #[test]
    fn test_url_encode_multibyte() {
        assert_eq!(url_encode("café"), "caf%C3%A9");
        assert_eq!(url_encode("日本語"), "%E6%97%A5%E6%9C%AC%E8%AA%9E");
    }

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
    fn test_is_valid_time_range() {
        assert!(is_valid_time_range("day"));
        assert!(is_valid_time_range("week"));
        assert!(is_valid_time_range("month"));
        assert!(is_valid_time_range("year"));
        assert!(!is_valid_time_range("pd"));
        assert!(!is_valid_time_range("noLimit"));
        assert!(!is_valid_time_range(""));
    }

    #[test]
    fn test_build_search_url_minimal() {
        let params = SearchParams {
            query: "test query".to_string(),
            count: None,
            language: None,
            time_range: None,
            categories: None,
        };
        let url = build_search_url(&params, 10);
        assert!(url.starts_with(SEARXNG_ENDPOINT));
        assert!(url.contains("q=test%20query"));
        assert!(url.contains("format=json"));
        assert!(url.contains("categories=general"));
        assert!(!url.contains("language="));
        assert!(!url.contains("time_range="));
    }

    #[test]
    fn test_build_search_url_full() {
        let params = SearchParams {
            query: "rust programming".to_string(),
            count: Some(20),
            language: Some("en-US".to_string()),
            time_range: Some("week".to_string()),
            categories: Some("it".to_string()),
        };
        let url = build_search_url(&params, 20);
        assert!(url.contains("q=rust%20programming"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("time_range=week"));
        assert!(url.contains("categories=it"));
    }

    #[test]
    fn test_parse_searxng_response() {
        let json = r#"{
            "results": [
                {
                    "title": "Test Page",
                    "url": "https://example.com",
                    "content": "A test snippet",
                    "engine": "google",
                    "engines": ["google", "bing"],
                    "publishedDate": "2025-01-01"
                }
            ],
            "number_of_results": 1.0
        }"#;
        let resp: SearxngResponse = serde_json::from_str(json).unwrap();
        let results = resp.results.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title.as_deref(), Some("Test Page"));
        assert_eq!(results[0].engines, vec!["google", "bing"]);
    }

    #[test]
    fn test_parse_empty_response() {
        let json = r#"{"results": [], "number_of_results": 0}"#;
        let resp: SearxngResponse = serde_json::from_str(json).unwrap();
        let results = resp.results.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_response_missing_fields() {
        let json = r#"{"results": [{"title": "T", "url": "https://x.com"}]}"#;
        let resp: SearxngResponse = serde_json::from_str(json).unwrap();
        let results = resp.results.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.is_none());
        assert!(results[0].publishedDate.is_none());
    }
}
