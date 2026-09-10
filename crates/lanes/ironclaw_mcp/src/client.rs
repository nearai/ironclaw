//! The Streamable-HTTP [`McpClient`] implementation.
//!
//! This module owns the *sequence*: plan a request, run the
//! `initialize` / `notifications/initialized` handshake, then `tools/call` or
//! the `tools/list` paging loop — and the per-invocation session state that
//! sequence depends on. It frames nothing itself (`jsonrpc` does), decides no
//! tool-shape rule (`discovery` does), and sends nothing directly (`egress`
//! does).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_host_api::{
    action::NetworkMethod,
    dispatch::{ProviderDiagnostic, UntrustedProviderMessage},
    http::CapabilityHostHttpRequest,
    ids::{CapabilityId, ExtensionId},
    resource::{ResourceScope, ResourceUsage},
};
use serde_json::Value;

use crate::contract::{
    McpClient, McpClientError, McpClientOutput, McpClientRequest, McpToolDiscoveryOutput,
};
use crate::diagnostics::{
    McpInvalidToolListCause, McpProviderRejectionCause, McpRequestDeniedCause,
    McpResponseErrorCause, invalid_tool_list, provider_error_code, request_denied, response_error,
};
use crate::discovery::{
    MAX_DISCOVERED_MCP_TOOLS, MAX_MCP_TOOLS_CATALOG_BYTES, MAX_MCP_TOOLS_LIST_PAGES,
    parse_tools_list_page,
};
use crate::egress::{
    McpHostHttp, McpHostHttpEgressPlan, McpHostHttpEgressPlanRequest, McpHostHttpEgressPlanner,
    effective_mcp_response_body_limit, mcp_client_http_error, requires_host_http_egress,
};
use crate::jsonrpc::{
    MCP_PROTOCOL_VERSION_HEADER, McpJsonRpcExchange, McpJsonRpcMethod, McpJsonRpcResponse,
    encode_json_rpc_request, is_mcp_auth_response_status, json_rpc_initialize_params,
    params_with_sep414_meta,
    mcp_auth_challenge_from_response, mcp_session_id_from_response, parse_mcp_response,
    protocol_version_from_initialize_response, validate_staged_credential_injections,
    validate_tools_call_credential_injections,
};

