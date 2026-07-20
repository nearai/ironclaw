// arch-exempt: large_file, web-access Exa SSE regression coverage stays with this tool harness, plan #5573
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::IpAddr,
    sync::{Arc, Mutex},
};

use futures_util::FutureExt as _;
use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    ResourceScope, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};
use url::{Host, Url};

use crate::latency::{
    FirstPartyToolLatencyFields, FirstPartyToolLatencyMetrics, json_bytes, started_at,
    trace_tool_error, trace_tool_ok,
};

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
const MAX_FETCH_URLS: usize = 10;
const MAX_URL_CHARS: usize = 2_048;
const MAX_STORED_RESPONSES: usize = 100;
/// 50 MiB total content budget across all cached responses.
const MAX_STORED_CONTENT_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_CONTEXT_CHARS: u64 = 3_000;
const INCLUDE_CONTENT_CONTEXT_CHARS: u64 = 50_000;
const DEFAULT_FETCH_MAX_CHARACTERS: u64 = 50_000;
const MAX_FETCH_MAX_CHARACTERS: u64 = 50_000;
const DEFAULT_TIMEOUT_MS: u32 = 60_000;
const RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct WebAccessExecutor {
    stored: Mutex<StoredResponseCache>,
}

#[derive(Debug, Default)]
struct StoredResponseCache {
    entries: HashMap<String, Arc<StoredWebSearch>>,
    order: VecDeque<String>,
    total_content_bytes: u64,
}

#[derive(Debug)]
struct StoredWebSearch {
    queries: Vec<StoredQuery>,
}

impl StoredWebSearch {
    fn content_bytes(&self) -> u64 {
        self.queries
            .iter()
            .flat_map(|q| q.results.iter())
            .map(|r| r.content.len() as u64)
            .sum()
    }
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

fn web_access_latency_started_at() -> Option<std::time::Instant> {
    started_at()
}

fn trace_web_access_latency_ok(
    operation: &'static str,
    fields: Option<&FirstPartyToolLatencyFields<'_>>,
    started_at: Option<std::time::Instant>,
    request_bytes: u64,
    output_bytes: u64,
) {
    trace_tool_ok(
        "web_access_first_party_tool",
        operation,
        fields,
        started_at,
        FirstPartyToolLatencyMetrics {
            request_bytes,
            output_bytes,
            ..FirstPartyToolLatencyMetrics::default()
        },
    );
}

fn trace_web_access_latency_error(
    operation: &'static str,
    fields: Option<&FirstPartyToolLatencyFields<'_>>,
    started_at: Option<std::time::Instant>,
    error_kind: &str,
    request_bytes: u64,
    output_bytes: u64,
) {
    trace_tool_error(
        "web_access_first_party_tool",
        operation,
        fields,
        started_at,
        error_kind,
        FirstPartyToolLatencyMetrics {
            request_bytes,
            output_bytes,
            ..FirstPartyToolLatencyMetrics::default()
        },
    );
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

    /// Override the `network_egress_bytes` in the usage, preserving the error kind.
    fn with_accumulated_bytes(self, bytes: u64) -> Self {
        self.with_usage(ResourceUsage::default().set_network_egress_bytes(bytes))
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
        let latency_fields = FirstPartyToolLatencyFields::from_input(
            request.capability_id,
            request.scope,
            request.input,
        );
        let started_at = web_access_latency_started_at();
        let result = match request.capability_id.as_str() {
            WEB_SEARCH_CAPABILITY_ID => self.search(request).await,
            WEB_GET_CONTENT_CAPABILITY_ID => self.get_content(request).await,
            _ => Err(WebAccessDispatchError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )),
        };
        match &result {
            Ok(result) => trace_web_access_latency_ok(
                "dispatch",
                latency_fields.as_ref(),
                started_at,
                result.usage.network_egress_bytes,
                result.usage.output_bytes,
            ),
            Err(error) => trace_web_access_latency_error(
                "dispatch",
                latency_fields.as_ref(),
                started_at,
                error.kind().as_str(),
                error
                    .usage()
                    .map(|usage| usage.network_egress_bytes)
                    .unwrap_or(0),
                error.usage().map(|usage| usage.output_bytes).unwrap_or(0),
            ),
        }
        result
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

    async fn get_content(
        &self,
        request: WebAccessDispatchRequest<'_>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        if request.input.get("response_id").is_some() {
            reject_keys(request.input, &["urls", "max_characters", "maxCharacters"])?;
            return self.get_cached_content(request);
        }
        reject_keys(request.input, &["query", "url_index"])?;
        let urls = fetch_url_list(request.input)?;
        if urls.is_empty() {
            return Err(input_error());
        }
        self.fetch_content(request, urls).await
    }

