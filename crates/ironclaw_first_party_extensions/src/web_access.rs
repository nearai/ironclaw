use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

pub const WEB_ACCESS_EXTENSION_ID: &str = "web-access";
pub const WEB_SEARCH_CAPABILITY_ID: &str = "web-access.search";
pub const WEB_GET_CONTENT_CAPABILITY_ID: &str = "web-access.get_content";

const EXA_MCP_URL: &str = "https://mcp.exa.ai/mcp";
const DEFAULT_NUM_RESULTS: u64 = 5;
const MAX_NUM_RESULTS: u64 = 20;
const DEFAULT_CONTEXT_CHARS: u64 = 3_000;
const INCLUDE_CONTENT_CONTEXT_CHARS: u64 = 50_000;
const DEFAULT_TIMEOUT_MS: u32 = 60_000;
const RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct WebAccessExecutor {
    stored: Mutex<HashMap<String, StoredWebSearch>>,
    next_response_id: AtomicU64,
}

#[derive(Debug, Clone)]
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
                RuntimeDispatchErrorKind::Client,
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
            .cloned()
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
        let domain_filter = string_array(request.input, "domain_filter")?;
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

        let response_id = format!(
            "web_{}",
            self.next_response_id.fetch_add(1, Ordering::Relaxed)
        );
        self.stored.lock().map_err(|_| operation_error())?.insert(
            response_id.clone(),
            StoredWebSearch {
                queries: stored_queries,
            },
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
    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|_| RuntimeHttpEgressError::Network {
            reason: "worker_join".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })??;
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
    let results = text
        .split("\nTitle: ")
        .map(|block| block.trim_start_matches("Title: "))
        .filter_map(parse_mcp_block)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Err(WebAccessDispatchError::new(
            RuntimeDispatchErrorKind::OutputDecode,
        ));
    }
    Ok(results)
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
    text.chars()
        .take(max_chars)
        .collect::<String>()
        .replace(char::is_control, " ")
        .trim()
        .to_string()
}

fn query_list(input: &Value) -> Result<Vec<String>, WebAccessDispatchError> {
    if let Some(query) = optional_string(input, "query")? {
        if query.trim().is_empty() {
            return Err(input_error());
        }
        return Ok(vec![query.trim().to_string()]);
    }
    let queries = string_array(input, "queries")?
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

fn string_array(input: &Value, key: &str) -> Result<Vec<String>, WebAccessDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(input_error)?
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(input_error))
        .collect()
}

fn exa_mcp_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "mcp.exa.ai".to_string(),
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
}
