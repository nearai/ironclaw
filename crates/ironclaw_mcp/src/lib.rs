//! MCP adapter contracts for IronClaw Reborn.
//!
//! `ironclaw_mcp` adapts manifest-declared MCP tools into IronClaw
//! capabilities. It does not grant MCP servers ambient filesystem, secret, or
//! network authority; the host-selected client is the only integration point and
//! resource accounting still happens through the host governor.

use std::{
    collections::HashMap,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_extensions::{ExtensionPackage, ExtensionRuntime};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkMethod, NetworkPolicy, ResourceEstimate, ResourceReservation,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeCredentialAuthRequirement,
    RuntimeCredentialInjection, RuntimeCredentialRequirement, RuntimeCredentialRequirementSource,
    RuntimeCredentialSource, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, RuntimeKind, SecretHandle,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt};
use serde_json::Value;
use thiserror::Error;

const STREAMABLE_HTTP_MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpAuthContext {
    required_secrets: Vec<SecretHandle>,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
}

#[derive(Debug)]
struct PreparedMcpClientRequest {
    request: McpClientRequest,
    auth_context: McpAuthContext,
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

/// MCP tool descriptor discovered from a hosted server's `tools/list` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDiscoveredTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: McpDiscoveredToolAnnotations,
}

/// MCP tool behavior hints returned by `tools/list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpDiscoveredToolAnnotations {
    pub destructive_hint: bool,
    pub side_effects_hint: bool,
    pub read_only_hint: bool,
}

/// Result of a hosted MCP schema-discovery pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolDiscoveryOutput {
    pub tools: Vec<McpDiscoveredTool>,
    pub usage: ResourceUsage,
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

    async fn call_tool(&self, request: McpClientRequest)
    -> Result<McpClientOutput, McpClientError>;

    async fn discover_tools(
        &self,
        request: McpClientRequest,
    ) -> Result<McpToolDiscoveryOutput, McpClientError> {
        let _ = request;
        Err(McpClientError::client(request_denied()))
    }
}

/// Stable, sanitized MCP client-side failure categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpClientError {
    Client { reason: String },
    AuthRequired,
}

impl McpClientError {
    pub fn client(reason: impl Into<String>) -> Self {
        Self::Client {
            reason: reason.into(),
        }
    }

    pub fn stable_reason(&self) -> &str {
        match self {
            Self::Client { reason } => reason,
            Self::AuthRequired => "auth_required",
        }
    }
}

impl From<String> for McpClientError {
    fn from(reason: String) -> Self {
        Self::client(reason)
    }
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

    pub async fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        AssertUnwindSafe(self.egress.execute(RuntimeHttpEgressRequest {
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
            save_body_to: None,
            timeout_ms: request.timeout_ms,
        }))
        .catch_unwind()
        .await
        .map_err(|_| McpHostHttpError::Egress {
            reason: "runtime_http_egress_panicked".to_string(),
        })?
        .map_err(mcp_http_error)
    }
}

fn mcp_http_error(error: RuntimeHttpEgressError) -> McpHostHttpError {
    McpHostHttpError::Egress {
        reason: error.stable_runtime_reason().to_string(),
    }
}

#[async_trait]
pub trait McpHostHttp: Send + Sync {
    async fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError>;
}

#[async_trait]
impl<E> McpHostHttp for McpRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress + Send + Sync,
{
    async fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        McpRuntimeHttpAdapter::request(self, request).await
    }
}