    fn get_cached_content(
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
        if url_selector.is_some() && url_index.is_some() {
            return Err(input_error());
        }
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
                "provider_used": "cache",
                "title": selected.title,
                "url": selected.url,
                "content": selected.content,
                "contents": [{
                    "title": selected.title,
                    "url": selected.url,
                    "content": selected.content,
                }],
            }),
            usage: ResourceUsage::default(),
        })
    }

    async fn fetch_content(
        &self,
        request: WebAccessDispatchRequest<'_>,
        urls: Vec<String>,
    ) -> Result<WebAccessDispatchResult, WebAccessDispatchError> {
        let max_characters = fetch_max_characters(request.input)?;
        let egress = request
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| WebAccessDispatchError::new(RuntimeDispatchErrorKind::NetworkDenied))?
            .clone();
        let response_text = call_exa_mcp_fetch(
            egress,
            request.capability_id,
            request.scope,
            &urls,
            max_characters,
        )
        .await
        .map_err(|e| {
            let total_bytes = e.total_bytes();
            map_egress_error(e.inner).with_accumulated_bytes(total_bytes)
        })?;
        let results = parse_fetch_results(&response_text.body, &urls)?;
        let Some(first) = results.first() else {
            return Err(operation_error());
        };
        let output = json!({
            "provider_used": "exa_mcp",
            "title": first.title,
            "url": first.url,
            "content": first.content,
            "contents": results.iter().map(|result| json!({
                "title": result.title,
                "url": result.url,
                "content": result.content,
            })).collect::<Vec<_>>(),
        });
        let output_bytes = json_bytes(&output);
        Ok(WebAccessDispatchResult {
            output,
            usage: ResourceUsage::default()
                .set_output_bytes(output_bytes)
                .set_network_egress_bytes(response_text.request_bytes),
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
            let response_text = call_exa_mcp_search(
                Arc::clone(&egress),
                request.capability_id,
                request.scope,
                &enriched_query,
                num_results,
                include_content,
            )
            .await
            .map_err(|e| {
                let combined = total_request_bytes.saturating_add(e.total_bytes());
                map_egress_error(e.inner).with_accumulated_bytes(combined)
            })?;
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
        let output_bytes = json_bytes(&output);
        Ok(WebAccessDispatchResult {
            output,
            usage: ResourceUsage::default()
                .set_output_bytes(output_bytes)
                .set_network_egress_bytes(total_request_bytes),
        })
    }
}

struct EgressText {
    body: String,
    request_bytes: u64,
}

/// Error returned from `call_exa_mcp`, carrying the bytes spent before failure.
struct EgressCallError {
    inner: RuntimeHttpEgressError,
    /// Bytes spent in partial handshake/prior steps before the failing request.
    prior_request_bytes: u64,
}

impl EgressCallError {
    fn new(inner: RuntimeHttpEgressError) -> Self {
        Self {
            inner,
            prior_request_bytes: 0,
        }
    }

    fn with_prior(mut self, bytes: u64) -> Self {
        self.prior_request_bytes = bytes;
        self
    }

    fn total_bytes(&self) -> u64 {
        self.prior_request_bytes
            .saturating_add(self.inner.request_bytes())
    }
}

impl StoredResponseCache {
    fn get(&self, response_id: &str) -> Option<Arc<StoredWebSearch>> {
        self.entries.get(response_id).cloned()
    }

    fn insert(&mut self, response_id: String, stored: Arc<StoredWebSearch>) {
        let new_bytes = stored.content_bytes();
        if let Some(old) = self.entries.get(&response_id) {
            self.total_content_bytes = self.total_content_bytes.saturating_sub(old.content_bytes());
        } else {
            self.order.push_back(response_id.clone());
        }
        self.entries.insert(response_id, stored);
        self.total_content_bytes = self.total_content_bytes.saturating_add(new_bytes);
        while (self.entries.len() > MAX_STORED_RESPONSES
            || self.total_content_bytes > MAX_STORED_CONTENT_BYTES)
            && !self.order.is_empty()
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_content_bytes = self
                    .total_content_bytes
                    .saturating_sub(removed.content_bytes());
            }
        }
    }
}

fn new_response_id() -> String {
    format!("web_{}", InvocationId::new())
}

/// Build a JSON-RPC 2.0 request body. `id` is `None` for notifications.
fn json_rpc_body(
    id: Option<u64>,
    method: &str,
    params: Option<Value>,
) -> Result<Vec<u8>, RuntimeHttpEgressError> {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    if let Some(id) = id {
        obj.insert(
            "id".to_string(),
            Value::Number(serde_json::Number::from(id)),
        );
    }
    obj.insert("method".to_string(), Value::String(method.to_string()));
    if let Some(params) = params {
        obj.insert("params".to_string(), params);
    }
    serde_json::to_vec(&Value::Object(obj)).map_err(|_| RuntimeHttpEgressError::Request {
        reason: "invalid_json".to_string(),
        request_bytes: 0,
        response_bytes: 0,
    })
}

