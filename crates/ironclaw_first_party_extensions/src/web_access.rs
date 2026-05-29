use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    ResourceScope, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

pub const WEB_ACCESS_EXTENSION_ID: &str = "web-access";
pub const WEB_SEARCH_CAPABILITY_ID: &str = "web-access.search";
pub const WEB_GET_CONTENT_CAPABILITY_ID: &str = "web-access.get_content";

const EXA_MCP_URL: &str = "https://mcp.exa.ai/mcp";
pub const EXA_MCP_HOST: &str = "mcp.exa.ai";
pub const NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;
const DEFAULT_NUM_RESULTS: u64 = 5;
const MAX_NUM_RESULTS: u64 = 20;
const MAX_QUERIES: usize = 10;
const MAX_QUERY_CHARS: usize = 500;
const MAX_DOMAIN_FILTERS: usize = 20;
const MAX_DOMAIN_CHARS: usize = 200;
const MAX_STORED_RESPONSES: usize = 100;
const DEFAULT_CONTEXT_CHARS: u64 = 3_000;
const INCLUDE_CONTENT_CONTEXT_CHARS: u64 = 50_000;
const DEFAULT_TIMEOUT_MS: u32 = 60_000;
const RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_BLOCKING_HTTP_EGRESS: usize = 8;

#[derive(Debug, Default)]
pub struct WebAccessExecutor {
    stored: Mutex<StoredResponseCache>,
}

#[derive(Debug, Default)]
struct StoredResponseCache {
    entries: HashMap<String, Arc<StoredWebSearch>>,
    order: VecDeque<String>,
}

#[derive(Debug)]
struct StoredWebSearch {
    queries: Vec<StoredQuery>,
}

#[derive(Debug, Clone)]
struct StoredQuery {
    query: String,
    results: Vec<SearchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    title: String,
    url: String,
    content: String,
}

pub struct WebAccessDispatchRequest<'a> {
    pub capability_id: &'a CapabilityId,
    pub scope: &'a ResourceScope,
    pub input: &'a Value,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebAccessDispatchResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("web access dispatch failed: {kind}")]
