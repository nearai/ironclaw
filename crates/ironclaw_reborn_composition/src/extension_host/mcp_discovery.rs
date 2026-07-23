use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionPackage, ExtensionRegistry, ExtensionRuntime, SharedExtensionRegistry,
    package_with_discovered_hosted_mcp_tools,
};
use ironclaw_host_api::{ResourceScope, RuntimeHttpEgress};
use ironclaw_mcp::{
    McpClient, McpClientError, McpClientRequest, McpHostHttpClient, McpRuntimeHttpAdapter,
};

use crate::extension_host::mcp::{MCP_RESPONSE_BODY_LIMIT, RegistryMcpEgressPlanner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostedMcpDiscoveryError {
    Transient(String),
    Permanent(String),
    /// The provider rejected the staged credentials mid-discovery (HTTP
    /// 401/403 during the `tools/list` exchange). Credentials were present
    /// pre-discovery, so this is not a retryable transport fault: retrying
    /// re-hits the same rejection forever. It must route the caller back
    /// through OAuth rather than looping in `setup_needed`.
    ReAuthRequired,
}

/// Map a concrete MCP client failure onto the discovery lifecycle outcome.
///
/// A provider `AuthRequired` (401/403 during discovery) is routed to
/// [`HostedMcpDiscoveryError::ReAuthRequired`] — the re-auth outcome — instead
/// of being folded into a retry-forever `Transient`. An invalid tool catalog
/// stays `Permanent` (repeating OAuth or the request cannot repair it). Every
/// other client failure (timeouts, 5xx, transport faults) stays `Transient`.
fn classify_discovery_error(error: McpClientError) -> HostedMcpDiscoveryError {
    match error {
        McpClientError::AuthRequired => HostedMcpDiscoveryError::ReAuthRequired,
        McpClientError::InvalidToolCatalog { reason } => HostedMcpDiscoveryError::Permanent(reason),
        McpClientError::Client { reason } => HostedMcpDiscoveryError::Transient(reason),
    }
}

pub(crate) async fn discover_hosted_mcp_package(
    package: &ExtensionPackage,
    max_tools: u32,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
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
            max_tools,
        )
        .await
        .map_err(classify_discovery_error)?;
    if output.tools.is_empty() {
        return Err(HostedMcpDiscoveryError::Transient(format!(
            "hosted MCP provider {} returned no discoverable tools",
            package.id
        )));
    }
    package_with_discovered_hosted_mcp_tools(package, &output.tools)
        .map_err(|error| HostedMcpDiscoveryError::Permanent(error.to_string()))
}

pub(crate) use ironclaw_extensions::is_hosted_http_mcp_package;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_auth_rejection_routes_to_reauth_not_transient() {
        // A 401/403 mid-discovery surfaces as `AuthRequired`; folding it into
        // `Transient` is the bug (retry forever, never re-OAuth). It must map
        // to the re-auth outcome instead.
        assert_eq!(
            classify_discovery_error(McpClientError::AuthRequired),
            HostedMcpDiscoveryError::ReAuthRequired,
        );
    }

    #[test]
    fn transport_and_catalog_failures_keep_their_lifecycle_class() {
        // Genuinely transient faults (timeouts, 5xx) stay retryable...
        assert_eq!(
            classify_discovery_error(McpClientError::client("mcp_http_status_503")),
            HostedMcpDiscoveryError::Transient("mcp_http_status_503".to_string()),
        );
        // ...and an invalid catalog stays permanent (OAuth cannot repair it).
        assert_eq!(
            classify_discovery_error(McpClientError::invalid_tool_catalog(
                "mcp_invalid_tool_list: unsafe_input_schema"
            )),
            HostedMcpDiscoveryError::Permanent(
                "mcp_invalid_tool_list: unsafe_input_schema".to_string()
            ),
        );
    }
}