fn mcp_base_headers(session_id: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        (
            "accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
    ];
    if let Some(id) = session_id {
        headers.push(("mcp-session-id".to_string(), id.to_string()));
    }
    headers
}

fn mcp_initialize_params() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"roots": {"listChanged": false}, "sampling": {}},
        "clientInfo": {"name": "ironclaw", "version": env!("CARGO_PKG_VERSION")}
    })
}

fn is_valid_mcp_initialize_response(body: &[u8]) -> bool {
    let Some(value) = mcp_json_value_from_body(body) else {
        return false;
    };
    value.get("error").is_none() && value.get("result").is_some_and(Value::is_object)
}

fn mcp_json_value_from_body(body: &[u8]) -> Option<Value> {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return Some(value);
    }

    let body = std::str::from_utf8(body).ok()?;
    let mut event_data = String::new();
    for line in body.lines() {
        if line.is_empty() {
            if let Some(value) = mcp_json_value_from_sse_event(&event_data) {
                return Some(value);
            }
            event_data.clear();
            continue;
        }

        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data.is_empty() {
            continue;
        }
        if !event_data.is_empty() {
            event_data.push('\n');
        }
        event_data.push_str(data);
    }
    mcp_json_value_from_sse_event(&event_data)
}

fn mcp_json_value_from_sse_event(data: &str) -> Option<Value> {
    if data.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(data).ok()
}

async fn call_exa_mcp_search(
    egress: Arc<dyn RuntimeHttpEgress>,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    query: &str,
    num_results: u64,
    include_content: bool,
) -> Result<EgressText, EgressCallError> {
    let arguments = json!({
        "query": query,
        "numResults": num_results,
        "livecrawl": "fallback",
        "type": "auto",
        "contextMaxCharacters": if include_content {
            INCLUDE_CONTENT_CONTEXT_CHARS
        } else {
            DEFAULT_CONTEXT_CHARS
        },
    });
    call_exa_mcp_tool(egress, capability_id, scope, "web_search_exa", arguments).await
}

async fn call_exa_mcp_fetch(
    egress: Arc<dyn RuntimeHttpEgress>,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    urls: &[String],
    max_characters: u64,
) -> Result<EgressText, EgressCallError> {
    call_exa_mcp_tool(
        egress,
        capability_id,
        scope,
        "web_fetch_exa",
        json!({
            "urls": urls,
            "maxCharacters": max_characters,
        }),
    )
    .await
}

