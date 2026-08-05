use ironclaw_extension_contracts::hosted_mcp::HostedMcpDiscoveredTool;
use ironclaw_extension_contracts::runtime::ExtensionRuntime;
use ironclaw_host_api::{
    action::{NetworkScheme, NetworkTargetPattern},
    capability::{CapabilityDescriptor, EffectKind, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    ids::CapabilityId,
    runtime::RuntimeKind,
};

use crate::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage, ManifestSource,
};

pub fn is_hosted_http_mcp_package(package: &ExtensionPackage) -> bool {
    hosted_http_mcp_url(package).is_some()
}

pub fn package_with_discovered_hosted_mcp_tools(
    package: &ExtensionPackage,
    tools: &[HostedMcpDiscoveredTool],
) -> Result<ExtensionPackage, ExtensionError> {
    if hosted_http_mcp_url(package).is_none() {
        return Err(invalid_hosted_mcp_manifest(format!(
            "extension {} is not a host-bundled hosted MCP provider",
            package.id
        )));
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
            network_targets: capability.network_targets.clone(),
            max_egress_bytes: capability.max_egress_bytes,
            resource_profile: capability.resource_profile.clone(),
            origin_gate_matrix: capability.origin_gate_matrix.clone(),
        })
        .collect();

    match package.manifest.source {
        ManifestSource::HostBundled => {
            ExtensionPackage::from_host_bundled_manifest_with_inline_dynamic_schemas(
                manifest,
                package
                    .materialized_root()
                    .map_err(|error| invalid_hosted_mcp_manifest(error.to_string()))?
                    .clone(),
                package.manifest_digest(),
                capabilities,
            )
        }
        ManifestSource::UserRegistered => ExtensionPackage::from_virtual_manifest(
            manifest,
            package.manifest_digest(),
            capabilities,
        ),
        _ => Err(invalid_hosted_mcp_manifest(
            "hosted MCP discovery requires bundled or user-registered provenance".to_string(),
        )),
    }
}

fn hosted_http_mcp_url(package: &ExtensionPackage) -> Option<&str> {
    if !matches!(
        package.manifest.source,
        ManifestSource::HostBundled | ManifestSource::UserRegistered
    ) {
        return None;
    }
    let ExtensionRuntime::Mcp {
        transport,
        command: None,
        args,
        url: Some(url),
    } = &package.manifest.runtime
    else {
        return None;
    };
    if transport != "http" || !args.is_empty() || !valid_hosted_mcp_url(url) {
        return None;
    }
    Some(url.as_str())
}

fn valid_hosted_mcp_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.host_str().is_some()
        && parsed.fragment().is_none()
}

fn hosted_mcp_capability_template(
    package: &ExtensionPackage,
) -> Result<HostedMcpCapabilityTemplate, ExtensionError> {
    let network_target = hosted_mcp_network_target(package).ok_or_else(|| {
        invalid_hosted_mcp_manifest(format!(
            "hosted MCP provider {} has no valid network target",
            package.id
        ))
    })?;
    let first = package.manifest.capabilities.first().ok_or_else(|| {
        invalid_hosted_mcp_manifest(format!(
            "hosted MCP provider {} has no capability template",
            package.id
        ))
    })?;
    for capability in &package.manifest.capabilities[1..] {
        if capability.required_host_ports != first.required_host_ports
            || capability.runtime_credentials != first.runtime_credentials
            || capability.resource_profile != first.resource_profile
        {
            return Err(invalid_hosted_mcp_manifest(format!(
                "hosted MCP provider {} has inconsistent capability templates",
                package.id
            )));
        }
    }
    Ok(HostedMcpCapabilityTemplate {
        provider_declares_external_write: package
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability.effects.contains(&EffectKind::ExternalWrite)),
        required_host_ports: first.required_host_ports.clone(),
        runtime_credentials: first.runtime_credentials.clone(),
        network_target,
        resource_profile: first.resource_profile.clone(),
        origin_gate_matrix: first.origin_gate_matrix.clone(),
    })
}

struct HostedMcpCapabilityTemplate {
    provider_declares_external_write: bool,
    required_host_ports: Vec<ironclaw_host_api::host_port::HostPortId>,
    runtime_credentials: Vec<ironclaw_host_api::capability::RuntimeCredentialRequirement>,
    network_target: NetworkTargetPattern,
    resource_profile: Option<ironclaw_host_api::resource::ResourceProfile>,
    origin_gate_matrix: Option<ironclaw_host_api::capability::OriginGateMatrix>,
}

