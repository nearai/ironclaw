use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionPackage, ExtensionRegistry, ExtensionRuntime, SharedExtensionRegistry,
    package_with_discovered_hosted_mcp_tools,
};
use ironclaw_host_api::{ResourceScope, RuntimeHttpEgress};
use ironclaw_mcp::{McpClient, McpClientRequest, McpHostHttpClient, McpRuntimeHttpAdapter};

use crate::mcp::{MCP_RESPONSE_BODY_LIMIT, RegistryMcpEgressPlanner};

const MCP_DISCOVERY_TOOL_LIMIT: u32 = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedMcpDiscoveryError {
    Transient(String),
    Permanent(String),
    /// The remote rejected the currently staged account. This is a setup
    /// outcome, not a retryable transport failure.
    CredentialsRejected(ironclaw_host_api::McpAuthChallenge),
}

pub async fn discover_hosted_mcp_package(
    package: &ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ExtensionPackage, HostedMcpDiscoveryError> {
    discover_hosted_mcp_package_with_policy(package, scope, runtime_http_egress, None).await
}

pub async fn discover_hosted_mcp_package_with_policy(
    package: &ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    safety: Option<&crate::McpCatalogAdmissionPolicy>,
) -> Result<ExtensionPackage, HostedMcpDiscoveryError> {
    let (transport, command, args, url) = match &package.manifest.runtime {
        ExtensionRuntime::Mcp {
            transport,
            command,
            args,
            url,
        } if is_hosted_http_mcp_package(package) => (
            transport.clone(),
            command.clone(),
            args.clone(),
            url.clone(),
        ),
        _ => {
            return Err(HostedMcpDiscoveryError::Permanent(format!(
                "extension {} is not a host-bundled hosted MCP provider",
                package.id
            )));
        }
    };
    let registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    registry.upsert(package.clone()).map_err(|error| {
        HostedMcpDiscoveryError::Permanent(format!(
            "failed to prepare hosted MCP discovery: {error}"
        ))
    })?;
    let planning_capability_id = package
        .manifest
        .capabilities
        .first()
        .map(|capability| capability.id.clone())
        .ok_or_else(|| {
            HostedMcpDiscoveryError::Permanent(format!(
                "hosted MCP provider {} has no capability template",
                package.id
            ))
        })?;
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(runtime_http_egress),
        RegistryMcpEgressPlanner::new(registry),
    );
    let output = client
        .discover_tools(
            McpClientRequest {
                provider: package.id.clone(),
                capability_id: planning_capability_id,
                scope,
                transport,
                command,
                args,
                url,
                input: serde_json::Value::Null,
                max_output_bytes: MCP_RESPONSE_BODY_LIMIT,
            },
            MCP_DISCOVERY_TOOL_LIMIT,
        )
        .await
        .map_err(|error| match error {
            ironclaw_mcp::McpClientError::AuthChallenge { challenge } => {
                HostedMcpDiscoveryError::CredentialsRejected(challenge)
            }
            ironclaw_mcp::McpClientError::AuthRequired => {
                HostedMcpDiscoveryError::Transient("auth_required_without_challenge".to_string())
            }
            ironclaw_mcp::McpClientError::InvalidToolCatalog { reason } => {
                HostedMcpDiscoveryError::Permanent(reason)
            }
            error => HostedMcpDiscoveryError::Transient(error.stable_reason().to_string()),
        })?;
    if output.tools.is_empty() {
        return Err(HostedMcpDiscoveryError::Transient(format!(
            "hosted MCP provider {} returned no discoverable tools",
            package.id
        )));
    }
    if let Some(safety) = safety
        && matches!(
            safety.admit(&output.tools),
            crate::McpCatalogAdmission::Rejected { .. }
        )
    {
        return Err(HostedMcpDiscoveryError::Permanent(
            "hosted MCP catalog rejected by safety policy".to_string(),
        ));
    }
    package_with_discovered_hosted_mcp_tools(package, &output.tools)
        .map_err(|error| HostedMcpDiscoveryError::Permanent(error.to_string()))
}

pub use ironclaw_extensions::is_hosted_http_mcp_package;