async fn call_exa_mcp_tool(
    egress: Arc<dyn RuntimeHttpEgress>,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    tool_name: &str,
    arguments: Value,
) -> Result<EgressText, EgressCallError> {
    let latency_fields = FirstPartyToolLatencyFields::from_input(capability_id, scope, &arguments);
    let mut prior_bytes = 0_u64;

    // 1. initialize — required before tools/call on compliant MCP servers.
    let init_body = json_rpc_body(Some(1), "initialize", Some(mcp_initialize_params()))
        .map_err(EgressCallError::new)?;
    let init_req = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: scope.clone(),
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: EXA_MCP_URL.to_string(),
        headers: mcp_base_headers(None),
        body: init_body,
        network_policy: exa_mcp_network_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let init_started_at = web_access_latency_started_at();
    let init_resp = match execute_runtime_http(init_req, Arc::clone(&egress)).await {
        Ok(response) => {
            trace_web_access_latency_ok(
                "exa_initialize",
                latency_fields.as_ref(),
                init_started_at,
                response.request_bytes,
                response.response_bytes,
            );
            response
        }
        Err(error) => {
            trace_web_access_latency_error(
                "exa_initialize",
                latency_fields.as_ref(),
                init_started_at,
                error.stable_runtime_reason(),
                prior_bytes.saturating_add(error.request_bytes()),
                error.response_bytes(),
            );
            return Err(EgressCallError::new(error).with_prior(prior_bytes));
        }
    };
    prior_bytes = prior_bytes.saturating_add(init_resp.request_bytes);

    // Extract Mcp-Session-Id for reuse in subsequent requests.
    let session_id: Option<String> = init_resp
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
        .map(|(_, v)| v.clone());

    // Check initialize response before moving to initialized notification.
    let init_parse_started_at = web_access_latency_started_at();
    if !is_valid_mcp_initialize_response(&init_resp.body) {
        let error = RuntimeHttpEgressError::Response {
            reason: "invalid_mcp_response".to_string(),
            request_bytes: prior_bytes,
            response_bytes: init_resp.response_bytes,
        };
        trace_web_access_latency_error(
            "exa_initialize_parse",
            latency_fields.as_ref(),
            init_parse_started_at,
            error.stable_runtime_reason(),
            error.request_bytes(),
            error.response_bytes(),
        );
        return Err(EgressCallError::new(error).with_prior(0));
    }

    // 2. notifications/initialized — no id (notification, not a request).
    let notif_body = json_rpc_body(None, "notifications/initialized", None)
        .map_err(|e| EgressCallError::new(e).with_prior(prior_bytes))?;
    let notif_req = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: scope.clone(),
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: EXA_MCP_URL.to_string(),
        headers: mcp_base_headers(session_id.as_deref()),
        body: notif_body,
        network_policy: exa_mcp_network_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let notif_started_at = web_access_latency_started_at();
    let notif_resp = match execute_runtime_http(notif_req, Arc::clone(&egress)).await {
        Ok(response) => {
            trace_web_access_latency_ok(
                "exa_initialized",
                latency_fields.as_ref(),
                notif_started_at,
                response.request_bytes,
                response.response_bytes,
            );
            response
        }
        Err(error) => {
            trace_web_access_latency_error(
                "exa_initialized",
                latency_fields.as_ref(),
                notif_started_at,
                error.stable_runtime_reason(),
                prior_bytes.saturating_add(error.request_bytes()),
                error.response_bytes(),
            );
            return Err(EgressCallError::new(error).with_prior(prior_bytes));
        }
    };
    prior_bytes = prior_bytes.saturating_add(notif_resp.request_bytes);

    // 3. tools/call with session ID.
    let call_params = json!({
        "name": tool_name,
        "arguments": arguments,
    });
    let call_body = json_rpc_body(Some(2), "tools/call", Some(call_params))
        .map_err(|e| EgressCallError::new(e).with_prior(prior_bytes))?;
    let call_req = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: scope.clone(),
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: EXA_MCP_URL.to_string(),
        headers: mcp_base_headers(session_id.as_deref()),
        body: call_body,
        network_policy: exa_mcp_network_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let call_started_at = web_access_latency_started_at();
    let call_resp = match execute_runtime_http(call_req, egress).await {
        Ok(response) => {
            trace_web_access_latency_ok(
                "exa_tool_call",
                latency_fields.as_ref(),
                call_started_at,
                response.request_bytes,
                response.response_bytes,
            );
            response
        }
        Err(error) => {
            trace_web_access_latency_error(
                "exa_tool_call",
                latency_fields.as_ref(),
                call_started_at,
                error.stable_runtime_reason(),
                prior_bytes.saturating_add(error.request_bytes()),
                error.response_bytes(),
            );
            return Err(EgressCallError::new(error).with_prior(prior_bytes));
        }
    };
    let call_request_bytes = call_resp.request_bytes;
    prior_bytes = prior_bytes.saturating_add(call_request_bytes);

    let parse_started_at = web_access_latency_started_at();
    let body = String::from_utf8(call_resp.body).map_err(|_| {
        let error = RuntimeHttpEgressError::Response {
            reason: "invalid_utf8".to_string(),
            request_bytes: prior_bytes,
            response_bytes: call_resp.response_bytes,
        };
        trace_web_access_latency_error(
            "exa_parse_tool_response",
            latency_fields.as_ref(),
            parse_started_at,
            error.stable_runtime_reason(),
            error.request_bytes(),
            error.response_bytes(),
        );
        EgressCallError::new(error)
    })?;
    let text = extract_mcp_text(&body).ok_or_else(|| {
        let error = RuntimeHttpEgressError::Response {
            reason: "invalid_mcp_response".to_string(),
            request_bytes: prior_bytes,
            response_bytes: call_resp.response_bytes,
        };
        trace_web_access_latency_error(
            "exa_parse_tool_response",
            latency_fields.as_ref(),
            parse_started_at,
            error.stable_runtime_reason(),
            error.request_bytes(),
            error.response_bytes(),
        );
        EgressCallError::new(error)
    })?;
    trace_web_access_latency_ok(
        "exa_parse_tool_response",
        latency_fields.as_ref(),
        parse_started_at,
        prior_bytes,
        text.len() as u64,
    );
    Ok(EgressText {
        body: text,
        request_bytes: prior_bytes,
    })
}

async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
    std::panic::AssertUnwindSafe(egress.execute(request))
        .catch_unwind()
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

fn parse_fetch_results(
    text: &str,
    requested_urls: &[String],
) -> Result<Vec<SearchResult>, WebAccessDispatchError> {
    let lines = text.lines().collect::<Vec<_>>();
    let requested = requested_urls
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if lines
        .iter()
        .any(|line| fetch_error_mentions_requested(line, &requested))
    {
        return Err(operation_error());
    }
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let next = lines.get(index + 1)?;
            let url = next.strip_prefix("URL: ")?.trim();
            (line.starts_with("# ") && requested.contains(url)).then_some(index)
        })
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    for (position, start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(lines.len());
        let title = lines[*start]
            .strip_prefix("# ")
            .unwrap_or(lines[*start])
            .trim()
            .to_string();
        let url = lines[*start + 1]
            .strip_prefix("URL: ")
            .unwrap_or(lines[*start + 1])
            .trim()
            .to_string();
        let content = lines[*start + 2..end].join("\n").trim().to_string();
        results.push(SearchResult {
            title,
            url,
            content,
        });
    }

    if !results.is_empty() {
        return Ok(results);
    }

    let Some(first_url) = requested_urls.first() else {
        return Err(input_error());
    };
    let title = text
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|value| !value.is_empty())
        .unwrap_or(first_url)
        .to_string();
    Ok(vec![SearchResult {
        title,
        url: first_url.clone(),
        content: text.trim().to_string(),
    }])
}

