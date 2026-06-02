use std::sync::Arc;

use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionPackage, ExtensionRegistry,
    ExtensionRuntime, SharedExtensionRegistry,
};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilityProfileSchemaRef, EffectKind, PermissionMode,
    ResourceScope, RuntimeHttpEgress, RuntimeKind,
};
use ironclaw_mcp::{
    McpClient, McpClientRequest, McpDiscoveredTool, McpHostHttpClient, McpRuntimeHttpAdapter,
};

use crate::mcp::{MCP_RESPONSE_BODY_LIMIT, RegistryMcpEgressPlanner, hosted_http_mcp_endpoint};

pub(crate) fn is_hosted_http_mcp_package(package: &ExtensionPackage) -> bool {
    hosted_http_mcp_endpoint(package).is_some()
}

pub(crate) async fn discover_hosted_mcp_package(
    package: &ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ExtensionPackage, String> {
    let (transport, command, args, url) = match &package.manifest.runtime {
        ExtensionRuntime::Mcp {
            transport,
            command,
            args,
            url,
        } if hosted_http_mcp_endpoint(package).is_some() => (
            transport.clone(),
            command.clone(),
            args.clone(),
            url.clone(),
        ),
        _ => {
            return Err(format!(
                "extension {} is not a host-bundled hosted MCP provider",
                package.id
            ));
        }
    };
    let template = hosted_mcp_capability_template(package)?;
    let registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    registry
        .upsert(package.clone())
        .map_err(|error| format!("failed to prepare hosted MCP discovery: {error}"))?;
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(runtime_http_egress),
        RegistryMcpEgressPlanner::new(registry),
    );
    let output = client
        .discover_tools(McpClientRequest {
            provider: package.id.clone(),
            capability_id: template.planning_capability_id,
            scope,
            transport,
            command,
            args,
            url,
            input: serde_json::Value::Null,
            max_output_bytes: MCP_RESPONSE_BODY_LIMIT,
        })
        .await?;
    if output.tools.is_empty() {
        return Err(format!(
            "hosted MCP provider {} returned no discoverable tools",
            package.id
        ));
    }
    package_with_discovered_tools(package, &output.tools)
}

pub(crate) fn package_with_discovered_tools(
    package: &ExtensionPackage,
    tools: &[McpDiscoveredTool],
) -> Result<ExtensionPackage, String> {
    if hosted_http_mcp_endpoint(package).is_none() {
        return Err(format!(
            "extension {} is not a host-bundled hosted MCP provider",
            package.id
        ));
    }
    let template = hosted_mcp_capability_template(package)?;

    let mut manifest = package.manifest.clone();
    manifest.capabilities = tools
        .iter()
        .map(|tool| discovered_capability_manifest(package, &template, tool))
        .collect::<Result<Vec<_>, _>>()?;

    let capabilities = manifest
        .capabilities
        .iter()
        .zip(tools)
        .map(|(capability, tool)| CapabilityDescriptor {
            id: capability.id.clone(),
            provider: package.id.clone(),
            runtime: RuntimeKind::Mcp,
            trust_ceiling: manifest.descriptor_trust_default,
            description: capability.description.clone(),
            parameters_schema: tool.input_schema.clone(),
            effects: capability.effects.clone(),
            default_permission: capability.default_permission,
            runtime_credentials: capability.runtime_credentials.clone(),
            resource_profile: capability.resource_profile.clone(),
        })
        .collect();

    ExtensionPackage::from_host_bundled_manifest_with_inline_dynamic_schemas(
        manifest,
        package.root.clone(),
        package.manifest_digest(),
        capabilities,
    )
    .map_err(|error| error.to_string())
}

fn hosted_mcp_capability_template(
    package: &ExtensionPackage,
) -> Result<HostedMcpCapabilityTemplate, String> {
    let first = package.manifest.capabilities.first().ok_or_else(|| {
        format!(
            "hosted MCP provider {} has no capability template",
            package.id
        )
    })?;
    for capability in &package.manifest.capabilities[1..] {
        if capability.required_host_ports != first.required_host_ports
            || capability.runtime_credentials != first.runtime_credentials
            || capability.resource_profile != first.resource_profile
        {
            return Err(format!(
                "hosted MCP provider {} has inconsistent capability templates",
                package.id
            ));
        }
    }
    Ok(HostedMcpCapabilityTemplate {
        planning_capability_id: first.id.clone(),
        provider_declares_external_write: package
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability.effects.contains(&EffectKind::ExternalWrite)),
        required_host_ports: first.required_host_ports.clone(),
        runtime_credentials: first.runtime_credentials.clone(),
        resource_profile: first.resource_profile.clone(),
    })
}

struct HostedMcpCapabilityTemplate {
    planning_capability_id: CapabilityId,
    provider_declares_external_write: bool,
    required_host_ports: Vec<ironclaw_host_api::HostPortId>,
    runtime_credentials: Vec<ironclaw_host_api::RuntimeCredentialRequirement>,
    resource_profile: Option<ironclaw_host_api::ResourceProfile>,
}

