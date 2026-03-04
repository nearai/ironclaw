//! Brave Web Search WASM Tool for IronClaw.
//!
//! Searches the web using the Brave Search API and returns structured results.
//!
//! # Authentication
//!
//! Store your Brave Search API key:
//! `ironclaw secret set brave_api_key <key>`
//!
//! Get a key at: https://brave.com/search/api/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BRAVE_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const MAX_COUNT: u32 = 20;
const DEFAULT_COUNT: u32 = 5;
const MAX_RETRIES: u32 = 3;

struct WebSearchTool;

impl exports::near::agent::tool::Guest for WebSearchTool {
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
        "Search the web using Brave Search. Returns titles, URLs, descriptions, and \
         publication dates for matching web pages. Supports filtering by country, \
         language, and freshness. Authentication is handled via the 'brave_api_key' \
         secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    query: String,
    count: Option<u32>,
    country: Option<String>,
    search_lang: Option<String>,
    ui_lang: Option<String>,
    freshness: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Option<Vec<BraveSearchResult>>,
}

#[derive(Debug, Deserialize)]
struct BraveSearchResult {
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    age: Option<String>,
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

    // Validate optional parameters.
    if let Some(ref lang) = params.search_lang {
        if !is_valid_lang_code(lang) {
            return Err(format!(
                "Invalid 'search_lang': expected 2-letter code like 'en', got '{lang}'"
            ));
        }
    }
    if let Some(ref country) = params.country {
        if !is_valid_country_code(country) {
            return Err(format!(
                "Invalid 'country': expected 2-letter code like 'US', got '{country}'"
            ));
        }
    }
    if let Some(ref ui_lang) = params.ui_lang {
        if !is_valid_ui_lang(ui_lang) {
            return Err(format!(
                "Invalid 'ui_lang': expected format like 'en-US', got '{ui_lang}'"
            ));
        }
    }
    if let Some(ref freshness) = params.freshness {
        if !is_valid_freshness(freshness) {
            return Err(format!(
                "Invalid 'freshness': expected 'pd', 'pw', 'pm', 'py', or \
                 'YYYY-MM-DDtoYYYY-MM-DD', got '{freshness}'"
            ));
        }
    }

    // Pre-flight: verify API key is available.
    if !near::agent::host::secret_exists("brave_api_key") {
        return Err(
            "Brave API key not found in secret store. Set it with: \
             ironclaw secret set brave_api_key <key>. \
             Get a key at: https://brave.com/search/api/"
                .into(),
        );
    }

