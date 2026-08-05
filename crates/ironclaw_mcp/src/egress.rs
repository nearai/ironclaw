//! The host-mediated HTTP seam.
//!
//! The MCP lane performs no networking of its own. It hands a
//! `CapabilityHostHttpRequest` to the [`McpHostHttp`] port, and the host-owned
//! [`McpHostHttpEgressPlanner`] — not the plugin input — supplies network
//! policy, credential handles, response limits, and timeouts. Everything that
//! decides *what* to send lives in `client`/`jsonrpc`; this module only decides
//! *how it leaves*.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_host_api::{
    action::{NetworkMethod, NetworkPolicy},
    http::{
        CapabilityHostHttpRequest, RuntimeCredentialInjection, RuntimeHttpEgress,
        RuntimeHttpEgressError, RuntimeHttpEgressResponse,
    },
    ids::{CapabilityId, ExtensionId},
    resource::ResourceScope,
    runtime::RuntimeKind,
};
use thiserror::Error;

use crate::contract::McpClientError;

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
        request: CapabilityHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError> {
        AssertUnwindSafe(
            self.egress
                .execute(request.into_runtime_request(RuntimeKind::Mcp)),
        )
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
        request: CapabilityHostHttpRequest,
    ) -> Result<McpHostHttpResponse, McpHostHttpError>;
}

#[async_trait]
impl<E> McpHostHttp for McpRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress + Send + Sync,
{
    async fn request(
        &self,
        request: CapabilityHostHttpRequest,
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
        request: CapabilityHostHttpRequest,
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
/// The planner is intentionally separate from [`McpClientRequest::input`](crate::McpClientRequest::input):
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

pub(crate) fn mcp_client_http_error(error: McpHostHttpError) -> McpClientError {
    match error {
        McpHostHttpError::Egress { reason } => McpClientError::client(reason),
    }
}

pub(crate) fn effective_mcp_response_body_limit(
    host_limit: Option<u64>,
    client_limit: u64,
) -> Option<u64> {
    Some(match host_limit {
        Some(limit) => limit.min(client_limit),
        None => client_limit,
    })
}

pub(crate) fn requires_host_http_egress(transport: &str) -> bool {
    matches!(transport, "http" | "sse")
}
