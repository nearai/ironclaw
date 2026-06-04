use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionPackage, ExtensionRegistry, ExtensionRuntime, HostedMcpDiscoveredTool,
    HostedMcpDiscoveredToolAnnotations, SharedExtensionRegistry,
    package_with_discovered_hosted_mcp_tools,
};
use ironclaw_host_api::{ResourceScope, RuntimeHttpEgress};
use ironclaw_mcp::{
    McpClient, McpClientRequest, McpDiscoveredTool, McpHostHttpClient, McpRuntimeHttpAdapter,
};

use crate::mcp::{MCP_RESPONSE_BODY_LIMIT, RegistryMcpEgressPlanner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostedMcpDiscoveryError {
    Transient(String),
    Permanent(String),
}

pub(crate) async fn discover_hosted_mcp_package(
    package: &ExtensionPackage,
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
        .discover_tools(McpClientRequest {
            provider: package.id.clone(),
            capability_id: planning_capability_id,
            scope,
            transport,
            command,
            args,
            url,
            input: serde_json::Value::Null,
            max_output_bytes: MCP_RESPONSE_BODY_LIMIT,
        })
        .await
        .map_err(|error| HostedMcpDiscoveryError::Transient(error.stable_reason().to_string()))?;
    if output.tools.is_empty() {
        return Err(HostedMcpDiscoveryError::Transient(format!(
            "hosted MCP provider {} returned no discoverable tools",
            package.id
        )));
    }
    let tools = output
        .tools
        .iter()
        .map(discovered_tool_for_extension_domain)
        .collect::<Vec<_>>();
    package_with_discovered_hosted_mcp_tools(package, &tools)
        .map_err(|error| HostedMcpDiscoveryError::Permanent(error.to_string()))
}

pub(crate) use ironclaw_extensions::is_hosted_http_mcp_package;

fn discovered_tool_for_extension_domain(tool: &McpDiscoveredTool) -> HostedMcpDiscoveredTool {
    HostedMcpDiscoveredTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
        annotations: HostedMcpDiscoveredToolAnnotations {
            destructive_hint: tool.annotations.destructive_hint,
            side_effects_hint: tool.annotations.side_effects_hint,
            read_only_hint: tool.annotations.read_only_hint,
        },
    }
}