    let count = params.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);
    let url = build_search_url(&params.query, count, &params);

    // X-Subscription-Token is injected by the host via credential config.
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-WebSearch-Tool/0.1"
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
                        "Brave API error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "Brave API error (HTTP {}): {}",
                resp.status, body
            ));
        }
    };

    let body =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let brave_response: BraveSearchResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse Brave response: {e}"))?;

    let results = brave_response
        .web
        .and_then(|w| w.results)
        .unwrap_or_default();

    let formatted: Vec<serde_json::Value> = results
        .into_iter()
        .filter_map(|r| {
            let title = r.title?;
            let url = r.url?;
            let description = r.description.unwrap_or_default();

            let mut entry = serde_json::json!({
                "title": title,
                "url": url,
                "description": description,
            });
            if let Some(age) = r.age {
                entry["published"] = serde_json::json!(age);
            }
            // Extract hostname for site_name.
            if let Some(host) = extract_hostname(&url) {
                entry["site_name"] = serde_json::json!(host);
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

fn build_search_url(query: &str, count: u32, params: &SearchParams) -> String {
    let mut url = format!(
        "{}?q={}&count={}",
        BRAVE_SEARCH_ENDPOINT,
        url_encode(query),
        count
    );

    if let Some(ref country) = params.country {
        url.push_str(&format!("&country={}", url_encode(country)));
    }
    if let Some(ref search_lang) = params.search_lang {
        url.push_str(&format!("&search_lang={}", url_encode(search_lang)));
    }
    if let Some(ref ui_lang) = params.ui_lang {
        url.push_str(&format!("&ui_lang={}", url_encode(ui_lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&freshness={}", url_encode(freshness)));
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

/// Validate a 2-letter language code (e.g. "en", "de").
fn is_valid_lang_code(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_lowercase())
}

/// Validate a 2-letter country code (e.g. "US", "DE").
fn is_valid_country_code(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// Validate a UI locale string (e.g. "en-US").
fn is_valid_ui_lang(s: &str) -> bool {
    let mut parts = s.split('-');
    if let (Some(lang), Some(country), None) = (parts.next(), parts.next(), parts.next()) {
        is_valid_lang_code(lang) && is_valid_country_code(country)
    } else {
        false
    }
}

/// Validate a freshness filter value.
fn is_valid_freshness(s: &str) -> bool {
    matches!(s, "pd" | "pw" | "pm" | "py") || is_valid_date_range(s)
}

/// Check if the string is a valid date range like "2024-01-01to2024-12-31".
fn is_valid_date_range(s: &str) -> bool {
    if let Some((start, end)) = s.split_once("to") {
        is_date_like(start) && is_date_like(end)
    } else {
        false
    }
}

/// Basic check for YYYY-MM-DD format.
fn is_date_like(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes().get(4) == Some(&b'-')
        && s.as_bytes().get(7) == Some(&b'-')
        && s.bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "The search query to look up on the web"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-20, default 5)",
            "minimum": 1,
            "maximum": 20,
            "default": 5
        },
        "country": {
            "type": "string",
            "description": "2-letter uppercase country code to bias results (e.g. 'US', 'DE', 'JP')"
        },
        "search_lang": {
            "type": "string",
            "description": "2-letter lowercase language code for search results (e.g. 'en', 'de', 'fr')"
        },
        "ui_lang": {
            "type": "string",
            "description": "Locale in language-region format (e.g. 'en-US', 'de-DE')"
        },
        "freshness": {
            "type": "string",
            "description": "Filter by discovery time: 'pd' (past day), 'pw' (past week), 'pm' (past month), 'py' (past year), or date range 'YYYY-MM-DDtoYYYY-MM-DD'"
        }
    },
    "required": ["query"],
    "additionalProperties": false
}"#;

export!(WebSearchTool);

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
    fn test_is_valid_lang_code() {
        assert!(is_valid_lang_code("en"));
        assert!(is_valid_lang_code("de"));
        assert!(!is_valid_lang_code("EN")); // must be lowercase
        assert!(!is_valid_lang_code("eng")); // too long
        assert!(!is_valid_lang_code("")); // empty
    }

    #[test]
    fn test_is_valid_country_code() {
        assert!(is_valid_country_code("US"));
        assert!(is_valid_country_code("DE"));
        assert!(!is_valid_country_code("us")); // must be uppercase
        assert!(!is_valid_country_code("USA")); // too long
    }

    #[test]
    fn test_is_valid_ui_lang() {
        assert!(is_valid_ui_lang("en-US"));
        assert!(is_valid_ui_lang("de-DE"));
        assert!(!is_valid_ui_lang("en"));
        assert!(!is_valid_ui_lang("EN-US")); // lang part must be lowercase
        assert!(!is_valid_ui_lang("en-us")); // country part must be uppercase
    }

    #[test]
    fn test_is_valid_freshness() {
        assert!(is_valid_freshness("pd"));
        assert!(is_valid_freshness("pw"));
        assert!(is_valid_freshness("pm"));
        assert!(is_valid_freshness("py"));
        assert!(is_valid_freshness("2024-01-01to2024-12-31"));
        assert!(!is_valid_freshness("invalid"));
        assert!(!is_valid_freshness("2024-01-01")); // missing end date
    }

    #[test]
    fn test_is_date_like() {
        assert!(is_date_like("2024-01-15"));
        assert!(is_date_like("2025-12-31"));
        assert!(!is_date_like("2024-1-15")); // not zero-padded
        assert!(!is_date_like("24-01-15")); // short year
        assert!(!is_date_like("")); // empty
    }

    #[test]
    fn test_build_search_url_minimal() {
        let params = SearchParams {
            query: "test query".to_string(),
            count: None,
            country: None,
            search_lang: None,
            ui_lang: None,
            freshness: None,
        };
        let url = build_search_url("test query", 5, &params);
        assert!(url.starts_with(BRAVE_SEARCH_ENDPOINT));
        assert!(url.contains("q=test%20query"));
        assert!(url.contains("count=5"));
        assert!(!url.contains("country="));
    }

    #[test]
    fn test_build_search_url_full() {
        let params = SearchParams {
            query: "rust programming".to_string(),
            count: Some(10),
            country: Some("US".to_string()),
            search_lang: Some("en".to_string()),
            ui_lang: Some("en-US".to_string()),
            freshness: Some("pw".to_string()),
        };
        let url = build_search_url("rust programming", 10, &params);
        assert!(url.contains("q=rust%20programming"));
        assert!(url.contains("count=10"));
        assert!(url.contains("country=US"));
        assert!(url.contains("search_lang=en"));
        assert!(url.contains("ui_lang=en-US"));
        assert!(url.contains("freshness=pw"));
    }

    #[test]
    fn test_url_encode_multibyte() {
        assert_eq!(url_encode("café"), "caf%C3%A9");
        assert_eq!(url_encode("日本語"), "%E6%97%A5%E6%9C%AC%E8%AA%9E");
    }

    #[test]
    fn test_extract_hostname_empty() {
        assert_eq!(extract_hostname("https://"), None);
        assert_eq!(extract_hostname("https:///path"), None);
        assert_eq!(extract_hostname(""), None);
    }
}
