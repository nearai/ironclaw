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
    http::CapabilityHostHttpRequest,
    ids::{CapabilityId, ExtensionId},
    resource::{ResourceScope, ResourceUsage},
};
use serde_json::Value;

use crate::contract::{
    McpClient, McpClientError, McpClientOutput, McpClientRequest, McpToolDiscoveryOutput,
};
use crate::diagnostics::{
    McpInvalidToolListCause, McpRequestDeniedCause, McpResponseErrorCause, invalid_tool_list,
    request_denied, response_error,
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
    mcp_auth_challenge_from_response, mcp_session_id_from_response, parse_mcp_response,
    protocol_version_from_initialize_response, validate_staged_credential_injections,
    validate_tools_call_credential_injections,
};

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
                        McpClientError::AuthRequired
                    } else {
                        McpClientError::AuthChallenge { challenge }
                    },
                );
            }
            return Err(McpClientError::client(response_error(
                McpResponseErrorCause::HttpStatus(response.status),
            )));
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
            return Err(McpClientError::client(response_error(
                McpResponseErrorCause::JsonRpcError {
                    code: error.code,
                    message: error.message,
                },
            )));
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
        if let Some(error) = initialized.response.error {
            return Err(McpClientError::client(response_error(
                McpResponseErrorCause::JsonRpcError {
                    code: error.code,
                    message: error.message,
                },
            )));
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
            .await?;
        accumulate_usage(&mut usage, call.usage);
        self.update_session_id(&session_key, call.session_id.clone())?;
        if let Some(error) = call.response.error {
            return Err(McpClientError::client(response_error(
                McpResponseErrorCause::JsonRpcError {
                    code: error.code,
                    message: error.message,
                },
            )));
        }
        let output = call.response.result.ok_or_else(|| {
            McpClientError::client(response_error(McpResponseErrorCause::MissingResult))
        })?;
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|err| {
                McpClientError::client(response_error(McpResponseErrorCause::ParseFailed(
                    err.to_string(),
                )))
            })?;
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

    #[test]
    fn mcp_tool_name_strips_provider_prefix_for_canonical_tool_name() {
        let provider = ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.web_search").unwrap();

        assert_eq!(mcp_tool_name(&provider, &capability_id), "web_search");
    }
}