/// A defensive fan-out ceiling for one MCP `CallToolResult.content` array.
/// The mediated response-body limit bounds bytes before parsing; this second
/// bound prevents a tiny-block array from expanding during projection.
const MAX_MCP_CONTENT_BLOCKS: usize = 1_024;
const MAX_MCP_CONTENT_TYPE_BYTES: usize = 128;

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

    /// Perform only the MCP initialization handshake.
    ///
    /// Registration uses this to distinguish credential-free access from an
    /// authentication challenge without fetching or admitting the tool
    /// catalog. The temporary session is always discarded before returning.
    pub async fn probe_auth(
        &self,
        request: McpClientRequest,
    ) -> Result<ResourceUsage, McpClientError> {
        if !requires_host_http_egress(&request.transport) {
            return Err(McpClientError::client(request_denied(
                McpRequestDeniedCause::UnsupportedTransport,
            )));
        }
        let url = request.url.as_deref().ok_or_else(|| {
            McpClientError::client(request_denied(McpRequestDeniedCause::MissingUrl))
        })?;
        let session_key = McpHostHttpSessionKey::new(&request.scope, &request.provider, url);
        let _session_cleanup =
            McpHostHttpSessionCleanup::new(Arc::clone(&self.state), session_key.clone());
        self.initialize_session(&request, &session_key).await
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
        let url = request.url.as_deref().ok_or_else(|| {
            McpClientError::client(request_denied(McpRequestDeniedCause::MissingUrl))
        })?;
        let body = encode_json_rpc_request(id, method.as_str(), params.clone())
            .map_err(McpClientError::client)?;
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
        // Stamp the SEP-414 `_meta` attribution on the tool-facing methods,
        // and ONLY for providers whose manifest opted in ([mcp] attribution =
        // "sep414", which the planner turns into the flag). Every other
        // provider keeps today's wire shape — no host identity or conversation
        // identifiers reach a server that never asked for them. `initialize`
        // keeps its exact handshake params, and neither it nor the
        // post-handshake `notifications/initialized` carries per-turn
        // attribution. Threading this through the shared planner (rather than
        // each call site) guarantees `tools/list` and `tools/call` agree.
        let body = if plan.sep414_attribution
            && matches!(
                method,
                McpJsonRpcMethod::ToolsList | McpJsonRpcMethod::ToolsCall
            ) {
            // A provider that opted in and gets no thread key cannot correlate
            // the call to a conversation, and silently falls back to guessing.
            // Say so once per call rather than let it degrade quietly.
            if request.scope.thread_id.is_none() {
                tracing::debug!(
                    provider = %request.provider,
                    capability_id = %request.capability_id,
                    method = method.as_str(),
                    "SEP-414 attribution is on but the turn scope carries no thread; \
                     omitting io.ironclaw/threadId"
                );
            }
            let params = Some(params_with_sep414_meta(params, &request.scope));
            encode_json_rpc_request(id, method.as_str(), params)
                .map_err(McpClientError::client)?
        } else {
            body
        };
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
        // Explicit `McpClientError::client`, not `?`: the removed
        // `impl From<String> for McpClientError` used to make this conversion
        // invisible, which is how the crate charter's "no module builds a
        // failure string of its own" rule lost its one enforceable seam. The
        // reason itself already comes from `diagnostics::request_denied`.
        let credential_injections = planned
            .method
            .credential_injections(planned.plan.credential_injections)
            .map_err(McpClientError::client)?;
        let response = self
            .http
            .request(CapabilityHostHttpRequest {
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

        let usage = ResourceUsage::default().set_network_egress_bytes(response.request_bytes);

        if !(200..300).contains(&response.status) {
            if is_mcp_auth_response_status(response.status) {
                // Bare `AuthRequired` when the response gives us nothing to
                // act on; `AuthChallenge` only when it actually carries
                // WWW-Authenticate/resource-metadata to resolve.
                let challenge = mcp_auth_challenge_from_response(&response);
                return Err(
                    if challenge.www_authenticate_metadata.is_empty()
                        && challenge.protected_resource_metadata.is_empty()
                    {
                        McpClientError::AuthRequired {
                            usage: usage.clone(),
                        }
                    } else {
                        McpClientError::AuthChallenge {
                            challenge,
                            usage: usage.clone(),
                        }
                    },
                );
            }
            return Err(McpClientError::ProviderRejected {
                diagnostic: Box::new(ProviderDiagnostic {
                    code: Some(provider_error_code(McpProviderRejectionCause::HttpStatus(
                        response.status,
                    ))),
                    message: None,
                    retry_after: None,
                }),
                usage,
            });
        }
        let session_id = mcp_session_id_from_response(&response).map_err(McpClientError::client)?;

        if response.status == 202 && planned.id.is_none() {
            return Ok(McpJsonRpcExchange {
                response: McpJsonRpcResponse {
                    result: None,
                    error: None,
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
            .map_err(|_| {
                McpClientError::client(request_denied(McpRequestDeniedCause::SessionStatePoisoned))
            })
    }

    fn store_session(
        &self,
        session_key: &McpHostHttpSessionKey,
        session: McpHostHttpSession,
    ) -> Result<(), McpClientError> {
        let mut guard = self.state.sessions.lock().map_err(|_| {
            McpClientError::client(request_denied(McpRequestDeniedCause::SessionStatePoisoned))
        })?;
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
        let mut guard = self.state.sessions.lock().map_err(|_| {
            McpClientError::client(request_denied(McpRequestDeniedCause::SessionStatePoisoned))
        })?;
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
        if let Some(error) = initialize.response.error {
            return Err(McpClientError::ProviderRejected {
                diagnostic: Box::new(json_rpc_provider_diagnostic(error.code, error.message)),
                usage,
            });
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
            .await
            .map_err(|error| mcp_client_error_with_prior_usage(error, usage.clone()))?;
        accumulate_usage(&mut usage, initialized.usage);
        self.update_session_id(session_key, initialized.session_id.clone())?;
        if let Some(error) = initialized.response.error {
            return Err(McpClientError::ProviderRejected {
                diagnostic: Box::new(json_rpc_provider_diagnostic(error.code, error.message)),
                usage,
            });
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
            return Err(McpClientError::client(request_denied(
                McpRequestDeniedCause::UnsupportedTransport,
            )));
        }

        let url = request.url.as_deref().ok_or_else(|| {
            McpClientError::client(request_denied(McpRequestDeniedCause::MissingUrl))
        })?;
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
            .await
            .map_err(|error| mcp_client_error_with_prior_usage(error, usage.clone()))?;
        accumulate_usage(&mut usage, call.usage);
        self.update_session_id(&session_key, call.session_id.clone())?;
        if let Some(error) = call.response.error {
            return Err(McpClientError::ProviderRejected {
                diagnostic: Box::new(json_rpc_provider_diagnostic(error.code, error.message)),
                usage,
            });
        }
        let output = call.response.result.ok_or_else(|| {
            McpClientError::client(response_error(McpResponseErrorCause::MissingResult))
        })?;
        let provider_rejection =
            call_tool_rejection_message(&output).map(|message| ProviderDiagnostic {
                code: Some(provider_error_code(McpProviderRejectionCause::ToolRejected)),
                message: Some(UntrustedProviderMessage::new(message)),
                retry_after: None,
            });
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|err| {
                McpClientError::client(response_error(McpResponseErrorCause::ParseFailed(
                    err.to_string(),
                )))
            })?;
        usage.output_bytes = usage.output_bytes.max(output_bytes);

        // Tool-declared rejection keeps the existing diagnostic/accounting
        // path. Successful output is projected at the MCP protocol boundary
        // before the generic durable writer ever sees it.
        let output = if provider_rejection.is_some() {
            output
        } else if let Some(projected) = project_call_tool_result(&output) {
            projected
        } else {
            return Err(McpClientError::InvalidToolResult {
                reason: response_error(McpResponseErrorCause::InvalidToolResult),
                usage,
            });
        };

        Ok(McpClientOutput {
            output,
            usage,
            output_bytes: Some(output_bytes),
            provider_rejection,
        })
    }

    async fn discover_tools(
        &self,
        request: McpClientRequest,
        max_tools: u32,
    ) -> Result<McpToolDiscoveryOutput, McpClientError> {
        if !requires_host_http_egress(&request.transport) {
            return Err(McpClientError::client(request_denied(
                McpRequestDeniedCause::UnsupportedTransport,
            )));
        }

        let url = request.url.as_deref().ok_or_else(|| {
            McpClientError::client(request_denied(McpRequestDeniedCause::MissingUrl))
        })?;
        let session_key = McpHostHttpSessionKey::new(&request.scope, &request.provider, url);
        let _session_cleanup =
            McpHostHttpSessionCleanup::new(Arc::clone(&self.state), session_key.clone());

        if max_tools == 0 {
            return Err(McpClientError::invalid_tool_catalog(invalid_tool_list(
                McpInvalidToolListCause::TooManyTools,
            )));
        }

        // The first page's plan is built before `initialize_session` runs (not
        // inside the loop below) so the planner observes `tools/list` before
        // `initialize`/`notifications/initialized`, matching the original
        // single-page discovery ordering that callers and tests depend on.
        // Only pages after the first are planned lazily inside the loop, once
        // a `nextCursor` is known.
        let first_tools_list_id = self.next_request_id();
        let first_tools_list_plan = self.plan_json_rpc(
            &request,
            Some(first_tools_list_id),
            McpJsonRpcMethod::ToolsList,
            None,
        )?;
        validate_staged_credential_injections(&first_tools_list_plan.plan.credential_injections)
            .map_err(McpClientError::client)?;

        let mut usage = self.initialize_session(&request, &session_key).await?;
        let mut discovered = Vec::new();
        let mut accepted_catalog_bytes = 0usize;
        let mut cursor = None;
        let mut pending_plan = Some(first_tools_list_plan);
        for page in 1..=MAX_MCP_TOOLS_LIST_PAGES {
            let tools_list_plan = match pending_plan.take() {
                Some(plan) => plan,
                None => {
                    let tools_list_id = self.next_request_id();
                    let plan = self.plan_json_rpc(
                        &request,
                        Some(tools_list_id),
                        McpJsonRpcMethod::ToolsList,
                        cursor
                            .as_ref()
                            .map(|cursor| serde_json::json!({ "cursor": cursor })),
                    )?;
                    validate_staged_credential_injections(&plan.plan.credential_injections)
                        .map_err(McpClientError::client)?;
                    plan
                }
            };

            let tools = self
                .send_planned_json_rpc(&request, &session_key, tools_list_plan)
                .await?;
            accumulate_usage(&mut usage, tools.usage);
            self.update_session_id(&session_key, tools.session_id.clone())?;
            if let Some(error) = tools.response.error {
                return Err(McpClientError::client(response_error(
                    McpResponseErrorCause::JsonRpcError {
                        code: error.code,
                        message: error.message,
                    },
                )));
            }
            let result = tools.response.result.ok_or_else(|| {
                McpClientError::client(response_error(McpResponseErrorCause::MissingResult))
            })?;
            let page_bytes = result
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| serde_json::to_vec(tools).ok())
                .map_or(usize::MAX, |bytes| bytes.len());
            let (page_tools, next_cursor) =
                parse_tools_list_page(&result).map_err(McpClientError::invalid_tool_catalog)?;
            accepted_catalog_bytes = accepted_catalog_bytes.saturating_add(page_bytes);
            if discovered.len().saturating_add(page_tools.len()) > MAX_DISCOVERED_MCP_TOOLS
                || discovered.len().saturating_add(page_tools.len()) > max_tools as usize
            {
                return Err(McpClientError::invalid_tool_catalog(invalid_tool_list(
                    McpInvalidToolListCause::TooManyTools,
                )));
            }
            if accepted_catalog_bytes > MAX_MCP_TOOLS_CATALOG_BYTES {
                return Err(McpClientError::invalid_tool_catalog(invalid_tool_list(
                    McpInvalidToolListCause::CatalogTooLarge,
                )));
            }
            discovered.extend(page_tools);
            match next_cursor {
                Some(_next_cursor) if page == MAX_MCP_TOOLS_LIST_PAGES => {
                    return Err(McpClientError::invalid_tool_catalog(invalid_tool_list(
                        McpInvalidToolListCause::TooManyPages,
                    )));
                }
                Some(next_cursor) => cursor = Some(next_cursor),
                None => break,
            }
        }
        Ok(McpToolDiscoveryOutput {
            tools: discovered,
            usage,
        })
    }
}

fn mcp_client_error_with_prior_usage(
    error: McpClientError,
    mut prior_usage: ResourceUsage,
) -> McpClientError {
    match error {
        McpClientError::AuthRequired { usage } => {
            accumulate_usage(&mut prior_usage, usage);
            McpClientError::AuthRequired { usage: prior_usage }
        }
        McpClientError::AuthChallenge { challenge, usage } => {
            accumulate_usage(&mut prior_usage, usage);
            McpClientError::AuthChallenge {
                challenge,
                usage: prior_usage,
            }
        }
        McpClientError::ProviderRejected { diagnostic, usage } => {
            accumulate_usage(&mut prior_usage, usage);
            McpClientError::ProviderRejected {
                diagnostic,
                usage: prior_usage,
            }
        }
        McpClientError::InvalidToolResult { reason, usage } => {
            accumulate_usage(&mut prior_usage, usage);
            McpClientError::InvalidToolResult {
                reason,
                usage: prior_usage,
            }
        }
        other => other,
    }
}

fn call_tool_rejection_message(result: &Value) -> Option<String> {
    if result.get("isError").and_then(Value::as_bool) != Some(true) {
        return None;
    }

    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if text.is_empty() {
        Some("MCP server rejected the tool call".to_string())
    } else {
        Some(text)
    }
}

/// Project an MCP `CallToolResult` into model-facing open JSON.
///
/// This is the protocol lane's output security boundary: it understands MCP
/// content discriminators, not capability ids or vendor fields. Semantic JSON
/// and text/resource metadata survive; inline binary and arbitrary future
/// block payloads do not.
fn project_call_tool_result(result: &Value) -> Option<Value> {
    let source = result.as_object()?;
    let content = source.get("content")?.as_array()?;
    if content.len() > MAX_MCP_CONTENT_BLOCKS {
        return None;
    }
    if source
        .get("structuredContent")
        .is_some_and(|value| !value.is_object())
        || source
            .get("isError")
            .is_some_and(|value| !value.is_boolean())
    {
        return None;
    }

    let mut projected = serde_json::Map::new();
    projected.insert(
        "content".to_string(),
        Value::Array(
            content
                .iter()
                .map(project_content_block)
                .collect::<Option<Vec<_>>>()?,
        ),
    );
    if let Some(structured) = source.get("structuredContent") {
        projected.insert("structuredContent".to_string(), structured.clone());
    }
    if let Some(is_error) = source.get("isError") {
        projected.insert("isError".to_string(), is_error.clone());
    }
    Some(Value::Object(projected))
}

fn project_content_block(block: &Value) -> Option<Value> {
    let source = block.as_object()?;
    let block_type = source.get("type").and_then(Value::as_str)?;
    let mut projected = serde_json::Map::new();
    match block_type {
        "text" => {
            let text = source.get("text").and_then(Value::as_str)?;
            projected.insert("type".to_string(), Value::String("text".to_string()));
            projected.insert("text".to_string(), Value::String(text.to_string()));
        }
        kind @ ("image" | "audio") => {
            let mime_type = source.get("mimeType").and_then(Value::as_str)?;
            source.get("data").and_then(Value::as_str)?;
            projected.insert("type".to_string(), Value::String(kind.to_string()));
            projected.insert("mimeType".to_string(), Value::String(mime_type.to_string()));
            projected.insert(
                "encoding".to_string(),
                Value::String("binary_unsupported".to_string()),
            );
        }
        "resource_link" => {
            let (Some(name), Some(uri)) = (
                source.get("name").and_then(Value::as_str),
                source.get("uri").and_then(Value::as_str),
            ) else {
                return None;
            };
            projected.insert(
                "type".to_string(),
                Value::String("resource_link".to_string()),
            );
            projected.insert("name".to_string(), Value::String(name.to_string()));
            projected.insert("uri".to_string(), Value::String(uri.to_string()));
            if !copy_optional_string_fields(
                source,
                &mut projected,
                &["title", "description", "mimeType"],
            ) {
                return None;
            }
            if let Some(size) = source.get("size") {
                let size = size.as_u64()?;
                projected.insert("size".to_string(), Value::from(size));
            }
        }
        "resource" => {
            let resource = source.get("resource").and_then(Value::as_object)?;
            let uri = resource.get("uri").and_then(Value::as_str)?;
            let mut projected_resource = serde_json::Map::new();
            projected_resource.insert("uri".to_string(), Value::String(uri.to_string()));
            if !copy_optional_string_fields(resource, &mut projected_resource, &["mimeType"]) {
                return None;
            }
            match (
                resource.get("text").and_then(Value::as_str),
                resource.get("blob").and_then(Value::as_str),
            ) {
                (Some(text), None) => {
                    projected_resource.insert("text".to_string(), Value::String(text.to_string()));
                }
                (None, Some(_)) => {
                    projected_resource.insert(
                        "encoding".to_string(),
                        Value::String("binary_unsupported".to_string()),
                    );
                }
                _ => return None,
            }
            projected.insert("type".to_string(), Value::String("resource".to_string()));
            projected.insert("resource".to_string(), Value::Object(projected_resource));
        }
        other => {
            projected.insert("type".to_string(), Value::String("unsupported".to_string()));
            let mut normalized: String = other
                .chars()
                .map(|character| {
                    if character.is_control() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect();
            if normalized.len() > MAX_MCP_CONTENT_TYPE_BYTES {
                const ELLIPSIS: &str = "...";
                let mut end = MAX_MCP_CONTENT_TYPE_BYTES - ELLIPSIS.len();
                while end > 0 && !normalized.is_char_boundary(end) {
                    end -= 1;
                }
                normalized.truncate(end);
                normalized.push_str(ELLIPSIS);
            }
            projected.insert("original_type".to_string(), Value::String(normalized));
            projected.insert(
                "status".to_string(),
                Value::String("unsupported_content_type".to_string()),
            );
        }
    }
    Some(Value::Object(projected))
}

fn copy_optional_string_fields(
    source: &serde_json::Map<String, Value>,
    projected: &mut serde_json::Map<String, Value>,
    fields: &[&str],
) -> bool {
    for field in fields {
        match source.get(*field) {
            None => {}
            Some(value) if value.is_string() => {
                projected.insert((*field).to_string(), value.clone());
            }
            Some(_) => return false,
        }
    }
    true
}

fn json_rpc_provider_diagnostic(code: Option<i64>, message: Option<String>) -> ProviderDiagnostic {
    ProviderDiagnostic {
        code: code.map(|code| provider_error_code(McpProviderRejectionCause::JsonRpcError(code))),
        message: message.map(UntrustedProviderMessage::new),
        retry_after: None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::{
        McpHostHttpError, McpHostHttpResponse, StaticMcpHostHttpEgressPlanner,
    };
    use ironclaw_host_api::http::CapabilityHostHttpRequest;
    use ironclaw_host_api::ids::{
        AgentId, InvocationId, MissionId, ProjectId, TenantId, ThreadId, UserId,
    };
    use serde_json::json;

    #[test]
    fn mcp_tool_name_strips_provider_prefix_for_canonical_tool_name() {
        let provider = ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.web_search").unwrap();

        assert_eq!(mcp_tool_name(&provider, &capability_id), "web_search");
    }

    #[test]
    fn call_tool_rejection_message_joins_non_empty_text_blocks() {
        let result = serde_json::json!({
            "isError": true,
            "content": [
                {"type": "text", "text": "  channel not found  "},
                {"type": "text", "text": ""},
                {"type": "text", "text": "retry with a valid channel"},
            ],
        });

        assert_eq!(
            call_tool_rejection_message(&result),
            Some("channel not found; retry with a valid channel".to_string())
        );
    }

    #[test]
    fn call_tool_rejection_message_falls_back_when_content_is_empty() {
        let result = serde_json::json!({
            "isError": true,
            "content": [],
        });

        assert_eq!(
            call_tool_rejection_message(&result),
            Some("MCP server rejected the tool call".to_string())
        );
    }

    #[test]
    fn call_tool_rejection_message_falls_back_when_content_missing() {
        let result = serde_json::json!({"isError": true});

        assert_eq!(
            call_tool_rejection_message(&result),
            Some("MCP server rejected the tool call".to_string())
        );
    }

    #[test]
    fn call_tool_rejection_message_returns_none_when_not_an_error() {
        let result = serde_json::json!({"isError": false, "content": []});

        assert_eq!(call_tool_rejection_message(&result), None);
    }

    struct UnusedHttp;

    #[async_trait::async_trait]
    impl McpHostHttp for UnusedHttp {
        async fn request(
            &self,
            _request: CapabilityHostHttpRequest,
        ) -> Result<McpHostHttpResponse, McpHostHttpError> {
            unreachable!("plan_json_rpc must not touch the transport")
        }
    }

    fn planning_client() -> McpHostHttpClient<UnusedHttp, StaticMcpHostHttpEgressPlanner> {
        McpHostHttpClient::new(
            UnusedHttp,
            StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan::default()),
        )
    }

    /// A planner whose provider opted into SEP-414 attribution
    /// (`[mcp] attribution = "sep414"`).
    fn attributed_planning_client() -> McpHostHttpClient<UnusedHttp, StaticMcpHostHttpEgressPlanner>
    {
        McpHostHttpClient::new(
            UnusedHttp,
            StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan {
                sep414_attribution: true,
                ..McpHostHttpEgressPlan::default()
            }),
        )
    }

    fn scope_with_thread(thread_id: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: Some(MissionId::new("mission-a").unwrap()),
            thread_id: thread_id.map(|id| ThreadId::new(id).unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn meta_probe_request(scope: ResourceScope, input: Value) -> McpClientRequest {
        McpClientRequest {
            provider: ExtensionId::new("agent-market").unwrap(),
            capability_id: CapabilityId::new("agent-market.search_agents").unwrap(),
            scope,
            transport: "http".to_string(),
            command: None,
            args: Vec::new(),
            url: Some("https://mcp.example.test/rpc".to_string()),
            input,
            max_output_bytes: 1_000_000,
        }
    }

    /// The outbound `params` object that `plan_json_rpc` actually encodes.
    fn planned_params(
        client: &McpHostHttpClient<UnusedHttp, StaticMcpHostHttpEgressPlanner>,
        request: &McpClientRequest,
        method: McpJsonRpcMethod,
        params: Option<Value>,
    ) -> Value {
        let planned = client
            .plan_json_rpc(request, Some(1), method, params)
            .expect("planning succeeds");
        let decoded: Value = serde_json::from_slice(&planned.body).expect("body is JSON");
        decoded.get("params").cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn tools_call_stamps_sep414_thread_attribution_on_the_wire() {
        let client = attributed_planning_client();
        let scope = scope_with_thread(Some("thread-a"));
        let expected_invocation = scope.invocation_id.to_string();
        let request = meta_probe_request(scope, json!({"limit": 10}));

        let params = planned_params(
            &client,
            &request,
            McpJsonRpcMethod::ToolsCall,
            Some(json!({ "name": "search_agents", "arguments": { "limit": 10 } })),
        );

        // Attribution is carried in `_meta`, never derived from arguments — this
        // is what lets one stable per-user token serve concurrent jobs without a
        // buyer seeing another buyer's connector.
        assert_eq!(params["_meta"]["io.ironclaw/threadId"], json!("thread-a"));
        assert_eq!(params["_meta"]["io.ironclaw/userId"], json!("user-a"));
        // The invocation id must be THE turn's id, not merely some string —
        // it is the provider-side replay-dedup key.
        assert_eq!(
            params["_meta"]["io.ironclaw/invocationId"],
            json!(expected_invocation)
        );
        // Original tool arguments survive the merge untouched.
        assert_eq!(params["name"], json!("search_agents"));
        assert_eq!(params["arguments"], json!({ "limit": 10 }));
    }

    #[test]
    fn tools_list_stamps_sep414_thread_attribution_even_without_other_params() {
        let client = attributed_planning_client();
        let request = meta_probe_request(scope_with_thread(Some("thread-a")), Value::Null);

        // `tools/list` carries no other params (`None`) — the block must still
        // materialize so per-turn discovery is attributed to its thread.
        let params = planned_params(&client, &request, McpJsonRpcMethod::ToolsList, None);

        assert_eq!(params["_meta"]["io.ironclaw/threadId"], json!("thread-a"));
        assert_eq!(params["_meta"]["io.ironclaw/userId"], json!("user-a"));
    }

    #[test]
    fn initialize_handshake_is_not_stamped_with_attribution() {
        let client = attributed_planning_client();
        let request = meta_probe_request(scope_with_thread(Some("thread-a")), Value::Null);

        // The handshake predates any turn attribution; stamping it would corrupt
        // the exact `initialize` params contract.
        let params = planned_params(
            &client,
            &request,
            McpJsonRpcMethod::Initialize,
            Some(json!({ "protocolVersion": "2025-06-18" })),
        );

        assert!(params.get("_meta").is_none());
        assert_eq!(params["protocolVersion"], json!("2025-06-18"));
    }

    /// The privacy contract: a provider that did NOT opt in receives no
    /// attribution at all — its wire shape is byte-identical to today's.
    #[test]
    fn non_opted_provider_gets_no_attribution() {
        let client = planning_client();
        let request =
            meta_probe_request(scope_with_thread(Some("thread-a")), json!({"limit": 10}));

        let params = planned_params(
            &client,
            &request,
            McpJsonRpcMethod::ToolsCall,
            Some(json!({ "name": "search_agents", "arguments": { "limit": 10 } })),
        );

        assert!(params.get("_meta").is_none(), "no opt-in ⇒ no _meta block");
        assert_eq!(params["name"], json!("search_agents"));
        assert_eq!(params["arguments"], json!({ "limit": 10 }));
    }

    #[test]
    fn thread_less_scope_omits_thread_id_but_keeps_user_attribution() {
        let client = attributed_planning_client();
        let request = meta_probe_request(scope_with_thread(None), Value::Null);

        let params = planned_params(&client, &request, McpJsonRpcMethod::ToolsList, None);

        // A non-threaded runtime sends no thread key (absent, not empty) but is
        // still user-attributed.
        assert!(params["_meta"].get("io.ironclaw/threadId").is_none());
        assert_eq!(params["_meta"]["io.ironclaw/userId"], json!("user-a"));
    }
}
