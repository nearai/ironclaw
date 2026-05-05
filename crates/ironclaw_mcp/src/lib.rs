//! MCP adapter contracts for IronClaw Reborn.
//!
//! `ironclaw_mcp` adapts manifest-declared MCP tools into IronClaw
//! capabilities. It does not grant MCP servers ambient filesystem, secret, or
//! network authority; the host-selected client is the only integration point and
//! resource accounting still happens through the host governor.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_extensions::{ExtensionPackage, ExtensionRuntime};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkMethod, NetworkPolicy, ResourceEstimate, ResourceReservation,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeCredentialInjection,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    RuntimeKind,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt};
use serde_json::Value;
use thiserror::Error;

/// Host-owned MCP adapter limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRuntimeConfig {
    pub max_output_bytes: u64,
}

impl Default for McpRuntimeConfig {
    fn default() -> Self {
        Self {
            max_output_bytes: 1024 * 1024,
        }
    }
}

impl McpRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            max_output_bytes: 64 * 1024,
        }
    }
}

/// JSON invocation passed to a manifest-declared MCP capability.
#[derive(Debug, Clone, PartialEq)]
pub struct McpInvocation {
    pub input: Value,
}

/// Full resource-governed MCP execution request.
#[derive(Debug)]
pub struct McpExecutionRequest<'a> {
    pub package: &'a ExtensionPackage,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub resource_reservation: Option<ResourceReservation>,
    pub invocation: McpInvocation,
}

/// Host-normalized request handed to the configured MCP client adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct McpClientRequest {
    pub provider: ExtensionId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub input: Value,
    pub max_output_bytes: u64,
}

/// Raw MCP adapter output before resource reconciliation.
#[derive(Debug, Clone, PartialEq)]
pub struct McpClientOutput {
    pub output: Value,
    pub usage: ResourceUsage,
    pub output_bytes: Option<u64>,
}

impl McpClientOutput {
    pub fn json(value: Value) -> Self {
        Self {
            output: value,
            usage: ResourceUsage::default(),
            output_bytes: None,
        }
    }
}

/// Host-selected MCP client adapter.
///
/// Implementations must enforce `McpClientRequest::max_output_bytes` while
/// reading MCP server output, before constructing the structured JSON `Value`.
/// The runtime re-checks serialized output size after the adapter returns, but
/// that check is a second line of defense rather than the primary memory bound.
#[async_trait]
pub trait McpClient: Send + Sync {
    /// HTTP/SSE MCP transports must be implemented through the shared host-mediated
    /// runtime egress boundary. The default is fail-closed so a generic client
    /// cannot accidentally perform direct outbound HTTP.
    fn uses_host_mediated_http_egress(&self) -> bool {
        false
    }

    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String>;
}

/// Parsed MCP capability result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCapabilityResult {
    pub output: Value,
    pub reservation_id: ResourceReservationId,
    pub usage: ResourceUsage,
    pub output_bytes: u64,
}

/// Full resource-governed MCP execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpExecutionResult {
    pub result: McpCapabilityResult,
    pub receipt: ResourceReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHostHttpRequest {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: ironclaw_host_api::NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub network_policy: ironclaw_host_api::NetworkPolicy,
    pub credential_injections: Vec<ironclaw_host_api::RuntimeCredentialInjection>,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

pub type McpHostHttpResponse = RuntimeHttpEgressResponse;

#[derive(Debug, Error)]
pub enum McpHostHttpError {
    #[error("MCP host HTTP error: {reason}")]
    Egress { reason: String },
}

#[derive(Debug, Clone)]
pub struct McpRuntimeHttpAdapter<E> {
    egress: E,
}

impl<E> McpRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress,
{
    pub fn new(egress: E) -> Self {
        Self { egress }
    }

    pub fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        self.egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::Mcp,
                scope: request.scope,
                capability_id: request.capability_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                network_policy: request.network_policy,
                credential_injections: request.credential_injections,
                response_body_limit: request.response_body_limit,
                timeout_ms: request.timeout_ms,
            })
            .map_err(mcp_http_error)
    }
}

fn mcp_http_error(error: RuntimeHttpEgressError) -> McpHostHttpError {
    McpHostHttpError::Egress {
        reason: error.stable_runtime_reason().to_string(),
    }
}

pub trait McpHostHttp: Send + Sync {
    fn request(&self, request: McpHostHttpRequest)
    -> Result<McpHostHttpResponse, McpHostHttpError>;
}

impl<E> McpHostHttp for McpRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress,
{
    fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        McpRuntimeHttpAdapter::request(self, request)
    }
}

