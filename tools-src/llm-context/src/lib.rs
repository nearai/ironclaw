//! Brave Search LLM Context WASM Tool for IronClaw.
//!
//! Fetches pre-extracted web content from the Brave Search LLM Context API,
//! optimized for grounding LLM responses (RAG, fact-checking, research).
//!
//! # Authentication
//!
//! Uses the same Brave Search API key as the Web Search tool:
//! `ironclaw secret set brave_api_key <key>`
//!
//! Get a key at: https://brave.com/search/api/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

// Brave LLM Context API endpoint documentation:
// https://api-dashboard.search.brave.com/documentation/services/llm-context
//
// This tool uses POST with a JSON body (unlike Web Search's GET + query params) to avoid
// URL length limits and support richer parameters.

const BRAVE_LLM_CONTEXT_ENDPOINT: &str = "https://api.search.brave.com/res/v1/llm/context";

// Query and result limits (aligned with Brave API)
const MAX_QUERY_LEN: usize = 400;
const MAX_QUERY_WORDS: usize = 50;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 50;
const DEFAULT_COUNT: u32 = 20;
const MIN_TOKENS: u32 = 1024;
const MAX_TOKENS: u32 = 32768;
const DEFAULT_MAX_TOKENS: u32 = 8192;
const MIN_URLS: u32 = 1;
const MAX_URLS: u32 = 50;
const DEFAULT_MAX_URLS: u32 = 20;
const MIN_SNIPPETS: u32 = 1;
const MAX_SNIPPETS: u32 = 100;
const DEFAULT_MAX_SNIPPETS: u32 = 50;
const MIN_TOKENS_PER_URL: u32 = 512;
const MAX_TOKENS_PER_URL: u32 = 8192;
const DEFAULT_MAX_TOKENS_PER_URL: u32 = 4096;
const MIN_SNIPPETS_PER_URL: u32 = 1;
const MAX_SNIPPETS_PER_URL: u32 = 100;
const DEFAULT_SNIPPETS_PER_URL: u32 = 50;
const MAX_RETRIES: u32 = 3;

struct LlmContextTool;

impl exports::near::agent::tool::Guest for LlmContextTool {
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
        "Fetch pre-extracted web content from Brave Search for grounding LLM answers. \
         Returns actual page content (text chunks, tables, code) relevant to the query, \
         ready for RAG or fact-checking. Supports location-aware queries via optional \
         loc_lat, loc_long, loc_city, loc_state, loc_country, etc. for local/POI results. \
         Use when you need substantive content from the web rather than just links and \
         snippets. Authentication via 'brave_api_key' (same as Web Search)."
            .to_string()
    }
}

/// Input parameters for the LLM Context API. Snake_case fields map to Brave's JSON body
/// and optional X-Loc-* headers; validation and clamping happen in `execute_inner` / `build_request_body`.
#[derive(Debug, Deserialize)]
struct LlmContextParams {
    query: String,
    country: Option<String>,
    search_lang: Option<String>,
    count: Option<u32>,
    // Context Size Parameters
    maximum_number_of_urls: Option<u32>,
    maximum_number_of_tokens: Option<u32>,
    maximum_number_of_snippets: Option<u32>,
    maximum_number_of_tokens_per_url: Option<u32>,
    maximum_number_of_snippets_per_url: Option<u32>,
    // Filtering and Local Parameters
    context_threshold_mode: Option<String>,
    goggles: Option<serde_json::Value>,
    // Location-aware query headers
    #[serde(rename = "loc_lat")]
    loc_lat: Option<f64>,
    #[serde(rename = "loc_long")]
    loc_long: Option<f64>,
    #[serde(rename = "loc_city")]
    loc_city: Option<String>,
    #[serde(rename = "loc_state")]
    loc_state: Option<String>,
    #[serde(rename = "loc_state_name")]
    loc_state_name: Option<String>,
    #[serde(rename = "loc_country")]
    loc_country: Option<String>,
    #[serde(rename = "loc_postal_code")]
    loc_postal_code: Option<String>,
}

