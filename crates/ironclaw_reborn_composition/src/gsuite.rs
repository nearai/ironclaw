use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::CredentialAccountService;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_first_party_extensions::gsuite::{
    GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor, GsuitePackageSpec,
    gsuite_package_specs, gsuite_resource_profile,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, ExtensionId, HostApiError, RequestedTrustClass,
    TrustClass, VirtualPath,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

/// Host-bundled GSuite packages available to an install/activation surface.
///
/// These packages are deliberately not inserted into the default built-in
/// first-party registry. A product or composition install surface must
/// explicitly register their package descriptors and first-party handlers before
/// their capabilities become model-visible.
pub fn bundled_gsuite_extension_packages() -> Result<Vec<ExtensionPackage>, ExtensionError> {
    gsuite_package_specs()
        .iter()
        .map(package_from_spec)
        .collect()
}

/// Build GSuite handlers for a GSuite surface that has already been activated.
pub fn bundled_gsuite_first_party_handlers(
    accounts: Arc<dyn CredentialAccountService>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let handler = Arc::new(GsuiteFirstPartyHandler {
        executor: GsuiteExecutor::new(accounts),
    });
    let mut registry = FirstPartyCapabilityRegistry::new();
    for package in gsuite_package_specs() {
        for capability in package.capabilities {
            registry.insert_handler(
                CapabilityId::new(format!(
                    "{}.{}",
                    package.extension_id, capability.short_name
                ))?,
                Arc::clone(&handler),
            );
        }
    }
    Ok(registry)
}

struct GsuiteFirstPartyHandler {
    executor: GsuiteExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for GsuiteFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let egress = request
            .services
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(
                    ironclaw_host_api::RuntimeDispatchErrorKind::NetworkDenied,
                )
            })?
            .clone();
        let result = self
            .executor
            .dispatch(GsuiteDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: egress,
            })
            .await
            .map_err(gsuite_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn package_from_spec(spec: &GsuitePackageSpec) -> Result<ExtensionPackage, ExtensionError> {
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| capability_manifest(spec, capability))
        .collect::<Result<Vec<_>, _>>()?;
    ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            id: ExtensionId::new(spec.extension_id)?,
            name: spec.name.to_string(),
            version: "0.1.0".to_string(),
            description: spec.description.to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ExtensionRuntime::FirstParty {
                service: spec.service.to_string(),
            },
            host_apis: Vec::new(),
            capabilities,
        },
        VirtualPath::new(format!("/system/extensions/{}", spec.extension_id))?,
    )
}

fn capability_manifest(
    package: &GsuitePackageSpec,
    capability: &ironclaw_first_party_extensions::gsuite::GsuiteCapabilitySpec,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(format!(
            "{}.{}",
            package.extension_id, capability.short_name
        ))?,
        implements: Vec::new(),
        description: capability.description.to_string(),
        effects: capability.effects.to_vec(),
        default_permission: capability.default_permission,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.input.v1.json",
            package.schema_prefix, capability.short_name
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.output.v1.json",
            package.schema_prefix, capability.short_name
        ))?,
        prompt_doc_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "prompts/{}/{}.md",
            package.schema_prefix, capability.short_name
        ))?),
        required_host_ports: Vec::new(),
        resource_profile: Some(gsuite_resource_profile()),
    })
}

fn gsuite_error(error: GsuiteDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}