impl<T> McpHostHttp for Arc<T>
where
    T: McpHostHttp + ?Sized,
{
    fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        self.as_ref().request(request)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpHostHttpEgressPlan {
    pub network_policy: NetworkPolicy,
    pub credential_injections: Vec<RuntimeCredentialInjection>,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct McpHostHttpEgressPlanRequest<'a> {
    pub provider: &'a ExtensionId,
    pub capability_id: &'a CapabilityId,
    pub scope: &'a ResourceScope,
    pub transport: &'a str,
    pub method: NetworkMethod,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
}

/// Host-owned egress planner for MCP HTTP/SSE requests.
///
/// The planner is intentionally separate from [`McpClientRequest::input`]:
/// runtime/plugin inputs can affect the JSON-RPC body, but only this host-owned
/// planner can provide network policy, credential handles, response limits, and
/// timeouts for the shared egress service.
pub trait McpHostHttpEgressPlanner: Send + Sync {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan;
}

impl<T> McpHostHttpEgressPlanner for Arc<T>
where
    T: McpHostHttpEgressPlanner + ?Sized,
{
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        self.as_ref().plan(request)
    }
}

#[derive(Debug, Clone)]
pub struct StaticMcpHostHttpEgressPlanner {
    plan: McpHostHttpEgressPlan,
}

impl StaticMcpHostHttpEgressPlanner {
    pub fn new(plan: McpHostHttpEgressPlan) -> Self {
        Self { plan }
    }
}

impl McpHostHttpEgressPlanner for StaticMcpHostHttpEgressPlanner {
    fn plan(&self, _request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        self.plan.clone()
    }
}

#[derive(Debug, Clone)]
pub struct McpHostHttpClient<H, P> {
    http: H,
    planner: P,
    state: Arc<McpHostHttpClientState>,
}

#[derive(Debug)]
struct McpHostHttpClientState {
    next_id: AtomicU64,
    session_ids: Mutex<HashMap<McpHostHttpSessionKey, String>>,
}

struct McpHostHttpSessionCleanup {
    state: Arc<McpHostHttpClientState>,
    session_key: McpHostHttpSessionKey,
}

impl McpHostHttpSessionCleanup {
    fn new(state: Arc<McpHostHttpClientState>, session_key: McpHostHttpSessionKey) -> Self {
        Self { state, session_key }
    }
}

impl Drop for McpHostHttpSessionCleanup {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.session_ids.lock() {
            guard.remove(&self.session_key);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct McpHostHttpSessionKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    provider: String,
    url: String,
}

impl McpHostHttpSessionKey {
    fn new(scope: &ResourceScope, provider: &ExtensionId, url: &str) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            provider: provider.as_str().to_string(),
            url: url.to_string(),
        }
    }
}