/// Top-level Brave LLM Context API response: optional grounding (generic/poi/map) and optional sources map.
#[derive(Debug, Deserialize)]
struct BraveLlmContextResponse {
    grounding: Option<Grounding>,
    sources: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Grounding content by type. See [LLM Context API](https://api-dashboard.search.brave.com/documentation/services/llm-context) and [LLM Context POST](https://api-dashboard.search.brave.com/api-reference/summarizer/llm_context/post).
#[derive(Debug, Deserialize)]
struct Grounding {
    /// Main grounding data: array of URL objects with extracted content (text chunks, tables, code).
    generic: Option<Vec<GenericEntry>>,
    /// Point-of-interest data, sometimes present when local recall is enabled (e.g. via X-Loc-* headers or enable_local).
    poi: Option<PoiMapEntry>,
    /// Map/place results when local recall is enabled. Array of place entries with name, url, title, snippets.
    map: Option<Vec<PoiMapEntry>>,
}

/// One URL's extracted content in `grounding.generic`: url, title, and text snippets.
#[derive(Clone, Debug, Deserialize)]
struct GenericEntry {
    url: Option<String>,
    title: Option<String>,
    snippets: Option<Vec<String>>,
}

/// Entry shape for `grounding.poi` (single object) and `grounding.map` (array). Present when local recall is active.
#[derive(Debug, Deserialize)]
struct PoiMapEntry {
    name: Option<String>,
    url: Option<String>,
    title: Option<String>,
    snippets: Option<Vec<String>>,
}

/// Runs the tool: validate params, call Brave LLM Context API (with retries), then normalize
/// grounding + sources into a single JSON output (sources, optional poi, optional map).
fn execute_inner(params: &str) -> Result<String, String> {
    let params: LlmContextParams =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    // Required and query-length validation (reject empty or whitespace-only)
    let query = params.query.trim();
    if query.is_empty() {
        return Err("'query' must not be empty or only whitespace".into());
    }
    if params.query.len() > MAX_QUERY_LEN {
        return Err(format!(
            "'query' exceeds maximum length of {} characters",
            MAX_QUERY_LEN
        ));
    }
    let word_count = params.query.split_whitespace().count();
    if word_count > MAX_QUERY_WORDS {
        return Err(format!(
            "'query' exceeds maximum of {} words (got {})",
            MAX_QUERY_WORDS, word_count
        ));
    }

    // Validate optional parameters (same style as Web Search tool)
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
    if let Some(ref mode) = params.context_threshold_mode {
        if !is_valid_threshold_mode(mode) {
            return Err(format!(
                "Invalid 'context_threshold_mode': expected 'strict', 'balanced', 'lenient', or 'disabled', got '{mode}'"
            ));
        }
    }

    if let Some(lat) = params.loc_lat {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(format!(
                "Invalid 'loc_lat': must be between -90 and 90 (got {lat})"
            ));
        }
    }
    if let Some(long) = params.loc_long {
        if !(-180.0..=180.0).contains(&long) {
            return Err(format!(
                "Invalid 'loc_long': must be between -180 and 180 (got {long})"
            ));
        }
    }
    if let Some(ref c) = params.loc_country {
        if !is_valid_country_code(c) {
            return Err(format!(
                "Invalid 'loc_country': expected 2-letter uppercase code like 'US', got '{c}'"
            ));
        }
    }

    // Pre-flight: verify API key is available (injected by host via credential config)
    if !near::agent::host::secret_exists("brave_api_key") {
        return Err(
            "Brave API key not found in secret store. Set it with: \
             ironclaw secret set brave_api_key <key>. \
             Get a key at: https://brave.com/search/api/"
                .into(),
        );
    }

    let url = BRAVE_LLM_CONTEXT_ENDPOINT;
    let body = build_request_body(&params)?;
    let headers = build_request_headers(&params);

    // Retry loop for transient errors (429 rate limit, 5xx server errors)
    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let resp = near::agent::host::http_request(
                "POST",
                url,
                &headers.to_string(),
                Some(&body),
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
                        "Brave LLM Context API error {} (attempt {}/{}). Retrying...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "Brave LLM Context API error (HTTP {}): {}",
                resp.status, body
            ));
        }
    };

    let body =
        String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let api_response: BraveLlmContextResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse Brave response: {e}"))?;

    let grounding = api_response.grounding.as_ref();
    let generic: Vec<GenericEntry> = grounding
        .and_then(|g| g.generic.as_ref())
        .map(|v| v.clone())
        .unwrap_or_default();

    // URL -> metadata (hostname, age) for enriching entries
    let sources = api_response.sources.unwrap_or_default();

    // Build normalized source entries: url, title, hostname, snippets, optional age
    let entries: Vec<serde_json::Value> = generic
        .into_iter()
        .filter_map(|e| {
            let url = e.url?;
            let title = e.title.unwrap_or_else(|| "Untitled".to_string());
            let snippets = e.snippets.unwrap_or_default();
            let source_meta = sources.get(&url).and_then(|v| v.get("hostname"));
            let hostname = source_meta
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| extract_hostname(&url).unwrap_or_default());
            let age_str = sources
                .get(&url)
                .and_then(|v| v.get("age"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str());

            let mut entry = serde_json::json!({
                "url": url,
                "title": title,
                "hostname": hostname,
                "snippets": snippets,
            });
            if let Some(age) = age_str {
                entry["age"] = serde_json::json!(age);
            }
            Some(entry)
        })
        .collect();

    // Optional POI (single) and map (array) when location-aware queries are used
    let poi_output = grounding
        .and_then(|g| g.poi.as_ref())
        .map(|e| poi_map_entry_to_json(e, &sources));
    let map_output: Vec<serde_json::Value> = grounding
        .and_then(|g| g.map.as_ref())
        .map(|m| m.iter().map(|e| poi_map_entry_to_json(e, &sources)).collect())
        .unwrap_or_default();

    let total_snippets: usize = entries
        .iter()
        .filter_map(|e| e.get("snippets").and_then(|s| s.as_array()))
        .map(|a| a.len())
        .sum();
    let poi_snippets: usize = poi_output
        .as_ref()
        .and_then(|o| o.get("snippets").and_then(|s| s.as_array()))
        .map(|a| a.len())
        .unwrap_or(0);
    let map_snippets: usize = map_output
        .iter()
        .filter_map(|e| e.get("snippets").and_then(|s| s.as_array()))
        .map(|a| a.len())
        .sum();

    let mut output = serde_json::json!({
        "query": params.query,
        "url_count": entries.len(),
        "snippet_count": total_snippets + poi_snippets + map_snippets,
        "sources": entries,
    });
    if let Some(poi) = poi_output {
        output["poi"] = poi;
    }
    if !map_output.is_empty() {
        output["map"] = serde_json::json!(map_output);
    }

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