fn fetch_error_mentions_requested(line: &str, requested: &HashSet<&str>) -> bool {
    let Some(remainder) = line.strip_prefix("Error fetching ") else {
        return false;
    };
    let remainder = remainder.trim();
    requested.iter().any(|url| {
        remainder == *url
            || remainder
                .strip_prefix(*url)
                .is_some_and(|suffix| suffix.trim_start().starts_with(':'))
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

fn fetch_url_list(input: &Value) -> Result<Vec<String>, WebAccessDispatchError> {
    let single_url = optional_string(input, "url")?
        .map(|url| validated_fetch_url(&url))
        .transpose()?;
    let urls = bounded_string_array(input, "urls", MAX_FETCH_URLS, MAX_URL_CHARS)?
        .into_iter()
        .map(|url| validated_fetch_url(&url))
        .collect::<Result<Vec<_>, _>>()?;

    match (single_url, urls.is_empty()) {
        (Some(_), false) => Err(input_error()),
        (Some(url), true) => Ok(vec![url]),
        (None, false) => Ok(urls),
        (None, true) => Ok(Vec::new()),
    }
}

fn fetch_max_characters(input: &Value) -> Result<u64, WebAccessDispatchError> {
    let snake_case = optional_u64(input, "max_characters")?;
    let camel_case = optional_u64(input, "maxCharacters")?;
    match (snake_case, camel_case) {
        (Some(_), Some(_)) => Err(input_error()),
        (Some(0), None) | (None, Some(0)) => Err(input_error()),
        (Some(value), None) | (None, Some(value)) if value <= MAX_FETCH_MAX_CHARACTERS => Ok(value),
        (Some(_), None) | (None, Some(_)) => Err(input_error()),
        (None, None) => Ok(DEFAULT_FETCH_MAX_CHARACTERS),
    }
}

fn reject_keys(input: &Value, keys: &[&str]) -> Result<(), WebAccessDispatchError> {
    if keys.iter().any(|key| input.get(*key).is_some()) {
        return Err(input_error());
    }
    Ok(())
}

fn validated_fetch_url(value: &str) -> Result<String, WebAccessDispatchError> {
    let url = bounded_trimmed_string(value, MAX_URL_CHARS)?;
    let parsed = Url::parse(&url).map_err(|_| input_error())?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err(input_error());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(input_error());
    }
    if url.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(input_error());
    }
    let host = parsed.host().ok_or_else(input_error)?;
    if disallowed_fetch_host(host) {
        return Err(input_error());
    }
    Ok(url)
}

fn disallowed_fetch_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(host) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host == "localhost"
                || host.ends_with(".localhost")
                || host
                    .parse::<IpAddr>()
                    .map(fetch_ip_is_not_public)
                    .unwrap_or(false)
        }
        Host::Ipv4(ip) => fetch_ip_is_not_public(IpAddr::V4(ip)),
        Host::Ipv6(ip) => fetch_ip_is_not_public(IpAddr::V6(ip)),
    }
}

