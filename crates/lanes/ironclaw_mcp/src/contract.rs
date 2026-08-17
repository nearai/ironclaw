//! The vocabulary a caller of `ironclaw_mcp` names.
//!
//! Everything public about this lane is declared here: the host-owned limits,
//! the invocation/request/output shapes, the two traits a composition root
//! wires (`McpClient` inward, `McpExecutor` outward), and the error taxonomy
//! both of them speak. Protocol framing, transport, and resource accounting
//! belong to `jsonrpc`, `egress`, and `runtime` respectively.

use async_trait::async_trait;
use ironclaw_extension_contracts::hosted_mcp::{HostedMcpDiscoveredTool, McpAuthChallenge};
use ironclaw_extension_contracts::runtime::ExtensionRuntime;
use ironclaw_host_api::{
    capability::CapabilityDescriptor,
    decision::RuntimeCredentialAuthRequirement,
    dispatch::ProviderDiagnostic,
    ids::{CapabilityId, ExtensionId, SecretHandle},
    resource::{
        CapabilityHostResult, ResourceEstimate, ResourceReceipt, ResourceReservation,
        ResourceScope, ResourceUsage, RuntimeResourceBudget, RuntimeResourceError,
    },
    runtime::RuntimeKind,
};
use serde_json::Value;
use thiserror::Error;

use crate::diagnostics::{McpRequestDeniedCause, request_denied};

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
    /// The extension whose manifest declares this lane.
    ///
    /// The lane deliberately does **not** receive the `ExtensionPackage`: it
    /// read only the id, the capability descriptors, and the runtime stanza,
    /// and taking the package forced a `runtimes -> loops` dependency on the
    /// registry crate (the W7 `ironclaw_mcp -> ironclaw_extension_registry` exception).
    /// The caller, which owns the package, projects those three.
    ///
    /// **Caller obligation (the cost of that carve-out).** `extension`,
    /// `capabilities`, and `runtime` are three independent borrows, so the type
    /// no longer *structurally* guarantees they came from one package the way
    /// `&ExtensionPackage` did. `execute_extension_json` re-checks the
    /// descriptor half (`descriptor.provider == extension`), but nothing in an
    /// `&ExtensionRuntime` identifies its owning extension, so the runtime half
    /// cannot be re-derived here — a caller that paired extension A's
    /// descriptors with extension B's runtime stanza would authenticate as A
    /// and dial B. **Always project all three from the same `ExtensionPackage`
    /// in one expression.** The single production caller
    /// (`ironclaw_host_runtime::services::runtime_adapters`) does exactly that.
    /// Restoring the compile-time binding needs a sealed projection minted by
    /// the package owner — it cannot be a check inside this lane, and it must
    /// not be a re-addition of the registry edge; tracked with the WS3 lane
    /// work.
    pub extension: &'a ExtensionId,
    pub capabilities: &'a [CapabilityDescriptor],
    pub runtime: &'a ExtensionRuntime,
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
    /// Protocol-level rejection returned after transport completed. Transport
    /// failures still use `McpClientError`.
    pub provider_rejection: Option<ProviderDiagnostic>,
}

impl McpClientOutput {
    pub fn json(value: Value) -> Self {
        Self {
            output: value,
            usage: ResourceUsage::default(),
            output_bytes: None,
            provider_rejection: None,
        }
    }
}

/// Result of a hosted MCP schema-discovery pass.
///
/// Discovered tools use the extension-domain [`HostedMcpDiscoveredTool`] shape
/// directly: `ironclaw_mcp` parses `tools/list` into the same descriptor the
/// extension domain consumes, so there is no separate MCP-local mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolDiscoveryOutput {
    pub tools: Vec<HostedMcpDiscoveredTool>,
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
        max_tools: u32,
    ) -> Result<McpToolDiscoveryOutput, McpClientError> {
        let _ = (request, max_tools);
        Err(McpClientError::client(request_denied(
            McpRequestDeniedCause::UnsupportedTransport,
        )))
    }
}

/// Stable, sanitized MCP client-side failure categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpClientError {
    Client {
        reason: String,
    },
    /// The server completed `tools/list`, but the advertised catalog violated
    /// the host's provider-neutral shape or safety contract. Repeating OAuth
    /// or the same request cannot repair this generation.
    InvalidToolCatalog {
        reason: String,
    },
    AuthRequired {
        usage: ResourceUsage,
    },
    /// A hosted server returned 401/403. The challenge is header-derived and
    /// deliberately redacted; it contains no remote response body or tokens.
    AuthChallenge {
        challenge: McpAuthChallenge,
        usage: ResourceUsage,
    },
    ProviderRejected {
        diagnostic: Box<ProviderDiagnostic>,
        usage: ResourceUsage,
    },
}

impl McpClientError {
    pub fn client(reason: impl Into<String>) -> Self {
        Self::Client {
            reason: reason.into(),
        }
    }

    pub fn invalid_tool_catalog(reason: impl Into<String>) -> Self {
        Self::InvalidToolCatalog {
            reason: reason.into(),
        }
    }

    pub fn stable_reason(&self) -> &str {
        match self {
            Self::Client { reason } | Self::InvalidToolCatalog { reason } => reason,
            Self::AuthRequired { .. } | Self::AuthChallenge { .. } => "auth_required",
            Self::ProviderRejected { diagnostic, .. } => diagnostic
                .code
                .as_ref()
                .map_or("provider_rejected", |code| code.as_str()),
        }
    }
}

/// Full resource-governed MCP execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpExecutionResult {
    pub result: CapabilityHostResult,
    pub receipt: ResourceReceipt,
}

#[derive(Clone, PartialEq, Eq)]
pub struct McpProviderRejection {
    pub diagnostic: ProviderDiagnostic,
    pub receipt: ResourceReceipt,
    pub usage: ResourceUsage,
}

impl std::fmt::Debug for McpProviderRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpProviderRejection")
            .field("diagnostic", &"<redacted>")
            .field("receipt", &self.receipt)
            .field("usage", &self.usage)
            .finish()
    }
}

/// MCP runtime failures.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("resource governor error: {0}")]
    Resource(RuntimeResourceError),
    #[error("MCP client error: {reason}")]
    Client { reason: String },
    #[error("MCP provider rejected the tool call")]
    ProviderRejected(Box<McpProviderRejection>),
    #[error("MCP server advertised an invalid tool catalog: {reason}")]
    InvalidToolCatalog { reason: String },
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

impl From<RuntimeResourceError> for McpError {
    fn from(error: RuntimeResourceError) -> Self {
        Self::Resource(error)
    }
}

/// Object-safe MCP executor interface used by the kernel composition layer.
#[async_trait]
pub trait McpExecutor: Send + Sync {
    async fn execute_extension_json(
        &self,
        budget: &dyn RuntimeResourceBudget,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError>;
}