fn discovered_capability_manifest(
    package: &ExtensionPackage,
    template: &HostedMcpCapabilityTemplate,
    tool: &McpDiscoveredTool,
) -> Result<CapabilityManifest, String> {
    let capability_id = CapabilityId::new(format!("{}.{}", package.id.as_str(), tool.name))
        .map_err(|error| {
            format!(
                "discovered MCP tool {} from {} cannot be published as a Reborn capability: {error}",
                tool.name, package.id
            )
        })?;
    let schema_path = tool.name.replace('.', "/");
    let input_schema_ref = CapabilityProfileSchemaRef::new(format!(
        "schemas/{}/dynamic/{schema_path}.input.v1.json",
        package.id.as_str()
    ))
    .map_err(|error| format!("invalid discovered MCP input schema ref: {error}"))?;
    let output_schema_ref = CapabilityProfileSchemaRef::new(format!(
        "schemas/{}/dynamic/{schema_path}.output.v1.json",
        package.id.as_str()
    ))
    .map_err(|error| format!("invalid discovered MCP output schema ref: {error}"))?;
    let mut effects = vec![EffectKind::DispatchCapability, EffectKind::Network];
    if !template.runtime_credentials.is_empty() {
        effects.push(EffectKind::UseSecret);
    }
    if discovered_tool_requires_external_write(template, tool) {
        effects.push(EffectKind::ExternalWrite);
    }

    Ok(CapabilityManifest {
        id: capability_id,
        implements: Vec::new(),
        description: if tool.description.trim().is_empty() {
            format!("Invoke hosted MCP tool {}", tool.name)
        } else {
            tool.description.clone()
        },
        effects,
        default_permission: PermissionMode::Ask,
        visibility: CapabilityVisibility::Model,
        input_schema_ref,
        output_schema_ref,
        prompt_doc_ref: None,
        required_host_ports: template.required_host_ports.clone(),
        runtime_credentials: template.runtime_credentials.clone(),
        resource_profile: template.resource_profile.clone(),
    })
}

fn discovered_tool_requires_external_write(
    template: &HostedMcpCapabilityTemplate,
    tool: &McpDiscoveredTool,
) -> bool {
    if tool.annotations.destructive_hint || tool.annotations.side_effects_hint {
        return true;
    }
    if tool.annotations.read_only_hint {
        return false;
    }
    // MCP annotations are advisory. For providers whose bundled manifest
    // declares write-capable tools, unannotated discovered tools stay
    // conservative so policy surfaces do not understate possible side effects.
    template.provider_declares_external_write
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_host_api::HostPortCatalog;

    const NOTION_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "notion"
name = "Notion"
version = "1.0.0"
description = "Hosted Notion MCP"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.notion.com/mcp"

[[capabilities]]
id = "notion.notion-fetch"
description = "Fetch a Notion page"
effects = ["dispatch_capability", "network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/notion/fetch.input.json"
output_schema_ref = "schemas/notion/fetch.output.json"
runtime_credentials = [
  { handle = "notion_access_token", source = { type = "product_auth_account", provider = "notion" }, audience = { scheme = "https", host_pattern = "mcp.notion.com" }, target = { type = "header", name = "authorization", prefix = "Bearer " }, required = true }
]
"#;

    #[test]
    fn discovered_mcp_tools_replace_provider_capabilities_with_inline_schemas() {
        let package = notion_package();
        let tools = vec![
            McpDiscoveredTool {
                name: "notion-search".to_string(),
                description: "Search live Notion tools".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]
                }),
                annotations: ironclaw_mcp::McpDiscoveredToolAnnotations {
                    read_only_hint: true,
                    ..Default::default()
                },
            },
            McpDiscoveredTool {
                name: "notion-create-pages".to_string(),
                description: "Create pages from live schema".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"pages": {"type": "array"}}
                }),
                annotations: ironclaw_mcp::McpDiscoveredToolAnnotations {
                    side_effects_hint: true,
                    ..Default::default()
                },
            },
        ];

        let discovered =
            package_with_discovered_tools(&package, &tools).expect("build discovered package");

        assert!(
            discovered
                .capabilities
                .iter()
                .all(|capability| capability.id.as_str() != "notion.notion-fetch")
        );
        let search = discovered
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "notion.notion-search")
            .expect("discovered search capability");
        assert_eq!(search.description, "Search live Notion tools");
        assert_eq!(search.runtime, RuntimeKind::Mcp);
        assert_eq!(
            search.parameters_schema,
            serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        );
        assert_eq!(search.runtime_credentials.len(), 1);
        assert!(search.effects.contains(&EffectKind::UseSecret));
        assert!(!search.effects.contains(&EffectKind::ExternalWrite));
        let create_pages = discovered
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "notion.notion-create-pages")
            .expect("discovered create-pages capability");
        assert!(create_pages.effects.contains(&EffectKind::ExternalWrite));
        assert_eq!(discovered.manifest.capabilities.len(), 2);
    }

    #[test]
    fn discovered_mcp_tools_reject_inconsistent_provider_templates() {
        let mut package = notion_package();
        let mut inconsistent = package.manifest.capabilities[0].clone();
        inconsistent.id = CapabilityId::new("notion.other").expect("valid capability id");
        inconsistent.runtime_credentials.clear();
        package.manifest.capabilities.push(inconsistent);
        let tools = vec![McpDiscoveredTool {
            name: "notion-search".to_string(),
            description: "Search live Notion tools".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Default::default(),
        }];

        let error = package_with_discovered_tools(&package, &tools)
            .expect_err("inconsistent templates must fail discovery");

        assert!(
            error.contains("inconsistent capability templates"),
            "unexpected error: {error}"
        );
    }

    fn notion_package() -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            NOTION_MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::default(),
        )
        .expect("valid Notion manifest");
        ExtensionPackage::from_manifest(
            manifest,
            ironclaw_host_api::VirtualPath::new("/system/extensions/notion").expect("valid root"),
        )
        .expect("valid Notion package")
    }
}
