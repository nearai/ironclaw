//! The MCP seam.
//!
//! The provider logic owns no MCP client and no HTTP client: it speaks to the
//! server through [`McpMemoryTransport`]. Composition injects the production
//! implementation, an adapter over `ironclaw_mcp`'s host-mediated
//! `McpHostHttpClient`, so this crate inherits the lane's session handling,
//! protocol-version validation, and — critically — the host-mediated egress
//! boundary, without depending on a `runtimes`-layer crate.
//!
//! Tests substitute [`MockMcpMemoryTransport`], which is why every mapping in
//! [`crate::service`] is unit-testable with no server present.

use async_trait::async_trait;
use serde_json::Value;

/// One `tools/call` against the bound memory server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpMemoryToolCall {
    /// Tool name, from [`crate::McpMemoryConfig`].
    pub tool: String,
    /// Tool arguments. Built entirely by this provider from the TRUSTED
    /// [`ironclaw_host_api::resource::ResourceScope`] plus the host's request —
    /// never from model-supplied text. A memory server must not be able to
    /// learn a tenant, user, agent, or project from anything the model wrote.
    pub arguments: Value,
}

/// Why a tool call did not produce a usable result.
///
/// Deliberately narrow and non-cause-preserving on the wire side: the provider
/// maps every variant onto [`ironclaw_memory::MemoryServiceError::unavailable`],
/// so a remote failure degrades the lane instead of failing the turn. The host
/// records that degradation, which is what makes "the backend is down"
/// distinguishable from "nothing matched".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpMemoryTransportError {
    /// The call did not complete: transport, session, protocol, or timeout.
    #[error("mcp memory transport failed: {reason}")]
    Transport { reason: String },
    /// The server reported the tool call itself as an error.
    #[error("mcp memory tool reported an error: {reason}")]
    ToolError { reason: String },
    /// The server requires (re)authentication.
    #[error("mcp memory server requires authentication")]
    AuthRequired,
}

impl McpMemoryTransportError {
    pub fn transport(reason: impl Into<String>) -> Self {
        Self::Transport {
            reason: reason.into(),
        }
    }

    pub fn tool_error(reason: impl Into<String>) -> Self {
        Self::ToolError {
            reason: reason.into(),
        }
    }
}

/// The MCP `tools/call` seam this provider depends on.
#[async_trait]
pub trait McpMemoryTransport: Send + Sync {
    /// Call one tool and return its structured result payload.
    async fn call_tool(&self, call: McpMemoryToolCall) -> Result<Value, McpMemoryTransportError>;
}

/// A scripted transport for tests: records every call and replays a queued
/// response per call, repeating the last one once the queue is drained.
///
/// Panic-free by construction (a poisoned lock yields the fallback response), so
/// it is safe to compile outside `cfg(test)` behind the `test-support` feature.
#[cfg(any(test, feature = "test-support"))]
pub struct MockMcpMemoryTransport {
    calls: std::sync::Mutex<Vec<McpMemoryToolCall>>,
    responses: std::sync::Mutex<std::collections::VecDeque<Result<Value, McpMemoryTransportError>>>,
    fallback: Result<Value, McpMemoryTransportError>,
}

#[cfg(any(test, feature = "test-support"))]
impl std::fmt::Debug for MockMcpMemoryTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MockMcpMemoryTransport").finish()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl MockMcpMemoryTransport {
    /// Always answer with `body`.
    pub fn always_ok(body: Value) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            fallback: Ok(body),
        }
    }

    /// Always fail with `error`.
    pub fn always_err(error: McpMemoryTransportError) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            fallback: Err(error),
        }
    }

    /// Replay `responses` in order, then repeat `fallback`.
    pub fn scripted(
        responses: Vec<Result<Value, McpMemoryTransportError>>,
        fallback: Result<Value, McpMemoryTransportError>,
    ) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            responses: std::sync::Mutex::new(responses.into_iter().collect()),
            fallback,
        }
    }

    /// Every call made so far, in order.
    pub fn calls(&self) -> Vec<McpMemoryToolCall> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl McpMemoryTransport for MockMcpMemoryTransport {
    async fn call_tool(&self, call: McpMemoryToolCall) -> Result<Value, McpMemoryTransportError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
        let queued = self
            .responses
            .lock()
            .ok()
            .and_then(|mut responses| responses.pop_front());
        queued.unwrap_or_else(|| self.fallback.clone())
    }
}