/// Build the POST request body as JSON. Clamps numeric fields to API min/max; only includes
/// optional fields when present and valid.
fn build_request_body(params: &LlmContextParams) -> Result<Vec<u8>, String> {
    let count = params
        .count
        .unwrap_or(DEFAULT_COUNT)
        .clamp(MIN_COUNT, MAX_COUNT);
    let max_tokens = params
        .maximum_number_of_tokens
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .clamp(MIN_TOKENS, MAX_TOKENS);
    let max_urls = params
        .maximum_number_of_urls
        .unwrap_or(DEFAULT_MAX_URLS)
        .clamp(MIN_URLS, MAX_URLS);
    let max_snippets = params
        .maximum_number_of_snippets
        .unwrap_or(DEFAULT_MAX_SNIPPETS)
        .clamp(MIN_SNIPPETS, MAX_SNIPPETS);
    let max_tokens_per_url = params
        .maximum_number_of_tokens_per_url
        .unwrap_or(DEFAULT_MAX_TOKENS_PER_URL)
        .clamp(MIN_TOKENS_PER_URL, MAX_TOKENS_PER_URL);
    let max_snippets_per_url = params
        .maximum_number_of_snippets_per_url
        .unwrap_or(DEFAULT_SNIPPETS_PER_URL)
        .clamp(MIN_SNIPPETS_PER_URL, MAX_SNIPPETS_PER_URL);

    let mut body = serde_json::Map::new();
    body.insert("q".to_string(), serde_json::Value::String(params.query.clone()));
    body.insert("count".to_string(), serde_json::Value::Number(serde_json::Number::from(count)));
    body.insert(
        "maximum_number_of_tokens".to_string(),
        serde_json::Value::Number(serde_json::Number::from(max_tokens)),
    );
    body.insert(
        "maximum_number_of_urls".to_string(),
        serde_json::Value::Number(serde_json::Number::from(max_urls)),
    );
    body.insert(
        "maximum_number_of_snippets".to_string(),
        serde_json::Value::Number(serde_json::Number::from(max_snippets)),
    );
    body.insert(
        "maximum_number_of_tokens_per_url".to_string(),
        serde_json::Value::Number(serde_json::Number::from(max_tokens_per_url)),
    );
    body.insert(
        "maximum_number_of_snippets_per_url".to_string(),
        serde_json::Value::Number(serde_json::Number::from(max_snippets_per_url)),
    );

    if let Some(ref country) = params.country {
        body.insert(
            "country".to_string(),
            serde_json::Value::String(country.clone()),
        );
    }
    if let Some(ref search_lang) = params.search_lang {
        body.insert(
            "search_lang".to_string(),
            serde_json::Value::String(search_lang.clone()),
        );
    }
    if let Some(ref mode) = params.context_threshold_mode {
        if is_valid_threshold_mode(mode) {
            body.insert(
                "context_threshold_mode".to_string(),
                serde_json::Value::String(mode.clone()),
            );
        }
    }
    if let Some(ref g) = params.goggles {
        if is_valid_goggles_value(g) {
            body.insert("goggles".to_string(), g.clone());
        }
    }

    serde_json::to_vec(&serde_json::Value::Object(body))
        .map_err(|e| format!("Failed to serialize request body: {e}"))
}

