//! MCP adapter contracts for IronClaw Reborn.
//!
//! `ironclaw_mcp` adapts manifest-declared MCP tools into IronClaw
//! capabilities. It does not grant MCP servers ambient filesystem, secret, or
//! network authority; the host-selected client is the only integration point and
//! resource accounting still happens through the host governor.

use async_trait::async_trait;
use ironclaw_extensions::{ExtensionPackage, ExtensionRuntime};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ResourceEstimate, ResourceReservation, ResourceReservationId,
    ResourceScope, ResourceUsage, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
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