pub struct WebAccessDispatchError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl WebAccessDispatchError {
    fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

impl WebAccessExecutor {
    pub async fn dispatch(
        &self,
        request: WebAccessDispatchRequest<'_>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        match request.capability_id.as_str() {
            WEB_SEARCH_CAPABILITY_ID => self.search(request).await,
            WEB_GET_CONTENT_CAPABILITY_ID => self.get_content(request),
            _ => Err(WebAccessDispatchError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )),
        }
    }

    async fn search(
        &self,
        request: WebAccessDispatchRequest<'_>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        let provider = optional_string(request.input, "provider")?.unwrap_or_else(|| "auto".into());
        match provider.as_str() {
            "auto" | "exa_mcp" => self.search_exa_mcp(request).await,
            "brave" => Err(WebAccessDispatchError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )),
            _ => Err(input_error()),
        }
    }

    fn get_content(
        &self,
        request: WebAccessDispatchRequest<'_>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        let response_id = required_string(request.input, "response_id")?;
        let stored = self
            .stored
            .lock()
            .map_err(|_| operation_error())?
            .get(response_id)
            .ok_or_else(operation_error)?;
        let query_selector = optional_string(request.input, "query")?;
        let url_selector = optional_string(request.input, "url")?;
        let url_index = optional_u64(request.input, "url_index")?;

        let selected_query = if let Some(query) = query_selector {
            stored
                .queries
                .iter()
                .find(|item| item.query == query)
                .ok_or_else(operation_error)?
        } else {
            stored.queries.first().ok_or_else(operation_error)?
        };
        let selected = if let Some(url) = url_selector {
            selected_query
                .results
                .iter()
                .find(|item| item.url == url)
                .ok_or_else(operation_error)?
        } else {
            let index = url_index.unwrap_or(0);
            let index = usize::try_from(index).map_err(|_| input_error())?;
            selected_query
                .results
                .get(index)
                .ok_or_else(operation_error)?
        };

        Ok(WebAccessDispatchResult {
            output: json!({
                "response_id": response_id,
                "query": selected_query.query,
                "title": selected.title,
                "url": selected.url,
                "content": selected.content,
            }),
            usage: ResourceUsage::default(),
        })
    }

    async fn search_exa_mcp(
        &self,
        request: WebAccessDispatchRequest<'_>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        let egress = request
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| WebAccessDispatchError::new(RuntimeDispatchErrorKind::NetworkDenied))?
            .clone();
        let queries = query_list(request.input)?;
        let include_content = optional_bool(request.input, "include_content")?.unwrap_or(false);
        let num_results = optional_u64(request.input, "num_results")?
            .or_else(|| optional_u64(request.input, "count").ok().flatten())
            .unwrap_or(DEFAULT_NUM_RESULTS)
            .clamp(1, MAX_NUM_RESULTS);
        let domain_filter = bounded_string_array(
            request.input,
            "domain_filter",
            MAX_DOMAIN_FILTERS,
            MAX_DOMAIN_CHARS,
        )?;
        let recency_filter = optional_string(request.input, "recency_filter")?;

        let mut total_request_bytes = 0_u64;
        let mut output_queries = Vec::new();
        let mut stored_queries = Vec::new();
        for query in queries {
            let enriched_query = build_mcp_query(&query, recency_filter.as_deref(), &domain_filter);
            let response_text = call_exa_mcp(
                Arc::clone(&egress),
                request.capability_id,
                request.scope,
                &enriched_query,
                num_results,
                include_content,
            )
            .await
            .map_err(map_egress_error)?;
            total_request_bytes = total_request_bytes.saturating_add(response_text.request_bytes);
            let results = parse_mcp_results(&response_text.body)?;
            let answer = build_answer(&results);
            output_queries.push(json!({
                "query": query,
                "provider_used": "exa_mcp",
                "answer": answer,
                "results": results.iter().enumerate().map(|(index, result)| json!({
                    "index": index,
                    "title": result.title,
                    "url": result.url,
                    "snippet": snippet(&result.content, 500),
                })).collect::<Vec<_>>(),
            }));
            stored_queries.push(StoredQuery { query, results });
        }

        let response_id = new_response_id();
        self.stored.lock().map_err(|_| operation_error())?.insert(
            response_id.clone(),
            Arc::new(StoredWebSearch {
                queries: stored_queries,
            }),
        );

        let output = json!({
            "response_id": response_id,
            "provider_used": "exa_mcp",
            "queries": output_queries,
        });
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0);
        Ok(WebAccessDispatchResult {
            output,
            usage: ResourceUsage {
                output_bytes,
                network_egress_bytes: total_request_bytes,
                ..ResourceUsage::default()
            },
        })
    }
}

struct EgressText {
    body: String,
    request_bytes: u64,
}

impl StoredResponseCache {
    fn get(&self, response_id: &str) -> Option<Arc<StoredWebSearch>> {
        self.entries.get(response_id).cloned()
    }

    fn insert(&mut self, response_id: String, stored: Arc<StoredWebSearch>) {
        if !self.entries.contains_key(&response_id) {
            self.order.push_back(response_id.clone());
        }
        self.entries.insert(response_id, stored);
        while self.entries.len() > MAX_STORED_RESPONSES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}

fn new_response_id() -> String {
    format!("web_{}", InvocationId::new())
}

async fn call_exa_mcp(
    egress: Arc<dyn RuntimeHttpEgress>,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    query: &str,
    num_results: u64,
    include_content: bool,
) -> Result<EgressText, RuntimeHttpEgressError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "numResults": num_results,
                "livecrawl": "fallback",
                "type": "auto",
                "contextMaxCharacters": if include_content {
                    INCLUDE_CONTENT_CONTEXT_CHARS
                } else {
                    DEFAULT_CONTEXT_CHARS
                },
            }
        }
    });
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: scope.clone(),
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: EXA_MCP_URL.to_string(),
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            (
                "accept".to_string(),
                "application/json, text/event-stream".to_string(),
            ),
        ],
        body: serde_json::to_vec(&body).map_err(|_| RuntimeHttpEgressError::Request {
            reason: "invalid_json".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })?,
        network_policy: exa_mcp_network_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let response = execute_runtime_http(http_request, egress).await?;
    let request_bytes = response.request_bytes;
    let body = String::from_utf8(response.body).map_err(|_| RuntimeHttpEgressError::Response {
        reason: "invalid_utf8".to_string(),
        request_bytes,
        response_bytes: response.response_bytes,
    })?;
    Ok(EgressText {
        body: extract_mcp_text(&body).ok_or_else(|| RuntimeHttpEgressError::Response {
            reason: "invalid_mcp_response".to_string(),
            request_bytes,
            response_bytes: response.response_bytes,
        })?,
        request_bytes,
    })
}