#[async_trait]
impl<T> McpHostHttp for Arc<T>
where
    T: McpHostHttp + ?Sized + Send + Sync,
{
    async fn request(
        &self,
        request: McpHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        self.as_ref().request(request).await
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
///
/// `plan` must be deterministic and side-effect-free. The concrete HTTP client
/// plans the real `tools/call` body once before the MCP handshake, validates
/// its credential sources, then threads that plan into the later `tools/call`
/// transport send. Planner-visible headers are stable policy headers only; the
/// dynamic MCP session header is added by the protocol client after planning.
/// Hosted MCP providers may require authentication for the entire JSON-RPC
/// session, including initialization, so staged credentials must remain scoped
/// to the invocation until the capability dispatch completes.
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
    // `std::sync::Mutex` is appropriate here: the lock is held only for O(1)
    // HashMap operations (never across an `.await`), and the key includes
    // `invocation_id` so concurrent dispatches from different invocations act
    // on disjoint map entries with no real contention.
    sessions: Mutex<HashMap<McpHostHttpSessionKey, McpHostHttpSession>>,
}

struct McpHostHttpSessionCleanup {
    state: Arc<McpHostHttpClientState>,
    session_key: McpHostHttpSessionKey,
}

struct PlannedMcpJsonRpc {
    id: Option<u64>,
    method: McpJsonRpcMethod,
    url: String,
    policy_headers: Vec<(String, String)>,
    body: Vec<u8>,
    plan: McpHostHttpEgressPlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpHostHttpSession {
    session_id: Option<String>,
    protocol_version: String,
}

impl McpHostHttpSessionCleanup {
    fn new(state: Arc<McpHostHttpClientState>, session_key: McpHostHttpSessionKey) -> Self {
        Self { state, session_key }
    }
}

impl Drop for McpHostHttpSessionCleanup {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.sessions.lock() {
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
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn next_request_id(&self) -> u64 {
        self.state.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn send_json_rpc(
        &self,
        request: &McpClientRequest,
        session_key: &McpHostHttpSessionKey,
        id: Option<u64>,
        method: McpJsonRpcMethod,
        params: Option<Value>,
    ) -> Result<McpJsonRpcExchange, McpClientError> {
        let planned = self.plan_json_rpc(request, id, method, params)?;
        self.send_planned_json_rpc(request, session_key, planned)
            .await
    }

    fn plan_json_rpc(
        &self,
        request: &McpClientRequest,
        id: Option<u64>,
        method: McpJsonRpcMethod,
        params: Option<Value>,
    ) -> Result<PlannedMcpJsonRpc, McpClientError> {
        let url = request
            .url
            .as_deref()
            .ok_or_else(|| McpClientError::client(request_denied()))?;
        let body =
            encode_json_rpc_request(id, method.as_str(), params).map_err(McpClientError::client)?;
        let policy_headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "Accept".to_string(),
                "application/json, text/event-stream".to_string(),
            ),
        ];

        let plan = self.planner.plan(McpHostHttpEgressPlanRequest {
            provider: &request.provider,
            capability_id: &request.capability_id,
            scope: &request.scope,
            transport: &request.transport,
            method: NetworkMethod::Post,
            url,
            headers: &policy_headers,
            body: &body,
        });
        Ok(PlannedMcpJsonRpc {
            id,
            method,
            url: url.to_string(),
            policy_headers,
            body,
            plan,
        })
    }

    async fn send_planned_json_rpc(
        &self,
        request: &McpClientRequest,
        session_key: &McpHostHttpSessionKey,
        planned: PlannedMcpJsonRpc,
    ) -> Result<McpJsonRpcExchange, McpClientError> {
        let mut headers = planned.policy_headers;
        if let Some(session) = self.current_session(session_key)? {
            headers.push((
                MCP_PROTOCOL_VERSION_HEADER.to_string(),
                session.protocol_version,
            ));
            if let Some(session_id) = session.session_id {
                headers.push(("Mcp-Session-Id".to_string(), session_id));
            }
        }

        let response_body_limit = effective_mcp_response_body_limit(
            planned.plan.response_body_limit,
            request.max_output_bytes,
        );
        let credential_injections = planned
            .method
            .credential_injections(planned.plan.credential_injections)?;
        let response = self
            .http
            .request(McpHostHttpRequest {
                scope: request.scope.clone(),
                capability_id: request.capability_id.clone(),
                method: NetworkMethod::Post,
                url: planned.url,
                headers,
                body: planned.body,
                network_policy: planned.plan.network_policy,
                credential_injections,
                response_body_limit,
                timeout_ms: planned.plan.timeout_ms,
            })
            .await
            .map_err(mcp_client_http_error)?;

        let usage = ResourceUsage {
            network_egress_bytes: response.request_bytes,
            ..ResourceUsage::default()
        };

        if !(200..300).contains(&response.status) {
            if is_mcp_auth_response_status(response.status) {
                return Err(McpClientError::AuthRequired);
            }
            return Err(McpClientError::client(response_error()));
        }
        let session_id = mcp_session_id_from_response(&response).map_err(McpClientError::client)?;

        if response.status == 202 && planned.id.is_none() {
            return Ok(McpJsonRpcExchange {
                response: McpJsonRpcResponse {
                    result: None,
                    error: false,
                },
                session_id,
                usage,
            });
        }

        Ok(McpJsonRpcExchange {
            response: parse_mcp_response(&response, planned.id).map_err(McpClientError::client)?,
            session_id,
            usage,
        })
    }

    fn current_session(
        &self,
        session_key: &McpHostHttpSessionKey,
    ) -> Result<Option<McpHostHttpSession>, McpClientError> {
        self.state
            .sessions
            .lock()
            .map(|guard| guard.get(session_key).cloned())
            .map_err(|_| McpClientError::client(request_denied()))
    }

    fn store_session(
        &self,
        session_key: &McpHostHttpSessionKey,
        session: McpHostHttpSession,
    ) -> Result<(), McpClientError> {
        let mut guard = self
            .state
            .sessions
            .lock()
            .map_err(|_| McpClientError::client(request_denied()))?;
        guard.insert(session_key.clone(), session);
        Ok(())
    }

    fn update_session_id(
        &self,
        session_key: &McpHostHttpSessionKey,
        session_id: Option<String>,
    ) -> Result<(), McpClientError> {
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let mut guard = self
            .state
            .sessions
            .lock()
            .map_err(|_| McpClientError::client(request_denied()))?;
        if let Some(session) = guard.get_mut(session_key) {
            session.session_id = Some(session_id);
        }
        Ok(())
    }

    async fn initialize_session(
        &self,
        request: &McpClientRequest,
        session_key: &McpHostHttpSessionKey,
    ) -> Result<ResourceUsage, McpClientError> {
        let mut usage = ResourceUsage::default();
        let initialize_id = self.next_request_id();
        let initialize = self
            .send_json_rpc(
                request,
                session_key,
                Some(initialize_id),
                McpJsonRpcMethod::Initialize,
                Some(json_rpc_initialize_params()),
            )
            .await?;
        accumulate_usage(&mut usage, initialize.usage);
        if initialize.response.error {
            return Err(McpClientError::client(response_error()));
        }
        self.store_session(
            session_key,
            McpHostHttpSession {
                session_id: initialize.session_id,
                protocol_version: protocol_version_from_initialize_response(&initialize.response)
                    .map_err(McpClientError::client)?,
            },
        )?;

        let initialized = self
            .send_json_rpc(
                request,
                session_key,
                None,
                McpJsonRpcMethod::InitializedNotification,
                None,
            )
            .await?;
        accumulate_usage(&mut usage, initialized.usage);
        self.update_session_id(session_key, initialized.session_id.clone())?;
        if initialized.response.error {
            return Err(McpClientError::client(response_error()));
        }
        Ok(usage)
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

    async fn call_tool(
        &self,
        request: McpClientRequest,
    ) -> Result<McpClientOutput, McpClientError> {
        if !requires_host_http_egress(&request.transport) {
            return Err(McpClientError::client(request_denied()));
        }

        let url = request
            .url
            .as_deref()
            .ok_or_else(|| McpClientError::client(request_denied()))?;
        let session_key = McpHostHttpSessionKey::new(&request.scope, &request.provider, url);
        let _session_cleanup =
            McpHostHttpSessionCleanup::new(Arc::clone(&self.state), session_key.clone());

        let tool_name = mcp_tool_name(&request.provider, &request.capability_id);
        let tool_call_params = serde_json::json!({
            "name": tool_name,
            "arguments": request.input.clone(),
        });
        let tool_call_id = self.next_request_id();
        let tool_call_plan = self.plan_json_rpc(
            &request,
            Some(tool_call_id),
            McpJsonRpcMethod::ToolsCall,
            Some(tool_call_params),
        )?;
        validate_tools_call_credential_injections(&tool_call_plan.plan.credential_injections)
            .map_err(McpClientError::client)?;

        let mut usage = self.initialize_session(&request, &session_key).await?;

        let call = self
            .send_planned_json_rpc(&request, &session_key, tool_call_plan)
            .await?;
        accumulate_usage(&mut usage, call.usage);
        self.update_session_id(&session_key, call.session_id.clone())?;
        if call.response.error {
            return Err(McpClientError::client(response_error()));
        }
        let output = call
            .response
            .result
            .ok_or_else(|| McpClientError::client(response_error()))?;
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| McpClientError::client(response_error()))?;
        usage.output_bytes = usage.output_bytes.max(output_bytes);

        Ok(McpClientOutput {
            output,
            usage,
            output_bytes: Some(output_bytes),
        })
    }

    async fn discover_tools(
        &self,
        request: McpClientRequest,
    ) -> Result<McpToolDiscoveryOutput, McpClientError> {
        if !requires_host_http_egress(&request.transport) {
            return Err(McpClientError::client(request_denied()));
        }

        let url = request
            .url
            .as_deref()
            .ok_or_else(|| McpClientError::client(request_denied()))?;
        let session_key = McpHostHttpSessionKey::new(&request.scope, &request.provider, url);
        let _session_cleanup =
            McpHostHttpSessionCleanup::new(Arc::clone(&self.state), session_key.clone());

        let tools_list_id = self.next_request_id();
        let tools_list_plan = self.plan_json_rpc(
            &request,
            Some(tools_list_id),
            McpJsonRpcMethod::ToolsList,
            None,
        )?;
        validate_staged_credential_injections(&tools_list_plan.plan.credential_injections)
            .map_err(McpClientError::client)?;

        let mut usage = self.initialize_session(&request, &session_key).await?;
        let tools = self
            .send_planned_json_rpc(&request, &session_key, tools_list_plan)
            .await?;
        accumulate_usage(&mut usage, tools.usage);
        self.update_session_id(&session_key, tools.session_id.clone())?;
        if tools.response.error {
            return Err(McpClientError::client(response_error()));
        }
        let result = tools
            .response
            .result
            .ok_or_else(|| McpClientError::client(response_error()))?;
        Ok(McpToolDiscoveryOutput {
            tools: parse_tools_list_result(&result).map_err(McpClientError::client)?,
            usage,
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
    session_id: Option<String>,
    usage: ResourceUsage,
}

/// Known MCP JSON-RPC methods whose credential-routing behavior is host-owned.
///
/// Hosted MCP providers may require bearer authentication for the whole
/// JSON-RPC session, including `initialize` and notifications. The host egress
/// planner remains the source of truth for which staged credentials may be
/// sent to the provider URL, and direct secret-store leases are rejected before
/// outbound transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpJsonRpcMethod {
    Initialize,
    InitializedNotification,
    ToolsList,
    ToolsCall,
}

impl McpJsonRpcMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::InitializedNotification => "notifications/initialized",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
        }
    }

    fn credential_injections(
        self,
        credential_injections: Vec<RuntimeCredentialInjection>,
    ) -> Result<Vec<RuntimeCredentialInjection>, String> {
        if credential_injections
            .iter()
            .any(|injection| matches!(injection.source, RuntimeCredentialSource::SecretStoreLease))
        {
            return Err(request_denied());
        }
        Ok(credential_injections)
    }
}

/// Validate credential injections planned for a `tools/call` request without
/// consuming the list, so the caller can reuse it in the actual send.
///
/// Returns `Err(denied)` if any injection uses a [`RuntimeCredentialSource::SecretStoreLease`],
/// which is not permitted over the MCP `tools/call` boundary.
fn validate_tools_call_credential_injections(
    credential_injections: &[RuntimeCredentialInjection],
) -> Result<(), String> {
    validate_staged_credential_injections(credential_injections)
}

fn validate_staged_credential_injections(
    credential_injections: &[RuntimeCredentialInjection],
) -> Result<(), String> {
    if credential_injections
        .iter()
        .any(|injection| matches!(injection.source, RuntimeCredentialSource::SecretStoreLease))
    {
        return Err(request_denied());
    }
    Ok(())
}

fn mcp_client_http_error(error: McpHostHttpError) -> McpClientError {
    match error {
        McpHostHttpError::Egress { reason } => McpClientError::client(reason),
    }
}

fn is_mcp_auth_response_status(status: u16) -> bool {
    matches!(status, 401 | 403)
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

fn mcp_session_id_from_response(response: &McpHostHttpResponse) -> Result<Option<String>, String> {
    let Some((_, value)) = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Mcp-Session-Id"))
    else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !is_safe_mcp_session_id(trimmed) {
        return Err(response_error());
    }
    Ok(Some(trimmed.to_string()))
}

fn is_safe_mcp_protocol_version(value: &str) -> bool {
    const MAX_MCP_PROTOCOL_VERSION_BYTES: usize = 64;
    !value.is_empty()
        && value.len() <= MAX_MCP_PROTOCOL_VERSION_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn protocol_version_from_initialize_response(
    response: &McpJsonRpcResponse,
) -> Result<String, String> {
    let Some(protocol_version) = response
        .result
        .as_ref()
        .and_then(|result| result.get("protocolVersion"))
        .and_then(Value::as_str)
    else {
        return Err(response_error());
    };
    if !is_safe_mcp_protocol_version(protocol_version) {
        return Err(response_error());
    }
    Ok(protocol_version.to_string())
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
        let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
            continue;
        };
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

fn parse_tools_list_result(value: &Value) -> Result<Vec<McpDiscoveredTool>, String> {
    const MAX_DISCOVERED_TOOLS: usize = 128;
    const MAX_TOOL_NAME_BYTES: usize = 128;
    const MAX_TOOL_DESCRIPTION_BYTES: usize = 2048;
    const MAX_SCHEMA_DEPTH: u8 = 8;
    const MAX_SCHEMA_NODES: usize = 512;
    const MAX_SCHEMA_STRING_BYTES: usize = 1024;

    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(response_error)?;
    if tools.len() > MAX_DISCOVERED_TOOLS {
        return Err(response_error());
    }

    tools
        .iter()
        .map(|tool| {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                // Discovered tool names become Reborn capability suffixes, so
                // discovery rejects unsupported names instead of normalizing
                // them into potentially colliding capability IDs.
                .filter(|name| is_supported_mcp_tool_name(name, MAX_TOOL_NAME_BYTES))
                .ok_or_else(response_error)?;
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            if description.len() > MAX_TOOL_DESCRIPTION_BYTES
                || description.chars().any(is_unsupported_description_char)
            {
                return Err(response_error());
            }
            let input_schema = tool
                .get("inputSchema")
                .filter(|schema| schema.is_object())
                .cloned()
                .ok_or_else(response_error)?;
            if !is_supported_mcp_input_schema(
                &input_schema,
                MAX_SCHEMA_DEPTH,
                MAX_SCHEMA_NODES,
                MAX_SCHEMA_STRING_BYTES,
            ) {
                return Err(response_error());
            }
            let annotations = parse_tool_annotations(tool.get("annotations"))?;
            Ok(McpDiscoveredTool {
                name: name.to_string(),
                description: description.to_string(),
                input_schema,
                annotations,
            })
        })
        .collect()
}

fn is_supported_mcp_input_schema(
    schema: &Value,
    max_depth: u8,
    max_nodes: usize,
    max_string_bytes: usize,
) -> bool {
    let mut nodes = 0usize;
    validate_mcp_schema_value(
        schema,
        0,
        max_depth,
        max_nodes,
        max_string_bytes,
        &mut nodes,
    )
}

fn validate_mcp_schema_value(
    value: &Value,
    depth: u8,
    max_depth: u8,
    max_nodes: usize,
    max_string_bytes: usize,
    nodes: &mut usize,
) -> bool {
    if depth > max_depth {
        return false;
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > max_nodes {
        return false;
    }
    match value {
        Value::String(value) => {
            value.len() <= max_string_bytes && !value.chars().any(is_unsupported_description_char)
        }
        Value::Array(values) => values.iter().all(|value| {
            validate_mcp_schema_value(
                value,
                depth + 1,
                max_depth,
                max_nodes,
                max_string_bytes,
                nodes,
            )
        }),
        Value::Object(values) => values.values().all(|value| {
            validate_mcp_schema_value(
                value,
                depth + 1,
                max_depth,
                max_nodes,
                max_string_bytes,
                nodes,
            )
        }),
        _ => true,
    }
}

fn is_unsupported_description_char(value: char) -> bool {
    value.is_control() && !matches!(value, '\n' | '\r' | '\t')
}

fn parse_tool_annotations(value: Option<&Value>) -> Result<McpDiscoveredToolAnnotations, String> {
    let Some(value) = value else {
        return Ok(McpDiscoveredToolAnnotations::default());
    };
    let object = value.as_object().ok_or_else(response_error)?;
    Ok(McpDiscoveredToolAnnotations {
        destructive_hint: object
            .get("destructiveHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        side_effects_hint: object
            .get("sideEffectsHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        read_only_hint: object
            .get("readOnlyHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn is_supported_mcp_tool_name(value: &str, max_bytes: usize) -> bool {
    if value.is_empty() || value.len() > max_bytes || value.contains("..") {
        return false;
    }
    value.split('.').all(is_supported_mcp_tool_name_segment)
}

fn is_supported_mcp_tool_name_segment(segment: &str) -> bool {
    let Some(first) = segment.as_bytes().first().copied() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    segment.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
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
        "protocolVersion": STREAMABLE_HTTP_MCP_PROTOCOL_VERSION,
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
    #[error("MCP capability requires authentication")]
    AuthRequired {
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
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
        let auth_context = client_request.auth_context;
        let client_request = client_request.request;
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
            Err(error) => {
                return Err(release_after_failure(
                    governor,
                    reservation.id,
                    mcp_error_from_client_error(error, auth_context),
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
    ) -> Result<PreparedMcpClientRequest, McpError> {
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

        let auth_context = mcp_auth_context(&descriptor.provider, &descriptor.runtime_credentials);

        Ok(PreparedMcpClientRequest {
            request: McpClientRequest {
                provider: request.package.id.clone(),
                capability_id: request.capability_id.clone(),
                scope: request.scope.clone(),
                transport: transport.clone(),
                command: command.clone(),
                args: args.clone(),
                url: url.clone(),
                input: request.invocation.input.clone(),
                max_output_bytes: self.config.max_output_bytes,
            },
            auth_context,
        })
    }
}

fn mcp_error_from_client_error(error: McpClientError, auth_context: McpAuthContext) -> McpError {
    match error {
        McpClientError::Client { reason } => McpError::Client { reason },
        McpClientError::AuthRequired => McpError::AuthRequired {
            required_secrets: auth_context.required_secrets,
            credential_requirements: auth_context.credential_requirements,
        },
    }
}

fn mcp_auth_context(
    requester_extension: &ExtensionId,
    credentials: &[RuntimeCredentialRequirement],
) -> McpAuthContext {
    let mut required_secrets = Vec::new();
    let mut credential_requirements = Vec::new();
    for credential in credentials.iter().filter(|credential| credential.required) {
        match &credential.source {
            RuntimeCredentialRequirementSource::SecretHandle => {
                required_secrets.push(credential.handle.clone());
            }
            RuntimeCredentialRequirementSource::ProductAuthAccount { provider, .. } => {
                credential_requirements.push(RuntimeCredentialAuthRequirement {
                    provider: provider.clone(),
                    requester_extension: requester_extension.clone(),
                    provider_scopes: credential.provider_scopes.clone(),
                });
            }
        }
    }
    McpAuthContext {
        required_secrets,
        credential_requirements,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_tools_list_result_rejects_oversized_tool_list() {
        let tools = (0..129)
            .map(|index| valid_tool(&format!("tool-{index}"), json!({"type": "object"})))
            .collect::<Vec<_>>();

        let error = parse_tools_list_result(&json!({ "tools": tools }))
            .expect_err("tool discovery must cap returned tools");

        assert_eq!(error, "response_error");
    }

    #[test]
    fn parse_tools_list_result_rejects_unsupported_description_control_char() {
        let mut tool = valid_tool("search", json!({"type": "object"}));
        tool["description"] = json!("bad\u{0000}description");

        let error = parse_tools_list_result(&json!({ "tools": [tool] }))
            .expect_err("unsupported description control characters must fail");

        assert_eq!(error, "response_error");
    }

    #[test]
    fn parse_tools_list_result_rejects_missing_or_non_object_schema() {
        let mut missing_schema = valid_tool("missing-schema", json!({"type": "object"}));
        missing_schema
            .as_object_mut()
            .expect("test tool object")
            .remove("inputSchema");
        let non_object_schema = valid_tool("bad-schema", json!("object please"));

        for tool in [missing_schema, non_object_schema] {
            let error = parse_tools_list_result(&json!({ "tools": [tool] }))
                .expect_err("schema must be present and object-shaped");

            assert_eq!(error, "response_error");
        }
    }

    #[test]
    fn parse_tools_list_result_rejects_unsafe_schema_strings_and_shape() {
        let cases = [
            valid_tool(
                "control",
                json!({"type": "object", "description": "bad\u{0008}schema"}),
            ),
            valid_tool(
                "long-string",
                json!({"type": "object", "description": "a".repeat(1025)}),
            ),
            valid_tool("too-deep", nested_schema(9)),
            valid_tool("too-many-nodes", wide_schema(513)),
        ];

        for tool in cases {
            let error = parse_tools_list_result(&json!({ "tools": [tool] }))
                .expect_err("unsafe schema strings and shape must fail");

            assert_eq!(error, "response_error");
        }
    }

    #[test]
    fn is_supported_mcp_tool_name_boundary_cases() {
        let exactly_128 = "a".repeat(128);
        let too_long = "a".repeat(129);

        assert!(!is_supported_mcp_tool_name("", 128));
        assert!(is_supported_mcp_tool_name(&exactly_128, 128));
        assert!(!is_supported_mcp_tool_name(&too_long, 128));
        assert!(!is_supported_mcp_tool_name("search..issues", 128));
        assert!(!is_supported_mcp_tool_name("Search", 128));
        assert!(!is_supported_mcp_tool_name("search._private", 128));
    }

    #[test]
    fn mcp_tool_name_strips_provider_prefix_for_canonical_tool_name() {
        let provider = ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.web_search").unwrap();

        assert_eq!(mcp_tool_name(&provider, &capability_id), "web_search");
    }

    #[test]
    fn parse_mcp_sse_response_skips_empty_data_keepalives() {
        let body = b"event: ping\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";

        let response = parse_mcp_sse_response(body, Some(7))
            .expect("empty SSE data lines should not abort parsing");

        assert_eq!(response.result, Some(json!({"ok": true})));
        assert!(!response.error);
    }

    fn valid_tool(name: &str, input_schema: Value) -> Value {
        json!({
            "name": name,
            "description": "Search hosted data",
            "inputSchema": input_schema
        })
    }

    fn nested_schema(depth: usize) -> Value {
        let mut value = json!({"type": "string"});
        for _ in 0..depth {
            value = json!({"type": "object", "properties": {"next": value}});
        }
        value
    }

    fn wide_schema(nodes: usize) -> Value {
        let properties = (0..nodes)
            .map(|index| (format!("field_{index}"), json!({"type": "string"})))
            .collect::<serde_json::Map<_, _>>();
        json!({"type": "object", "properties": properties})
    }
}
