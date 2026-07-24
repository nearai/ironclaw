use ironclaw_extensions::{CapabilityVisibility, ExtensionRegistry};
use ironclaw_host_api::{ExtensionId, InstallationState};

/// Owner-side failure to project the runtime capability contract for an
/// extension the lifecycle has already declared active.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityProjectionError {
    #[error("active extension `{extension_id}` has no published runtime contract")]
    MissingActiveRuntimeContract { extension_id: String },
}

/// Project the caller-visible capability ids for one lifecycle checkpoint.
///
/// Before activation, the catalog contract is the setup ceiling and remains
/// visible so the product can explain what will become available. Once active,
/// the published registry is authoritative: hosted MCP discovery may replace
/// the catalog template with a different live tool set. An active checkpoint
/// without a published package is an inconsistent host state and fails closed
/// instead of advertising capabilities that cannot execute.
pub fn project_capability_ids(
    active_registry: &ExtensionRegistry,
    extension_id: &ExtensionId,
    phase: InstallationState,
    catalog_visible_capability_ids: &[String],
    catalog_visible_read_only_capability_ids: &[String],
) -> Result<(Vec<String>, Vec<String>), CapabilityProjectionError> {
    if phase != InstallationState::Active {
        return Ok((
            catalog_visible_capability_ids.to_vec(),
            catalog_visible_read_only_capability_ids.to_vec(),
        ));
    }

    let package = active_registry.get_extension(extension_id).ok_or_else(|| {
        CapabilityProjectionError::MissingActiveRuntimeContract {
            extension_id: extension_id.as_str().to_string(),
        }
    })?;
    let visible = package
        .capabilities
        .iter()
        .filter(|descriptor| {
            active_registry.capability_visibility(&descriptor.id)
                == Some(CapabilityVisibility::Model)
        })
        .collect::<Vec<_>>();
    let visible_capability_ids = visible
        .iter()
        .map(|descriptor| descriptor.id.as_str().to_string())
        .collect();
    let visible_read_only_capability_ids = visible
        .into_iter()
        .filter(|descriptor| !descriptor.effects.iter().any(|effect| effect.is_write()))
        .map(|descriptor| descriptor.id.as_str().to_string())
        .collect();

    Ok((visible_capability_ids, visible_read_only_capability_ids))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_extensions::{
        CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage, ExtensionRegistry,
        HostApiContractRegistry, ManifestSource,
    };
    use ironclaw_host_api::{ExtensionId, HostPortCatalog, InstallationState, VirtualPath};

    use super::project_capability_ids;

    fn runtime_registry() -> ExtensionRegistry {
        let manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "hosted-search"
name = "Hosted Search"
version = "1.0.0"
description = "discovered runtime contract"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://search.example.test/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "hosted-search.discovered"
description = "Discovered search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                CapabilityProviderHostApiContract::new().expect("capability contract"),
            ))
            .expect("register capability contract");
        let manifest = ExtensionManifest::parse(
            manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("manifest");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/hosted-search").expect("root"),
        )
        .expect("package");
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).expect("register package");
        registry
    }

    #[test]
    fn setup_needed_projects_catalog_capabilities_before_runtime_publication() {
        let registry = ExtensionRegistry::new();
        let catalog_visible = vec!["hosted-search.template".to_string()];
        let catalog_read_only = catalog_visible.clone();

        let projected = project_capability_ids(
            &registry,
            &ExtensionId::new("hosted-search").expect("extension id"),
            InstallationState::Installed,
            &catalog_visible,
            &catalog_read_only,
        )
        .expect("setup-needed projection");

        assert_eq!(projected, (catalog_visible, catalog_read_only));
    }

    #[test]
    fn active_projects_discovered_runtime_capabilities() {
        let registry = runtime_registry();

        let projected = project_capability_ids(
            &registry,
            &ExtensionId::new("hosted-search").expect("extension id"),
            InstallationState::Active,
            &["hosted-search.template".to_string()],
            &["hosted-search.template".to_string()],
        )
        .expect("active projection");

        assert_eq!(
            projected,
            (
                vec!["hosted-search.discovered".to_string()],
                vec!["hosted-search.discovered".to_string()],
            )
        );
    }

    #[test]
    fn active_without_a_published_runtime_contract_fails_closed() {
        let error = project_capability_ids(
            &ExtensionRegistry::new(),
            &ExtensionId::new("hosted-search").expect("extension id"),
            InstallationState::Active,
            &["hosted-search.template".to_string()],
            &["hosted-search.template".to_string()],
        )
        .expect_err("active projection requires a published runtime contract");

        assert_eq!(
            error.to_string(),
            "active extension `hosted-search` has no published runtime contract"
        );
    }
}