fn discovered_capability_manifest(
    package: &ExtensionPackage,
    template: &HostedMcpCapabilityTemplate,
    tool: &HostedMcpDiscoveredTool,
) -> Result<CapabilityManifest, ExtensionError> {
    let capability_id =
        CapabilityId::new(format!("{}.{}", package.id.as_str(), tool.name)).map_err(|error| {
            invalid_hosted_mcp_manifest(format!(
                "discovered MCP tool {} from {} cannot be published as a Reborn capability: {error}",
                tool.name, package.id
            ))
        })?;
    let schema_path = tool.name.replace('.', "/");
    let input_schema_ref = CapabilityProfileSchemaRef::new(format!(
        "schemas/{}/dynamic/{schema_path}.input.v1.json",
        package.id.as_str()
    ))
    .map_err(|error| {
        invalid_hosted_mcp_manifest(format!("invalid discovered MCP input schema ref: {error}"))
    })?;
    let output_schema_ref = CapabilityProfileSchemaRef::new(format!(
        "schemas/{}/dynamic/{schema_path}.output.v1.json",
        package.id.as_str()
    ))
    .map_err(|error| {
        invalid_hosted_mcp_manifest(format!("invalid discovered MCP output schema ref: {error}"))
    })?;
    let mut effects = vec![EffectKind::DispatchCapability, EffectKind::Network];
    if !template.runtime_credentials.is_empty() {
        effects.push(EffectKind::UseSecret);
    }
    if discovered_tool_requires_external_write(template, tool) {
        effects.push(EffectKind::ExternalWrite);
    }

    Ok(CapabilityManifest {
        id: capability_id,
        description: if tool.description.trim().is_empty() {
            format!("Invoke hosted MCP tool {}", tool.name)
        } else {
            tool.description.clone()
        },
        effects,
        default_permission: PermissionMode::Ask,
        visibility: CapabilityVisibility::Model,
        input_schema_ref,
        output_schema_ref: Some(output_schema_ref),
        prompt_doc_ref: None,
        required_host_ports: template.required_host_ports.clone(),
        runtime_credentials: template.runtime_credentials.clone(),
        // Credential-free providers have no credential audience from which to
        // derive egress, so every discovered tool retains the registered MCP
        // endpoint as its explicit allowlist target.
        network_targets: vec![template.network_target.clone()],
        // MCP discovered tools take no manifest egress cap; their egress is
        // bounded by credential audiences and the runtime resource profile.
        max_egress_bytes: None,
        resource_profile: template.resource_profile.clone(),
        origin_gate_matrix: template.origin_gate_matrix.clone(),
    })
}

fn hosted_mcp_network_target(package: &ExtensionPackage) -> Option<NetworkTargetPattern> {
    let endpoint = url::Url::parse(hosted_http_mcp_url(package)?).ok()?;
    Some(NetworkTargetPattern {
        scheme: Some(NetworkScheme::Https),
        host_pattern: endpoint.host_str()?.to_ascii_lowercase(),
        port: endpoint.port(),
    })
}

fn discovered_tool_requires_external_write(
    template: &HostedMcpCapabilityTemplate,
    tool: &HostedMcpDiscoveredTool,
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

fn invalid_hosted_mcp_manifest(reason: String) -> ExtensionError {
    ExtensionError::InvalidManifest { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtensionManifest, HostPortCatalog, ManifestSource};
    use ironclaw_extension_contracts::hosted_mcp::HostedMcpDiscoveredToolAnnotations;
    use ironclaw_host_api::{capability::EffectKind, path::VirtualPath, runtime::RuntimeKind};

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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
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
            HostedMcpDiscoveredTool {
                name: "notion-search".to_string(),
                description: "Search live Notion tools".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]
                }),
                annotations: HostedMcpDiscoveredToolAnnotations {
                    read_only_hint: true,
                    ..Default::default()
                },
            },
            HostedMcpDiscoveredTool {
                name: "notion-create-pages".to_string(),
                description: "Create pages from live schema".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"pages": {"type": "array"}}
                }),
                annotations: HostedMcpDiscoveredToolAnnotations {
                    side_effects_hint: true,
                    ..Default::default()
                },
            },
        ];

        let discovered = package_with_discovered_hosted_mcp_tools(&package, &tools)
            .expect("build discovered package");

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

    /// Regression: credential-free hosted MCP tools still need the registered
    /// server in their egress allowlist. Without it, authorization emits an
    /// empty network-policy obligation and dispatch fails before any request.
    #[test]
    fn discovered_public_mcp_tool_keeps_endpoint_as_network_target() {
        let mut package = notion_package();
        package.manifest.capabilities[0].runtime_credentials.clear();
        package.capabilities[0].runtime_credentials.clear();
        let tools = vec![HostedMcpDiscoveredTool {
            name: "read_wiki".to_string(),
            description: "Read a repository wiki".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: HostedMcpDiscoveredToolAnnotations {
                read_only_hint: true,
                ..Default::default()
            },
        }];

        let discovered = package_with_discovered_hosted_mcp_tools(&package, &tools)
            .expect("build discovered public MCP package");
        let capability = discovered
            .capabilities
            .first()
            .expect("discovered capability");

        assert_eq!(
            capability.network_targets,
            vec![ironclaw_host_api::action::NetworkTargetPattern {
                scheme: Some(ironclaw_host_api::action::NetworkScheme::Https),
                host_pattern: "mcp.notion.com".to_string(),
                port: None,
            }]
        );
    }

    #[test]
    fn discovered_mcp_tools_reject_inconsistent_provider_templates() {
        let mut package = notion_package();
        let mut inconsistent = package.manifest.capabilities[0].clone();
        inconsistent.id = CapabilityId::new("notion.other").expect("valid capability id");
        inconsistent.runtime_credentials.clear();
        package.manifest.capabilities.push(inconsistent);
        let tools = vec![HostedMcpDiscoveredTool {
            name: "notion-search".to_string(),
            description: "Search live Notion tools".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Default::default(),
        }];

        let error = package_with_discovered_hosted_mcp_tools(&package, &tools)
            .expect_err("inconsistent templates must fail discovery");

        assert!(
            error
                .to_string()
                .contains("inconsistent capability templates"),
            "unexpected error: {error}"
        );
    }

    fn capability_provider_contracts() -> crate::HostApiContractRegistry {
        let mut contracts = crate::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                crate::CapabilityProviderHostApiContract::new().expect("contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    fn notion_package() -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            NOTION_MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::default(),
            &capability_provider_contracts(),
        )
        .expect("valid Notion manifest");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/notion").expect("valid root"),
        )
        .expect("valid Notion package")
    }
}