/// Build HTTP request headers: Accept, Content-Type, User-Agent, and optional X-Loc-*
/// for location-aware queries. API key is injected by the host (same as Web Search).
fn build_request_headers(params: &LlmContextParams) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "Accept".to_string(),
        serde_json::Value::String("application/json".to_string()),
    );
    map.insert(
        "Content-Type".to_string(),
        serde_json::Value::String("application/json".to_string()),
    );
    map.insert(
        "User-Agent".to_string(),
        serde_json::Value::String("IronClaw-LlmContext-Tool/0.1".to_string()),
    );

    if let Some(lat) = params.loc_lat {
        map.insert(
            "X-Loc-Lat".to_string(),
            serde_json::Value::String(format!("{lat}")),
        );
    }
    if let Some(long) = params.loc_long {
        map.insert(
            "X-Loc-Long".to_string(),
            serde_json::Value::String(format!("{long}")),
        );
    }
    if let Some(ref s) = params.loc_city {
        map.insert(
            "X-Loc-City".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }
    if let Some(ref s) = params.loc_state {
        map.insert(
            "X-Loc-State".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }
    if let Some(ref s) = params.loc_state_name {
        map.insert(
            "X-Loc-State-Name".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }
    if let Some(ref s) = params.loc_country {
        map.insert(
            "X-Loc-Country".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }
    if let Some(ref s) = params.loc_postal_code {
        map.insert(
            "X-Loc-Postal-Code".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }

    serde_json::Value::Object(map)
}

/// Build a JSON object for a POI or map entry (name, url, title, hostname, snippets, age when available).
fn poi_map_entry_to_json(
    e: &PoiMapEntry,
    sources: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let url = e.url.clone().unwrap_or_default();
    let title = e.title.clone().unwrap_or_else(|| "Untitled".to_string());
    let name = e.name.clone().unwrap_or_default();
    let snippets = e.snippets.clone().unwrap_or_default();
    let hostname = sources
        .get(&url)
        .and_then(|v| v.get("hostname").and_then(|h| h.as_str()))
        .map(String::from)
        .unwrap_or_else(|| extract_hostname(&url).unwrap_or_default());
    let age_str = sources
        .get(&url)
        .and_then(|v| v.get("age"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str());

    let mut entry = serde_json::json!({
        "name": name,
        "url": url,
        "title": title,
        "hostname": hostname,
        "snippets": snippets,
    });
    if let Some(age) = age_str {
        entry["age"] = serde_json::json!(age);
    }
    entry
}

/// Extract hostname from a URL string (no URL parser dependency). Handles http(s) and strips port.
fn extract_hostname(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next()?;
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

/// Validate context_threshold_mode: strict, balanced, lenient, or disabled.
fn is_valid_threshold_mode(s: &str) -> bool {
    matches!(s, "strict" | "balanced" | "lenient" | "disabled")
}

/// Goggles must be a non-empty string or a non-empty array of strings (URLs or inline definitions).
fn is_valid_goggles_value(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => {
            !a.is_empty()
                && a.iter().all(|e| matches!(e, serde_json::Value::String(s) if !s.is_empty()))
        }
        _ => false,
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "Search query; returns pre-extracted web content (text, tables, code) for grounding LLM answers",
            "minLength": 1,
            "maxLength": 400
        },
        "count": {
            "type": "integer",
            "description": "Maximum number of search results to consider (1-50, default 20)",
            "minimum": 1,
            "maximum": 50,
            "default": 20
        },
        "country": {
            "type": "string",
            "description": "2-letter uppercase country code (e.g. 'US', 'DE')"
        },
        "search_lang": {
            "type": "string",
            "description": "2-letter lowercase language code for results (e.g. 'en', 'de')"
        },
        "maximum_number_of_tokens": {
            "type": "integer",
            "description": "Approximate max tokens in returned context (1024-32768, default 8192)",
            "minimum": 1024,
            "maximum": 32768,
            "default": 8192
        },
        "maximum_number_of_urls": {
            "type": "integer",
            "description": "Maximum URLs to include (1-50, default 20)",
            "minimum": 1,
            "maximum": 50,
            "default": 20
        },
        "maximum_number_of_snippets": {
            "type": "integer",
            "description": "Maximum snippets across all URLs (1-100, default 50)",
            "minimum": 1,
            "maximum": 100,
            "default": 50
        },
        "maximum_number_of_tokens_per_url": {
            "type": "integer",
            "description": "Max tokens per URL (512-8192, default 4096)",
            "minimum": 512,
            "maximum": 8192,
            "default": 4096
        },
        "maximum_number_of_snippets_per_url": {
            "type": "integer",
            "description": "Max snippets per URL (1-100, default 50)",
            "minimum": 1,
            "maximum": 100,
            "default": 50
        },
        "context_threshold_mode": {
            "type": "string",
            "description": "Relevance filter: 'strict' (fewer, more relevant), 'balanced', 'lenient', or 'disabled'",
            "enum": ["strict", "balanced", "lenient", "disabled"]
        },
        "loc_lat": {
            "type": "number",
            "description": "Latitude for location-aware queries (-90 to 90). Use with loc_long or place-name headers for local/POI results."
        },
        "loc_long": {
            "type": "number",
            "description": "Longitude for location-aware queries (-180 to 180). Use with loc_lat or place-name headers for local/POI results."
        },
        "loc_city": {
            "type": "string",
            "description": "City name for location-aware queries (e.g. 'San Francisco')"
        },
        "loc_state": {
            "type": "string",
            "description": "State/region code for location-aware queries (e.g. 'CA', ISO 3166-2)"
        },
        "loc_state_name": {
            "type": "string",
            "description": "State/region full name for location-aware queries"
        },
        "loc_country": {
            "type": "string",
            "description": "2-letter uppercase country code for location headers (e.g. 'US'). Enables local recall for queries like 'coffee shops near me'."
        },
        "loc_postal_code": {
            "type": "string",
            "description": "Postal code for location-aware queries"
        },
        "goggles": {
            "description": "Custom ranking/filtering: URL to a Goggle file, inline Goggles rules, or array of URLs/inline strings. Restrict or boost sources (e.g. trusted domains). See https://api-dashboard.search.brave.com/documentation/resources/goggles",
            "oneOf": [
                { "type": "string", "minLength": 1 },
                { "type": "array", "items": { "type": "string", "minLength": 1 }, "minItems": 1 }
            ]
        }
    },
    "required": ["query"],
    "additionalProperties": false
}"#;

export!(LlmContextTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_hostname() {
        assert_eq!(
            extract_hostname("https://example.com/path"),
            Some("example.com".into())
        );
        assert_eq!(extract_hostname("https://"), None);
    }

    #[test]
    fn test_is_valid_lang_code() {
        assert!(is_valid_lang_code("en"));
        assert!(!is_valid_lang_code("EN"));
        assert!(!is_valid_lang_code("eng"));
    }

    #[test]
    fn test_is_valid_country_code() {
        assert!(is_valid_country_code("US"));
        assert!(!is_valid_country_code("us"));
        assert!(!is_valid_country_code("USA"));
    }

    #[test]
    fn test_is_valid_threshold_mode() {
        assert!(is_valid_threshold_mode("strict"));
        assert!(is_valid_threshold_mode("balanced"));
        assert!(is_valid_threshold_mode("lenient"));
        assert!(is_valid_threshold_mode("disabled"));
        assert!(!is_valid_threshold_mode("invalid"));
    }

    fn params_minimal() -> LlmContextParams {
        LlmContextParams {
            query: "rust async".to_string(),
            count: None,
            country: None,
            search_lang: None,
            maximum_number_of_tokens: None,
            maximum_number_of_urls: None,
            maximum_number_of_snippets: None,
            maximum_number_of_tokens_per_url: None,
            maximum_number_of_snippets_per_url: None,
            context_threshold_mode: None,
            loc_lat: None,
            loc_long: None,
            loc_city: None,
            loc_state: None,
            loc_state_name: None,
            loc_country: None,
            loc_postal_code: None,
            goggles: None,
        }
    }

    #[test]
    fn test_build_request_body_minimal() {
        let params = params_minimal();
        let body = build_request_body(&params).unwrap();
        let obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(obj.get("q").and_then(|v| v.as_str()), Some("rust async"));
        assert_eq!(obj.get("count").and_then(|v| v.as_u64()), Some(20));
        assert_eq!(
            obj.get("maximum_number_of_tokens").and_then(|v| v.as_u64()),
            Some(8192)
        );
        assert!(!obj.contains_key("country"));
        assert!(!obj.contains_key("context_threshold_mode"));
    }

    #[test]
    fn test_build_request_body_full() {
        let params = LlmContextParams {
            query: "python asyncio".to_string(),
            count: Some(10),
            country: Some("US".to_string()),
            search_lang: Some("en".to_string()),
            maximum_number_of_tokens: Some(4096),
            maximum_number_of_urls: Some(10),
            maximum_number_of_snippets: Some(25),
            maximum_number_of_tokens_per_url: Some(2048),
            maximum_number_of_snippets_per_url: Some(25),
            context_threshold_mode: Some("strict".to_string()),
            loc_lat: None,
            loc_long: None,
            loc_city: None,
            loc_state: None,
            loc_state_name: None,
            loc_country: None,
            loc_postal_code: None,
            goggles: None,
        };
        let body = build_request_body(&params).unwrap();
        let obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(obj.get("q").and_then(|v| v.as_str()), Some("python asyncio"));
        assert_eq!(obj.get("count").and_then(|v| v.as_u64()), Some(10));
        assert_eq!(obj.get("country").and_then(|v| v.as_str()), Some("US"));
        assert_eq!(obj.get("search_lang").and_then(|v| v.as_str()), Some("en"));
        assert_eq!(
            obj.get("maximum_number_of_tokens").and_then(|v| v.as_u64()),
            Some(4096)
        );
        assert_eq!(
            obj.get("context_threshold_mode").and_then(|v| v.as_str()),
            Some("strict")
        );
    }

    #[test]
    fn test_build_request_headers_with_location() {
        let params = LlmContextParams {
            query: "coffee shops".to_string(),
            count: None,
            country: None,
            search_lang: None,
            maximum_number_of_tokens: None,
            maximum_number_of_urls: None,
            maximum_number_of_snippets: None,
            maximum_number_of_tokens_per_url: None,
            maximum_number_of_snippets_per_url: None,
            context_threshold_mode: None,
            loc_lat: Some(37.7749),
            loc_long: Some(-122.4194),
            loc_city: Some("San Francisco".to_string()),
            loc_state: Some("CA".to_string()),
            loc_state_name: Some("California".to_string()),
            loc_country: Some("US".to_string()),
            loc_postal_code: Some("94102".to_string()),
            goggles: None,
        };
        let headers = build_request_headers(&params);
        let obj = headers.as_object().unwrap();
        assert_eq!(obj.get("Accept").and_then(|v| v.as_str()), Some("application/json"));
        assert_eq!(obj.get("X-Loc-Lat").and_then(|v| v.as_str()), Some("37.7749"));
        assert_eq!(obj.get("X-Loc-Long").and_then(|v| v.as_str()), Some("-122.4194"));
        assert_eq!(obj.get("X-Loc-City").and_then(|v| v.as_str()), Some("San Francisco"));
        assert_eq!(obj.get("X-Loc-State").and_then(|v| v.as_str()), Some("CA"));
        assert_eq!(obj.get("X-Loc-State-Name").and_then(|v| v.as_str()), Some("California"));
        assert_eq!(obj.get("X-Loc-Country").and_then(|v| v.as_str()), Some("US"));
        assert_eq!(obj.get("X-Loc-Postal-Code").and_then(|v| v.as_str()), Some("94102"));
    }

    #[test]
    fn test_build_request_headers_no_location() {
        let params = params_minimal();
        let headers = build_request_headers(&params);
        let obj = headers.as_object().unwrap();
        assert_eq!(obj.get("Accept").and_then(|v| v.as_str()), Some("application/json"));
        assert_eq!(
            obj.get("Content-Type").and_then(|v| v.as_str()),
            Some("application/json")
        );
        assert!(obj.get("User-Agent").is_some());
        assert!(obj.get("X-Loc-Lat").is_none());
        assert!(obj.get("X-Loc-Country").is_none());
    }

    #[test]
    fn test_build_request_body_with_goggles_string() {
        let mut params = params_minimal();
        params.query = "rust programming".to_string();
        params.goggles = Some(serde_json::Value::String(
            "https://raw.githubusercontent.com/brave/goggles-quickstart/main/goggles/tech_blogs.goggle"
                .to_string(),
        ));
        let body = build_request_body(&params).unwrap();
        let obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(
            obj.get("goggles").and_then(|v| v.as_str()),
            Some("https://raw.githubusercontent.com/brave/goggles-quickstart/main/goggles/tech_blogs.goggle")
        );
    }

    #[test]
    fn test_build_request_body_with_goggles_array() {
        let mut params = params_minimal();
        params.query = "web development".to_string();
        params.goggles = Some(serde_json::json!([
            "https://example.com/goggle1.goggle",
            "$boost=3,site=dev.to"
        ]));
        let body = build_request_body(&params).unwrap();
        let obj: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&body).unwrap();
        let arr = obj.get("goggles").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str(), Some("https://example.com/goggle1.goggle"));
        assert_eq!(arr[1].as_str(), Some("$boost=3,site=dev.to"));
    }

    #[test]
    fn test_is_valid_goggles_value() {
        assert!(is_valid_goggles_value(&serde_json::Value::String("https://x.com/a.goggle".to_string())));
        assert!(is_valid_goggles_value(&serde_json::json!(["https://a.com", "$boost,site=dev.to"])));
        assert!(!is_valid_goggles_value(&serde_json::Value::String("".to_string())));
        assert!(!is_valid_goggles_value(&serde_json::Value::Array(vec![])));
        assert!(!is_valid_goggles_value(&serde_json::Value::Bool(true)));
    }

    #[test]
    fn test_parse_response() {
        let body = r#"{
            "grounding": {
                "generic": [
                    {
                        "url": "https://example.com/page",
                        "title": "Example Page",
                        "snippets": ["First snippet.", "Second snippet."]
                    }
                ]
            },
            "sources": {
                "https://example.com/page": {
                    "title": "Example Page",
                    "hostname": "example.com",
                    "age": ["2024-01-15", "380 days ago"]
                }
            }
        }"#;
        let r: BraveLlmContextResponse = serde_json::from_str(body).unwrap();
        let generic = r.grounding.unwrap().generic.unwrap();
        assert_eq!(generic.len(), 1);
        assert_eq!(generic[0].url.as_deref(), Some("https://example.com/page"));
        assert_eq!(generic[0].title.as_deref(), Some("Example Page"));
        assert_eq!(generic[0].snippets.as_ref().unwrap().len(), 2);
        let sources = r.sources.unwrap();
        let meta = sources.get("https://example.com/page").unwrap();
        assert_eq!(meta.get("hostname").and_then(|v| v.as_str()), Some("example.com"));
    }

    #[test]
    fn test_parse_response_with_poi_and_map() {
        let body = r#"{
            "grounding": {
                "generic": [{"url": "https://example.com/page", "title": "Example", "snippets": []}],
                "poi": {
                    "name": "Business Name",
                    "url": "https://business.com",
                    "title": "Title of business.com website",
                    "snippets": ["Business details."]
                },
                "map": [
                    {
                        "name": "Place Name",
                        "url": "https://place.com",
                        "title": "Title of place.com",
                        "snippets": ["Place information."]
                    }
                ]
            },
            "sources": {
                "https://business.com": {"title": "Business Name", "hostname": "business.com", "age": null},
                "https://place.com": {"title": "Place", "hostname": "place.com", "age": null}
            }
        }"#;
        let r: BraveLlmContextResponse = serde_json::from_str(body).unwrap();
        let g = r.grounding.as_ref().unwrap();
        assert_eq!(g.generic.as_ref().unwrap().len(), 1);
        let poi = g.poi.as_ref().unwrap();
        assert_eq!(poi.name.as_deref(), Some("Business Name"));
        assert_eq!(poi.url.as_deref(), Some("https://business.com"));
        assert_eq!(poi.snippets.as_ref().unwrap().len(), 1);
        let map = g.map.as_ref().unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].name.as_deref(), Some("Place Name"));
        assert_eq!(map[0].url.as_deref(), Some("https://place.com"));
    }

    #[test]
    fn test_poi_map_entry_to_json() {
        let e = PoiMapEntry {
            name: Some("Cafe Example".to_string()),
            url: Some("https://cafe.example.com".to_string()),
            title: Some("Cafe Example - Coffee".to_string()),
            snippets: Some(vec!["Best coffee in town.".to_string()]),
        };
        let mut sources = serde_json::Map::new();
        sources.insert(
            "https://cafe.example.com".to_string(),
            serde_json::json!({"hostname": "cafe.example.com", "age": ["2024-06-01"]}),
        );
        let out = poi_map_entry_to_json(&e, &sources);
        assert_eq!(out.get("name").and_then(|v| v.as_str()), Some("Cafe Example"));
        assert_eq!(out.get("url").and_then(|v| v.as_str()), Some("https://cafe.example.com"));
        assert_eq!(out.get("hostname").and_then(|v| v.as_str()), Some("cafe.example.com"));
        assert_eq!(out.get("age").and_then(|v| v.as_str()), Some("2024-06-01"));
        let snippets = out.get("snippets").and_then(|s| s.as_array()).unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].as_str(), Some("Best coffee in town."));
    }
}