async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
    static HTTP_EGRESS_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let _permit = HTTP_EGRESS_SEMAPHORE
        .get_or_init(|| tokio::sync::Semaphore::new(MAX_BLOCKING_HTTP_EGRESS))
        .acquire()
        .await
        .map_err(|_| RuntimeHttpEgressError::Network {
            reason: "worker_join".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })?;
    tokio::task::spawn_blocking(move || egress.execute(request))
        .await
        .map_err(|_| RuntimeHttpEgressError::Network {
            reason: "worker_join".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })?
}

fn extract_mcp_text(body: &str) -> Option<String> {
    for line in body.lines().filter_map(|line| line.strip_prefix("data:")) {
        if let Some(text) = text_from_mcp_json(line.trim()) {
            return Some(text);
        }
    }
    text_from_mcp_json(body)
}

fn text_from_mcp_json(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    if value.get("error").is_some() || value.pointer("/result/isError") == Some(&Value::Bool(true))
    {
        return None;
    }
    value
        .pointer("/result/content")?
        .as_array()?
        .iter()
        .find_map(|item| {
            (item.get("type")?.as_str()? == "text")
                .then(|| item.get("text")?.as_str().map(str::to_string))?
        })
}

fn parse_mcp_results(text: &str) -> Result<Vec<SearchResult>, WebAccessDispatchError> {
    Ok(text
        .split("\nTitle: ")
        .map(|block| block.trim_start_matches("Title: "))
        .filter_map(parse_mcp_block)
        .collect::<Vec<_>>())
}

fn parse_mcp_block(block: &str) -> Option<SearchResult> {
    let title = line_value(block, "Title:").unwrap_or_else(|| {
        block
            .lines()
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    });
    let url = line_value(block, "URL:")?;
    let content = if let Some(index) = block.find("\nText: ") {
        block[index + "\nText: ".len()..].trim()
    } else if let Some(index) = block.find("\nHighlights:\n") {
        block[index + "\nHighlights:\n".len()..].trim()
    } else {
        ""
    }
    .trim_end_matches("---")
    .trim()
    .to_string();
    Some(SearchResult {
        title,
        url,
        content,
    })
}

fn line_value(block: &str, prefix: &str) -> Option<String> {
    block.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn build_answer(results: &[SearchResult]) -> String {
    results
        .iter()
        .filter_map(|result| {
            let text = snippet(&result.content, 500);
            (!text.is_empty())
                .then(|| format!("{}\nSource: {} ({})", text, result.title, result.url))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_mcp_query(query: &str, recency_filter: Option<&str>, domain_filter: &[String]) -> String {
    let mut parts = vec![query.to_string()];
    for domain in domain_filter {
        if let Some(excluded) = domain.strip_prefix('-') {
            parts.push(format!("-site:{excluded}"));
        } else {
            parts.push(format!("site:{domain}"));
        }
    }
    if let Some(filter) = recency_filter {
        match filter {
            "day" => parts.push("past 24 hours".to_string()),
            "week" => parts.push("past week".to_string()),
            "month" => parts.push("past month".to_string()),
            "year" => parts.push("past year".to_string()),
            _ => {}
        }
    }
    parts.join(" ")
}

fn snippet(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out.trim().to_string()
}

fn query_list(input: &Value) -> Result<Vec<String>, WebAccessDispatchError> {
    if let Some(query) = optional_string(input, "query")? {
        let query = bounded_trimmed_string(&query, MAX_QUERY_CHARS)?;
        return Ok(vec![query]);
    }
    let queries = bounded_string_array(input, "queries", MAX_QUERIES, MAX_QUERY_CHARS)?
        .into_iter()
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty())
        .collect::<Vec<_>>();
    if queries.is_empty() {
        return Err(input_error());
    }
    Ok(queries)
}

fn required_string<'a>(input: &'a Value, key: &str) -> Result<&'a str, WebAccessDispatchError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(input_error)
}

fn optional_string(input: &Value, key: &str) -> Result<Option<String>, WebAccessDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(input_error)
}