fn fetch_ip_is_not_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4() {
                return fetch_ip_is_not_public(IpAddr::V4(mapped));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
        }
    }
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
    WebAccessDispatchError::new(kind)
        .with_usage(ResourceUsage::default().set_network_egress_bytes(error.request_bytes()))
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
        responses: StdMutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
        requests: StdMutex<Vec<Value>>,
    }

    impl RecordingEgress {
        fn with_responses(
            responses: Vec<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>,
        ) -> Self {
            Self {
                responses: StdMutex::new(responses.into()),
                requests: StdMutex::new(Vec::new()),
            }
        }

        fn ok_json(body: Value) -> RuntimeHttpEgressResponse {
            let bytes = serde_json::to_vec(&body).unwrap();
            Self::ok_body(bytes, 20)
        }

        fn ok_body(body: Vec<u8>, response_bytes: u64) -> RuntimeHttpEgressResponse {
            RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body,
                saved_body: None,
                request_bytes: 10,
                response_bytes,
                redaction_applied: false,
            }
        }

        fn ok_sse_json(body: Value) -> RuntimeHttpEgressResponse {
            let body = format!("event: message\ndata: {body}\n");
            let bytes = body.into_bytes();
            let response_bytes = bytes.len() as u64;
            Self::ok_body(bytes, response_bytes)
        }

        fn ok_sse_data_lines(lines: &[&str]) -> RuntimeHttpEgressResponse {
            let mut body = String::from("event: message\n");
            for line in lines {
                body.push_str("data: ");
                body.push_str(line);
                body.push('\n');
            }
            body.push('\n');
            let bytes = body.into_bytes();
            let response_bytes = bytes.len() as u64;
            Self::ok_body(bytes, response_bytes)
        }

        fn accepted() -> RuntimeHttpEgressResponse {
            RuntimeHttpEgressResponse {
                status: 202,
                headers: Vec::new(),
                body: Vec::new(),
                saved_body: None,
                request_bytes: 5,
                response_bytes: 0,
                redaction_applied: false,
            }
        }

        /// Build a recording egress for a full MCP handshake + tools/call sequence.
        /// `tools_call_body` is returned for the final tools/call request.
        fn for_mcp_search(tools_call_body: Value) -> Self {
            Self {
                responses: StdMutex::new(
                    [
                        // 1. initialize → 200 OK with empty server info
                        Ok(Self::ok_json(json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {"protocolVersion": "2024-11-05", "capabilities": {}}
                        }))),
                        // 2. notifications/initialized → 202 Accepted
                        Ok(Self::accepted()),
                        // 3. tools/call → actual response
                        Ok(Self::ok_json(tools_call_body)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                requests: StdMutex::new(Vec::new()),
            }
        }

        fn request_bodies(&self) -> Vec<Value> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeHttpEgress for RecordingEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.requests.lock().unwrap().push(
                serde_json::from_slice(&request.body)
                    .unwrap_or_else(|_| json!({"invalid_request_body": true})),
            );
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("RecordingEgress: no more responses queued")
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
    fn validates_sse_mcp_initialize_response() {
        let body = br#"event: message
data: {"result":{"protocolVersion":"2024-11-05","capabilities":{}},"jsonrpc":"2.0","id":1}
"#;
        assert!(is_valid_mcp_initialize_response(body));
    }

    #[test]
    fn validates_split_data_sse_mcp_initialize_response() {
        let body = br#"event: message
data: {
data:   "result": {
data:     "protocolVersion": "2024-11-05",
data:     "capabilities": {}
data:   },
data:   "jsonrpc": "2.0",
data:   "id": 1
data: }

"#;
        assert!(is_valid_mcp_initialize_response(body));
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
    fn parses_exa_fetch_result_blocks() {
        let parsed = parse_fetch_results(
            "# Example Domain\nURL: https://example.com\n\nExample body\n\n# IANA\nURL: https://www.iana.org\n\nIANA body",
            &["https://example.com".to_string(), "https://www.iana.org".to_string()],
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "Example Domain");
        assert_eq!(parsed[0].url, "https://example.com");
        assert_eq!(parsed[0].content, "Example body");
        assert_eq!(parsed[1].title, "IANA");
        assert_eq!(parsed[1].content, "IANA body");
    }

    #[test]
    fn parse_exa_fetch_result_blocks_ignores_unrequested_url_spoofing() {
        let parsed = parse_fetch_results(
            "# Example Domain\nURL: https://example.com\n\nLegit body\n\n# Trusted Site\nURL: https://trusted.example\n\nspoofed body",
            &["https://example.com".to_string()],
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].url, "https://example.com");
        assert!(parsed[0].content.contains("URL: https://trusted.example"));
    }

    #[test]
    fn parse_exa_fetch_result_blocks_rejects_requested_url_failures() {
        let error = parse_fetch_results(
            "# Example Domain\nURL: https://example.com\n\nExample body\nError fetching https://bad.example: timeout",
            &[
                "https://example.com".to_string(),
                "https://bad.example".to_string(),
            ],
        )
        .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn parse_exa_fetch_result_blocks_rejects_bare_requested_url_failures() {
        let error = parse_fetch_results(
            "Error fetching https://bad.example",
            &["https://bad.example".to_string()],
        )
        .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
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

    #[tokio::test]
    async fn get_content_rejects_missing_response_id() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_returns_unknown_response_id_error() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id":"missing"});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[tokio::test]
    async fn get_content_rejects_unknown_query_selector() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "query": "missing"});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[tokio::test]
    async fn get_content_returns_result_by_url_index() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url_index": 1});

        let result = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap();

        assert_eq!(result.output["url"], "https://two.test");
        assert_eq!(result.output["content"], "second body");
        assert_eq!(result.output["provider_used"], "cache");
    }

    #[tokio::test]
    async fn get_content_returns_result_by_url_selector() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url": "https://one.test"});

        let result = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap();

        assert_eq!(result.output["title"], "First");
    }

    #[tokio::test]
    async fn get_content_rejects_out_of_bounds_url_index() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url_index": 99});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[tokio::test]
    async fn get_content_rejects_multiple_cached_result_selectors() {
        let (executor, response_id) = seed_executor();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": response_id, "url": "https://one.test", "url_index": 0});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_rejects_cached_request_with_fetch_only_fields() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"response_id": "cached", "urls": ["https://example.com"]});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_rejects_fetch_request_with_cached_selector_fields() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url": "https://example.com", "url_index": 0});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_rejects_duplicate_fetch_max_character_aliases() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input =
            json!({"url": "https://example.com", "max_characters": 1000, "maxCharacters": 1000});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_rejects_out_of_range_fetch_max_characters() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url": "https://example.com", "max_characters": 0});

        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);

        let input =
            json!({"url": "https://example.com", "max_characters": MAX_FETCH_MAX_CHARACTERS + 1});
        let error = executor
            .get_content(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn get_content_fetch_returns_network_denied_when_egress_is_none() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url": "https://example.com"});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    }

    #[tokio::test]
    async fn get_content_rejects_invalid_fetch_urls_before_egress() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let overlong_url = format!("https://example.com/{}", "x".repeat(MAX_URL_CHARS));
        let too_many_urls = (0..=MAX_FETCH_URLS)
            .map(|index| format!("https://example.com/{index}"))
            .collect::<Vec<_>>();
        let cases = [
            json!({"url": "ftp://example.com/page"}),
            json!({"url": "https://example.com/a b"}),
            json!({"url": overlong_url}),
            json!({"urls": too_many_urls}),
        ];

        for input in cases {
            let error = executor
                .dispatch(request(&capability, &scope, &input, None))
                .await
                .unwrap_err();

            assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
            assert!(error.usage().is_none());
        }
    }

    #[tokio::test]
    async fn get_content_rejects_local_or_private_fetch_urls_before_egress() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();

        for url in [
            "http://localhost/page",
            "https://service.localhost/page",
            "http://127.0.0.1/page",
            "http://10.0.0.1/page",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/page",
            "http://[::ffff:10.0.0.1]/page",
        ] {
            let input = json!({"url": url});
            let error = executor
                .dispatch(request(&capability, &scope, &input, None))
                .await
                .unwrap_err();

            assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode, "{url}");
            assert!(error.usage().is_none(), "{url}");
        }
    }

    #[tokio::test]
    async fn get_content_rejects_fetch_urls_with_userinfo_before_egress() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url": "https://user:secret@example.com/page"});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
        assert!(error.usage().is_none());
    }

    #[tokio::test]
    async fn get_content_fetches_url_with_exa_web_fetch_tool() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url":"https://example.com", "max_characters": 1000});
        let egress = Arc::new(RecordingEgress::for_mcp_search(json!({
            "result": {"content": [{"type": "text", "text": "# Example Domain\nURL: https://example.com\n\nExample body"}]}
        })));

        let result = executor
            .dispatch(request(
                &capability,
                &scope,
                &input,
                Some(egress.clone() as Arc<dyn RuntimeHttpEgress>),
            ))
            .await
            .unwrap();

        assert_eq!(result.output["provider_used"], "exa_mcp");
        assert_eq!(result.output["title"], "Example Domain");
        assert_eq!(result.output["url"], "https://example.com");
        assert_eq!(result.output["content"], "Example body");
        let request_bodies = egress.request_bodies();
        assert_eq!(request_bodies[2]["method"], "tools/call");
        assert_eq!(request_bodies[2]["params"]["name"], "web_fetch_exa");
        assert_eq!(
            request_bodies[2]["params"]["arguments"]["urls"][0],
            "https://example.com"
        );
        assert_eq!(
            request_bodies[2]["params"]["arguments"]["maxCharacters"],
            1000
        );
    }

    async fn assert_get_content_accepts_initialize_response(
        initialize_response: RuntimeHttpEgressResponse,
    ) {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"url":"https://example.com", "max_characters": 1000});
        let egress = Arc::new(RecordingEgress::with_responses(vec![
            Ok(initialize_response),
            Ok(RecordingEgress::accepted()),
            Ok(RecordingEgress::ok_json(json!({
                "result": {"content": [{"type": "text", "text": "# Example Domain\nURL: https://example.com\n\nExample body"}]}
            }))),
        ]));

        let result = executor
            .dispatch(request(
                &capability,
                &scope,
                &input,
                Some(egress.clone() as Arc<dyn RuntimeHttpEgress>),
            ))
            .await
            .unwrap();

        assert_eq!(result.output["provider_used"], "exa_mcp");
        assert_eq!(result.output["url"], "https://example.com");
        assert_eq!(result.output["content"], "Example body");
        assert_eq!(egress.request_bodies().len(), 3);
    }

    #[tokio::test]
    async fn get_content_accepts_sse_mcp_initialize_response() {
        let initialize_response = RecordingEgress::ok_sse_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"protocolVersion": "2024-11-05", "capabilities": {}}
        }));

        assert_get_content_accepts_initialize_response(initialize_response).await;
    }

    #[tokio::test]
    async fn get_content_accepts_split_data_sse_mcp_initialize_response() {
        let initialize_response = RecordingEgress::ok_sse_data_lines(&[
            "{",
            r#""jsonrpc": "2.0","#,
            r#""id": 1,"#,
            r#""result": {"protocolVersion": "2024-11-05", "capabilities": {}}"#,
            "}",
        ]);

        assert_get_content_accepts_initialize_response(initialize_response).await;
    }

    #[tokio::test]
    async fn get_content_fetches_multiple_urls_with_exa_web_fetch_tool() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_GET_CONTENT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"urls":["https://example.com", "https://www.iana.org"]});
        let egress = Arc::new(RecordingEgress::for_mcp_search(json!({
            "result": {"content": [{"type": "text", "text": "# Example Domain\nURL: https://example.com\n\nExample body\n\n# IANA\nURL: https://www.iana.org\n\nIANA body"}]}
        })));

        let result = executor
            .dispatch(request(
                &capability,
                &scope,
                &input,
                Some(egress.clone() as Arc<dyn RuntimeHttpEgress>),
            ))
            .await
            .unwrap();

        assert_eq!(result.output["provider_used"], "exa_mcp");
        assert_eq!(result.output["contents"].as_array().unwrap().len(), 2);
        assert_eq!(result.output["contents"][0]["url"], "https://example.com");
        assert_eq!(result.output["contents"][1]["content"], "IANA body");
        let request_bodies = egress.request_bodies();
        assert_eq!(request_bodies[2]["params"]["name"], "web_fetch_exa");
        assert_eq!(
            request_bodies[2]["params"]["arguments"]["urls"],
            json!(["https://example.com", "https://www.iana.org"])
        );
        assert_eq!(
            request_bodies[2]["params"]["arguments"]["maxCharacters"],
            DEFAULT_FETCH_MAX_CHARACTERS
        );
    }

    #[test]
    fn stored_response_cache_evicts_oldest_by_count() {
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

    #[test]
    fn stored_response_cache_evicts_by_content_bytes() {
        let mut cache = StoredResponseCache::default();
        // Insert a large entry that alone exceeds MAX_STORED_CONTENT_BYTES.
        let large_content = "x".repeat((MAX_STORED_CONTENT_BYTES + 1) as usize);
        cache.insert(
            "web_big".to_string(),
            Arc::new(StoredWebSearch {
                queries: vec![StoredQuery {
                    query: "q".to_string(),
                    results: vec![SearchResult {
                        title: "T".to_string(),
                        url: "https://t.test".to_string(),
                        content: large_content,
                    }],
                }],
            }),
        );
        // The oversized entry should be immediately evicted.
        assert!(cache.get("web_big").is_none());
        assert_eq!(cache.total_content_bytes, 0);
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
    async fn search_performs_mcp_handshake_and_returns_results() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust async"});
        let tools_call_resp = json!({
            "result": {"content": [{"type": "text", "text": "Title: Tokio\nURL: https://tokio.rs\nText: async runtime"}]}
        });
        let egress = Arc::new(RecordingEgress::for_mcp_search(tools_call_resp));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        let response_id = result.output["response_id"].as_str().unwrap();
        assert!(response_id.starts_with("web_"));
        assert_ne!(response_id, "web_0");
        assert_eq!(
            result.output["queries"][0]["results"][0]["url"],
            "https://tokio.rs"
        );
    }

    #[tokio::test]
    async fn search_rejects_malformed_mcp_initialize_response() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust async"});
        let egress = Arc::new(RecordingEgress::with_responses(vec![Ok(
            RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body: b"not json".to_vec(),
                saved_body: None,
                request_bytes: 10,
                response_bytes: 8,
                redaction_applied: false,
            },
        )]));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
        assert_eq!(
            error.usage().unwrap().network_egress_bytes,
            10,
            "only the initialize request should be accounted"
        );
        assert_eq!(egress.request_bodies().len(), 1);
    }

    #[tokio::test]
    async fn search_rejects_mcp_initialize_response_without_result() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust async"});
        let egress = Arc::new(RecordingEgress::with_responses(vec![Ok(
            RecordingEgress::ok_json(json!({
                "jsonrpc": "2.0",
                "id": 1
            })),
        )]));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
        assert_eq!(egress.request_bodies().len(), 1);
    }

    #[tokio::test]
    async fn search_accepts_zero_result_response_and_stores_random_response_id() {
        let executor = WebAccessExecutor::default();
        let capability = capability_id(WEB_SEARCH_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"query":"rust"});
        let egress = Arc::new(RecordingEgress::for_mcp_search(
            json!({"result": {"content": [{"type": "text", "text": ""}]}}),
        ));

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