impl<H, P> McpHostHttpClient<H, P>
where
    H: McpHostHttp,
    P: McpHostHttpEgressPlanner,
{
    pub fn new(http: H, planner: P) -> Self {
        Self {
            http,
            planner,
            state: Arc::new(McpHostHttpClientState {
                next_id: AtomicU64::new(1),
                session_ids: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn next_request_id(&self) -> u64 {
        self.state.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn send_json_rpc(
        &self,
        request: &McpClientRequest,
        session_key: &McpHostHttpSessionKey,
        id: Option<u64>,
        method: &str,
        params: Option<Value>,
    ) -> Result<McpJsonRpcExchange, String> {
        let url = request.url.as_deref().ok_or_else(request_denied)?;
        let body = encode_json_rpc_request(id, method, params)?;
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "Accept".to_string(),
                "application/json, text/event-stream".to_string(),
            ),
        ];
        if let Some(session_id) = self.current_session_id(session_key)? {
            headers.push(("Mcp-Session-Id".to_string(), session_id));
        }

        let plan = self.planner.plan(McpHostHttpEgressPlanRequest {
            provider: &request.provider,
            capability_id: &request.capability_id,
            scope: &request.scope,
            transport: &request.transport,
            method: NetworkMethod::Post,
            url,
            headers: &headers,
            body: &body,
        });

        let response_body_limit =
            effective_mcp_response_body_limit(plan.response_body_limit, request.max_output_bytes);
        let response = self
            .http
            .request(McpHostHttpRequest {
                scope: request.scope.clone(),
                capability_id: request.capability_id.clone(),
                method: NetworkMethod::Post,
                url: url.to_string(),
                headers,
                body,
                network_policy: plan.network_policy,
                credential_injections: plan.credential_injections,
                response_body_limit,
                timeout_ms: plan.timeout_ms,
            })
            .map_err(mcp_client_http_error)?;

        let usage = ResourceUsage {
            network_egress_bytes: response.request_bytes,
            ..ResourceUsage::default()
        };

        if !(200..300).contains(&response.status) {
            return Err(response_error());
        }
        self.capture_session_id(session_key, &response)?;

        if response.status == 202 && id.is_none() {
            return Ok(McpJsonRpcExchange {
                response: McpJsonRpcResponse {
                    result: None,
                    error: false,
                },
                usage,
            });
        }

        Ok(McpJsonRpcExchange {
            response: parse_mcp_response(&response, id)?,
            usage,
        })
    }

    fn current_session_id(
        &self,
        session_key: &McpHostHttpSessionKey,
    ) -> Result<Option<String>, String> {
        self.state
            .session_ids
            .lock()
            .map(|guard| guard.get(session_key).cloned())
            .map_err(|_| request_denied())
    }

    fn capture_session_id(
        &self,
        session_key: &McpHostHttpSessionKey,
        response: &McpHostHttpResponse,
    ) -> Result<(), String> {
        let Some((_, value)) = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Mcp-Session-Id"))
        else {
            return Ok(());
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !is_safe_mcp_session_id(trimmed) {
            return Err(response_error());
        }
        let mut guard = self
            .state
            .session_ids
            .lock()
            .map_err(|_| request_denied())?;
        guard.insert(session_key.clone(), trimmed.to_string());
        Ok(())
    }
}

#[async_trait]
impl<H, P> McpClient for McpHostHttpClient<H, P>
where
    H: McpHostHttp,
    P: McpHostHttpEgressPlanner,
{
    fn uses_host_mediated_http_egress(&self) -> bool {
        true
    }

    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String> {
        if !requires_host_http_egress(&request.transport) {
            return Err(request_denied());
        }

        let url = request.url.as_deref().ok_or_else(request_denied)?;
        let session_key = McpHostHttpSessionKey::new(&request.scope, &request.provider, url);
        let _session_cleanup =
            McpHostHttpSessionCleanup::new(Arc::clone(&self.state), session_key.clone());

        let mut usage = ResourceUsage::default();
        let initialize = self.send_json_rpc(
            &request,
            &session_key,
            Some(self.next_request_id()),
            "initialize",
            Some(json_rpc_initialize_params()),
        )?;
        accumulate_usage(&mut usage, initialize.usage);
        if initialize.response.error {
            return Err(response_error());
        }

        let initialized = self.send_json_rpc(
            &request,
            &session_key,
            None,
            "notifications/initialized",
            None,
        )?;
        accumulate_usage(&mut usage, initialized.usage);
        if initialized.response.error {
            return Err(response_error());
        }

        let tool_name = mcp_tool_name(&request.provider, &request.capability_id);
        let call = self.send_json_rpc(
            &request,
            &session_key,
            Some(self.next_request_id()),
            "tools/call",
            Some(serde_json::json!({
                "name": tool_name,
                "arguments": request.input,
            })),
        )?;
        accumulate_usage(&mut usage, call.usage);
        if call.response.error {
            return Err(response_error());
        }
        let output = call.response.result.ok_or_else(response_error)?;
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| response_error())?;
        usage.output_bytes = usage.output_bytes.max(output_bytes);

        Ok(McpClientOutput {
            output,
            usage,
            output_bytes: Some(output_bytes),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct McpJsonRpcResponse {
    result: Option<Value>,
    error: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct McpJsonRpcExchange {
    response: McpJsonRpcResponse,
    usage: ResourceUsage,
}

fn mcp_client_http_error(error: McpHostHttpError) -> String {
    match error {
        McpHostHttpError::Egress { reason } => reason,
    }
}

fn effective_mcp_response_body_limit(host_limit: Option<u64>, client_limit: u64) -> Option<u64> {
    Some(match host_limit {
        Some(limit) => limit.min(client_limit),
        None => client_limit,
    })
}

fn is_safe_mcp_session_id(value: &str) -> bool {
    const MAX_MCP_SESSION_ID_BYTES: usize = 1024;
    !value.is_empty()
        && value.len() <= MAX_MCP_SESSION_ID_BYTES
        && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
}

fn encode_json_rpc_request(
    id: Option<u64>,
    method: &str,
    params: Option<Value>,
) -> Result<Vec<u8>, String> {
    let mut object = serde_json::Map::new();
    object.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    if let Some(id) = id {
        object.insert(
            "id".to_string(),
            Value::Number(serde_json::Number::from(id)),
        );
    }
    object.insert("method".to_string(), Value::String(method.to_string()));
    if let Some(params) = params {
        object.insert("params".to_string(), params);
    }
    serde_json::to_vec(&Value::Object(object)).map_err(|_| request_denied())
}

fn parse_mcp_response(
    response: &McpHostHttpResponse,
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    if response_is_sse(response) {
        parse_mcp_sse_response(&response.body, expected_id)
    } else {
        let value =
            serde_json::from_slice::<Value>(&response.body).map_err(|_| response_error())?;
        parse_mcp_json_rpc_value(&value, expected_id)
    }
}

fn response_is_sse(response: &McpHostHttpResponse) -> bool {
    response.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value.to_ascii_lowercase().contains("text/event-stream")
    })
}

fn parse_mcp_sse_response(
    body: &[u8],
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    let text = std::str::from_utf8(body).map_err(|_| response_error())?;
    for line in text.lines() {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let value = serde_json::from_str::<Value>(payload.trim()).map_err(|_| response_error())?;
        let parsed_id = json_rpc_id(&value);
        if expected_id.is_none() || parsed_id == expected_id {
            return parse_mcp_json_rpc_value(&value, expected_id);
        }
    }
    Err(response_error())
}

fn parse_mcp_json_rpc_value(
    value: &Value,
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    let parsed_id = json_rpc_id(value);
    if let Some(expected) = expected_id
        && parsed_id != Some(expected)
    {
        return Err(response_error());
    }
    Ok(McpJsonRpcResponse {
        result: value.get("result").cloned(),
        error: value.get("error").is_some(),
    })
}

fn json_rpc_id(value: &Value) -> Option<u64> {
    match value.get("id") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(value)) => value.parse::<u64>().ok(),
        _ => None,
    }
}

fn json_rpc_initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "roots": { "listChanged": false },
            "sampling": {}
        },
        "clientInfo": {
            "name": "ironclaw",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn mcp_tool_name(provider: &ExtensionId, capability_id: &CapabilityId) -> String {
    let prefix = format!("{}.", provider.as_str());
    capability_id
        .as_str()
        .strip_prefix(&prefix)
        .unwrap_or_else(|| capability_id.as_str())
        .to_string()
}

fn accumulate_usage(total: &mut ResourceUsage, usage: ResourceUsage) {
    total.network_egress_bytes = total
        .network_egress_bytes
        .saturating_add(usage.network_egress_bytes);
    total.output_bytes = total.output_bytes.saturating_add(usage.output_bytes);
}

fn request_denied() -> String {
    "request_denied".to_string()
}

fn response_error() -> String {
    "response_error".to_string()
}

/// MCP runtime failures.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("resource governor error: {0}")]
    Resource(Box<ResourceError>),
    #[error("MCP client error: {reason}")]
    Client { reason: String },
    #[error("unsupported MCP transport {transport}")]
    UnsupportedTransport { transport: String },
    #[error("MCP transport {transport} requires host-mediated HTTP egress")]
    HostHttpEgressRequired { transport: String },
    #[error("stdio MCP transport is unsupported until process-level egress controls land")]
    ExternalStdioTransportUnsupported,
    #[error("extension {extension} uses runtime {actual:?}, not RuntimeKind::Mcp")]
    ExtensionRuntimeMismatch {
        extension: ExtensionId,
        actual: RuntimeKind,
    },
    #[error("capability {capability} is not declared by this extension package")]
    CapabilityNotDeclared { capability: CapabilityId },
    #[error("MCP descriptor mismatch: {reason}")]
    DescriptorMismatch { reason: String },
    #[error("invalid MCP invocation: {reason}")]
    InvalidInvocation { reason: String },
    #[error("MCP output limit exceeded: limit {limit}, actual {actual}")]
    OutputLimitExceeded { limit: u64, actual: u64 },
}

impl From<ResourceError> for McpError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(Box::new(error))
    }
}

/// Runtime for executing manifest-declared MCP capabilities through a host adapter.
#[derive(Debug, Clone)]
pub struct McpRuntime<C> {
    config: McpRuntimeConfig,
    client: C,
}

impl<C> McpRuntime<C>
where
    C: McpClient,
{
    pub fn new(config: McpRuntimeConfig, client: C) -> Self {
        Self { config, client }
    }

    pub fn config(&self) -> &McpRuntimeConfig {
        &self.config
    }

    pub async fn execute_extension_json<G>(
        &self,
        governor: &G,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError>
    where
        G: ResourceGovernor + ?Sized,
    {
        let client_request = self.prepare_client_request(&request)?;
        let transport = client_request.transport.clone();
        if requires_host_http_egress(&transport) && !self.client.uses_host_mediated_http_egress() {
            return Err(McpError::HostHttpEgressRequired { transport });
        }
        let reservation = reserve_or_use_existing(
            governor,
            request.scope.clone(),
            request.estimate.clone(),
            request.resource_reservation.clone(),
        )?;

        let output = match self.client.call_tool(client_request).await {
            Ok(output) => output,
            Err(reason) => {
                return Err(release_after_failure(
                    governor,
                    reservation.id,
                    McpError::Client { reason },
                ));
            }
        };

        let serialized_len = serde_json::to_vec(&output.output)
            .map_err(|error| {
                release_after_failure(
                    governor,
                    reservation.id,
                    McpError::InvalidInvocation {
                        reason: error.to_string(),
                    },
                )
            })?
            .len() as u64;
        let output_bytes = output
            .output_bytes
            .unwrap_or(serialized_len)
            .max(serialized_len);
        if output_bytes > self.config.max_output_bytes {
            return Err(release_after_failure(
                governor,
                reservation.id,
                McpError::OutputLimitExceeded {
                    limit: self.config.max_output_bytes,
                    actual: output_bytes,
                },
            ));
        }

        let mut usage = output.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        if transport == "stdio" {
            usage.process_count = usage.process_count.max(1);
        }
        let receipt = governor.reconcile(reservation.id, usage.clone())?;
        Ok(McpExecutionResult {
            result: McpCapabilityResult {
                output: output.output,
                reservation_id: reservation.id,
                usage,
                output_bytes,
            },
            receipt,
        })
    }

    fn prepare_client_request(
        &self,
        request: &McpExecutionRequest<'_>,
    ) -> Result<McpClientRequest, McpError> {
        let descriptor = request
            .package
            .capabilities
            .iter()
            .find(|descriptor| &descriptor.id == request.capability_id)
            .cloned()
            .ok_or_else(|| McpError::CapabilityNotDeclared {
                capability: request.capability_id.clone(),
            })?;

        if descriptor.runtime != RuntimeKind::Mcp {
            return Err(McpError::ExtensionRuntimeMismatch {
                extension: request.package.id.clone(),
                actual: descriptor.runtime,
            });
        }
        if descriptor.provider != request.package.id {
            return Err(McpError::DescriptorMismatch {
                reason: format!(
                    "descriptor {} provider {} does not match package {}",
                    descriptor.id, descriptor.provider, request.package.id
                ),
            });
        }

        let (transport, command, args, url) = match &request.package.manifest.runtime {
            ExtensionRuntime::Mcp {
                transport,
                command,
                args,
                url,
            } => (transport, command, args, url),
            other => {
                return Err(McpError::ExtensionRuntimeMismatch {
                    extension: request.package.id.clone(),
                    actual: other.kind(),
                });
            }
        };

        if transport == "stdio" {
            return Err(McpError::ExternalStdioTransportUnsupported);
        }
        if !matches!(transport.as_str(), "http" | "sse") {
            return Err(McpError::UnsupportedTransport {
                transport: transport.clone(),
            });
        }
        if matches!(transport.as_str(), "http" | "sse") && url.is_none() {
            return Err(McpError::InvalidInvocation {
                reason: format!("{transport} MCP transport requires a manifest url"),
            });
        }

        Ok(McpClientRequest {
            provider: request.package.id.clone(),
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            transport: transport.clone(),
            command: command.clone(),
            args: args.clone(),
            url: url.clone(),
            input: request.invocation.input.clone(),
            max_output_bytes: self.config.max_output_bytes,
        })
    }
}

/// Object-safe MCP executor interface used by the kernel composition layer.
#[async_trait]
pub trait McpExecutor: Send + Sync {
    async fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError>;
}

#[async_trait]
impl<C> McpExecutor for McpRuntime<C>
where
    C: McpClient,
{
    async fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        McpRuntime::execute_extension_json(self, governor, request).await
    }
}

fn requires_host_http_egress(transport: &str) -> bool {
    matches!(transport, "http" | "sse")
}

fn reserve_or_use_existing<G>(
    governor: &G,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation: Option<ResourceReservation>,
) -> Result<ResourceReservation, McpError>
where
    G: ResourceGovernor + ?Sized,
{
    if let Some(reservation) = reservation {
        if reservation.scope != scope || reservation.estimate != estimate {
            return Err(McpError::Resource(Box::new(
                ResourceError::ReservationMismatch { id: reservation.id },
            )));
        }
        return Ok(reservation);
    }
    governor.reserve(scope, estimate).map_err(McpError::from)
}

fn release_after_failure<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    original: McpError,
) -> McpError
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
    original
}