fn optional_bool(input: &Value, key: &str) -> Result<Option<bool>, WebAccessDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(input_error)
}

fn optional_u64(input: &Value, key: &str) -> Result<Option<u64>, WebAccessDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(input_error)
}

fn bounded_string_array(
    input: &Value,
    key: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, WebAccessDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(input_error)?;
    if values.len() > max_items {
        return Err(input_error());
    }
    values
        .iter()
        .map(|item| {
            let value = item.as_str().ok_or_else(input_error)?;
            bounded_trimmed_string(value, max_chars)
        })
        .collect()
}

fn bounded_trimmed_string(value: &str, max_chars: usize) -> Result<String, WebAccessDispatchError> {
    if value.chars().count() > max_chars {
        return Err(input_error());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err(input_error());
    }
    Ok(value.to_string())
}

fn exa_mcp_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: EXA_MCP_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

fn map_egress_error(error: RuntimeHttpEgressError) -> WebAccessDispatchError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OutputDecode,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    WebAccessDispatchError::new(kind).with_usage(ResourceUsage {
        network_egress_bytes: error.request_bytes(),
        ..ResourceUsage::default()
    })
}

fn input_error() -> WebAccessDispatchError {
    WebAccessDispatchError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn operation_error() -> WebAccessDispatchError {
    WebAccessDispatchError::new(RuntimeDispatchErrorKind::OperationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, RuntimeHttpEgressResponse, UserId};
    use std::sync::Mutex as StdMutex;

    fn scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .unwrap()
    }

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).unwrap()
    }

    fn request<'a>(
        capability_id: &'a CapabilityId,
        scope: &'a ResourceScope,
        input: &'a Value,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> WebAccessDispatchRequest<'a> {
        WebAccessDispatchRequest {
            capability_id,
            scope,
            input,
            runtime_http_egress,
        }
    }

    fn seed_executor() -> (WebAccessExecutor, String) {
        let executor = WebAccessExecutor::default();
        let response_id = "web_seed".to_string();
        executor.stored.lock().unwrap().insert(
            response_id.clone(),
            Arc::new(StoredWebSearch {
                queries: vec![StoredQuery {
                    query: "rust async".to_string(),
                    results: vec![
                        SearchResult {
                            title: "First".to_string(),
                            url: "https://one.test".to_string(),
                            content: "first body".to_string(),
                        },
                        SearchResult {
                            title: "Second".to_string(),
                            url: "https://two.test".to_string(),
                            content: "second body".to_string(),
                        },
                    ],
                }],
            }),
        );
        (executor, response_id)
    }

    struct RecordingEgress {
        response: StdMutex<Option<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
    }

    impl RecordingEgress {
        fn ok(body: Value) -> Self {
            let bytes = serde_json::to_vec(&body).unwrap();
            Self {
                response: StdMutex::new(Some(Ok(RuntimeHttpEgressResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: bytes,
                    saved_body: None,
                    request_bytes: 10,
                    response_bytes: 20,
                    redaction_applied: false,
                }))),
            }
        }
    }

    impl RuntimeHttpEgress for RecordingEgress {
        fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.response.lock().unwrap().take().unwrap()
        }
    }

    #[test]
    fn extracts_text_from_sse_mcp_response() {
        let body = r#"event: message
data: {"result":{"content":[{"type":"text","text":"Title: Example\nURL: https://example.com\nText: Body"}]}}
"#;
        assert_eq!(
            extract_mcp_text(body).as_deref(),
            Some("Title: Example\nURL: https://example.com\nText: Body")
        );
    }

    #[test]
    fn extracts_no_text_from_mcp_error_responses() {
        assert_eq!(text_from_mcp_json(r#"{"error":{"message":"bad"}}"#), None);
        assert_eq!(
            text_from_mcp_json(
                r#"{"result":{"isError":true,"content":[{"type":"text","text":"bad"}]}}"#
            ),
            None
        );
    }

    #[test]
    fn parses_exa_mcp_result_blocks() {
        let parsed = parse_mcp_results(
            "Title: One\nURL: https://one.test\nText: First body\n---\nTitle: Two\nURL: https://two.test\nHighlights:\nSecond body",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "One");
        assert_eq!(parsed[0].content, "First body");
        assert_eq!(parsed[1].url, "https://two.test");
    }

    #[test]
    fn parses_empty_mcp_results_as_empty_result_list() {
        assert_eq!(parse_mcp_results("").unwrap(), Vec::new());
    }

    #[test]
    fn query_list_rejects_blank_and_over_limit_inputs() {
        assert_eq!(
            query_list(&json!({"query":"  "})).unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
        assert_eq!(
            query_list(&json!({"queries":[" ",""]})).unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
        assert_eq!(
            query_list(&json!({"queries":["a","b","c","d","e","f","g","h","i","j","k"]}))
                .unwrap_err()
                .kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
        assert_eq!(
            query_list(&json!({"query":"x".repeat(MAX_QUERY_CHARS + 1)}))
                .unwrap_err()
                .kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn domain_filter_rejects_over_limit_inputs() {
        assert_eq!(
            bounded_string_array(
                &json!({"domain_filter": ["a","b","c","d","e","f","g","h","i","j","k","l","m","n","o","p","q","r","s","t","u"]}),
                "domain_filter",
                MAX_DOMAIN_FILTERS,
                MAX_DOMAIN_CHARS,
            )
            .unwrap_err()
            .kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn build_mcp_query_includes_domains_and_recency_filters() {
        assert_eq!(
            build_mcp_query(
                "rust",
                Some("day"),
                &["example.com".into(), "-old.test".into()]
            ),
            "rust site:example.com -site:old.test past 24 hours"
        );
        assert!(build_mcp_query("rust", Some("week"), &[]).ends_with("past week"));
        assert!(build_mcp_query("rust", Some("month"), &[]).ends_with("past month"));
        assert!(build_mcp_query("rust", Some("year"), &[]).ends_with("past year"));
    }

    #[test]
    fn get_content_rejects_missing_response_id() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[test]
    fn get_content_returns_unknown_response_id_error() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id":"missing"});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn get_content_rejects_unknown_query_selector() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "query": "missing"});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn get_content_returns_result_by_url_index() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url_index": 1});

        let result = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap();

        assert_eq!(result.output["url"], "https://two.test");
        assert_eq!(result.output["content"], "second body");
    }

    #[test]
    fn get_content_returns_result_by_url_selector() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url": "https://one.test"});

        let result = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap();

        assert_eq!(result.output["title"], "First");
    }

    #[test]
    fn get_content_rejects_out_of_bounds_url_index() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url_index": 99});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn stored_response_cache_evicts_oldest_entries() {
        let mut cache = StoredResponseCache::default();
        for index in 0..=MAX_STORED_RESPONSES {
            cache.insert(
                format!("web_{index}"),
                Arc::new(StoredWebSearch {
                    queries: Vec::new(),
                }),
            );
        }

        assert!(cache.get("web_0").is_none());
        assert!(cache.get(&format!("web_{MAX_STORED_RESPONSES}")).is_some());
    }

    #[tokio::test]
    async fn dispatch_returns_undeclared_capability_for_unknown_id() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id("web-access.unknown");
        let scope = scope();
        let input = json!({});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::UndeclaredCapability);
    }

    #[tokio::test]
    async fn brave_provider_returns_undeclared_capability() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust", "provider":"brave"});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::UndeclaredCapability);
    }

    #[tokio::test]
    async fn search_returns_network_denied_when_egress_is_none() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust"});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    }

    #[tokio::test]
    async fn search_accepts_zero_result_response_and_stores_random_response_id() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust"});
        let egress = Arc::new(RecordingEgress::ok(json!({
            "result": {"content": [{"type": "text", "text": ""}]}
        })));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        let response_id = result.output["response_id"].as_str().unwrap();
        assert!(response_id.starts_with("web_"));
        assert_ne!(response_id, "web_0");
        assert!(
            result.output["queries"][0]["results"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
