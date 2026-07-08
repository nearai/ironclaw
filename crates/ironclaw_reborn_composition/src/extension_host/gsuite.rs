use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    CredentialAccount, CredentialAccountRecordSource, CredentialAccountSelectionRequest,
    CredentialAccountService, GOOGLE_PROVIDER_ID,
};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_first_party_extensions::{
    GsuiteCapabilitySpec, GsuiteCredentialStageError, GsuiteCredentialStageRequest,
    GsuiteCredentialStager, GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor,
    GsuitePackageSpec, find_gsuite_capability, gsuite_google_account_visible_to_requester,
    gsuite_package_specs, gsuite_resource_profile,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, ExtensionId, HostApiError, NetworkScheme,
    NetworkTargetPattern, RequestedTrustClass, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, RuntimeCredentialRequirement,
    RuntimeCredentialRequirementSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind,
    SecretHandle, TrustClass, VirtualPath,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ProductAuthProviderRuntimePorts,
};

use crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy;

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

pub(crate) struct GsuiteRuntimeCredentialAccountVisibilityPolicy;

impl RuntimeCredentialAccountVisibilityPolicy for GsuiteRuntimeCredentialAccountVisibilityPolicy {
    fn account_visible_to_requester(
        &self,
        account: &CredentialAccount,
        lookup: &CredentialAccountSelectionRequest,
    ) -> bool {
        let requester = lookup.requester_extension.as_ref();
        if lookup.provider.as_str() != GOOGLE_PROVIDER_ID {
            return account.is_authorized_for_requester(requester);
        }
        let Some(requester) = requester else {
            return account.is_authorized_for_requester(None);
        };
        gsuite_google_account_visible_to_requester(account, requester)
    }
}

/// Build GSuite handlers for a surface that can install and activate GSuite packages.
///
/// Handler registration is allowed before lifecycle activation because runtime
/// dispatch still requires active package descriptors.
pub fn bundled_gsuite_first_party_handlers(
    accounts: Arc<dyn CredentialAccountService>,
    account_records: Arc<dyn CredentialAccountRecordSource>,
    credential_stager: Arc<dyn GsuiteCredentialStager>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = FirstPartyCapabilityRegistry::new();
    register_bundled_gsuite_first_party_handlers(
        &mut registry,
        accounts,
        account_records,
        credential_stager,
    )?;
    Ok(registry)
}

pub(crate) fn register_bundled_gsuite_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    accounts: Arc<dyn CredentialAccountService>,
    account_records: Arc<dyn CredentialAccountRecordSource>,
    credential_stager: Arc<dyn GsuiteCredentialStager>,
) -> Result<(), HostApiError> {
    let handler = Arc::new(GsuiteFirstPartyHandler {
        executor: GsuiteExecutor::new(accounts, account_records, credential_stager),
    });
    for package in gsuite_package_specs() {
        for capability in package.capabilities {
            registry.insert_handler(CapabilityId::new(capability.id)?, Arc::clone(&handler));
        }
    }
    Ok(())
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
            .map_err(|error| gsuite_error(error, &request.capability_id))?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn package_from_spec(spec: &GsuitePackageSpec) -> Result<ExtensionPackage, ExtensionError> {
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| capability_manifest(capability, spec))
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
            hooks: Vec::new(),
            capabilities,
        },
        VirtualPath::new(format!("/system/extensions/{}", spec.extension_id))?,
    )
}

fn capability_manifest(
    capability: &GsuiteCapabilitySpec,
    spec: &GsuitePackageSpec,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(capability.id)?,
        implements: Vec::new(),
        description: capability.description.to_string(),
        effects: capability.effects.to_vec(),
        default_permission: capability.default_permission,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.input.v1.json",
            spec.schema_prefix, capability.short_name
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.output.v1.json",
            spec.schema_prefix, capability.short_name
        ))?,
        prompt_doc_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "prompts/{}/{}.md",
            spec.schema_prefix, capability.short_name
        ))?),
        required_host_ports: Vec::new(),
        runtime_credentials: runtime_credentials(capability, spec)?,
        resource_profile: Some(gsuite_resource_profile()),
    })
}

fn runtime_credentials(
    capability: &GsuiteCapabilitySpec,
    spec: &GsuitePackageSpec,
) -> Result<Vec<RuntimeCredentialRequirement>, ExtensionError> {
    let provider_scopes = required_provider_scopes(capability);
    Ok(vec![RuntimeCredentialRequirement {
        handle: SecretHandle::new(spec.credential_handle)?,
        source: RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: RuntimeCredentialAccountProviderId::new(ironclaw_auth::GOOGLE_PROVIDER_ID)?,
            setup: RuntimeCredentialAccountSetup::OAuth {
                scopes: provider_scopes.clone(),
            },
        },
        provider_scopes,
        audience: NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: spec.credential_host_pattern.to_string(),
            port: None,
        },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    }])
}

fn required_provider_scopes(capability: &GsuiteCapabilitySpec) -> Vec<String> {
    capability
        .required_scopes
        .iter()
        .map(|scope| (*scope).to_string())
        .collect()
}

/// Convert a [`GsuiteDispatchError`] into the neutral [`FirstPartyCapabilityError`].
fn gsuite_error(
    error: GsuiteDispatchError,
    capability_id: &CapabilityId,
) -> FirstPartyCapabilityError {
    let usage = error.usage().cloned();
    let base = match error.auth_requirement() {
        Some(required_secrets) => match gsuite_credential_requirements(capability_id) {
            Ok(credential_requirements) => FirstPartyCapabilityError::auth_required_with_context(
                required_secrets,
                credential_requirements,
            ),
            Err(error) => error,
        },
        None => FirstPartyCapabilityError::new(error.kind()),
    };
    match usage {
        Some(u) => base.with_usage(u),
        None => base,
    }
}

fn gsuite_credential_requirements(
    capability_id: &CapabilityId,
) -> Result<Vec<RuntimeCredentialAuthRequirement>, FirstPartyCapabilityError> {
    let (package, capability) =
        find_gsuite_capability(capability_id.as_str()).ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::UndeclaredCapability)
        })?;
    let requester_extension = ExtensionId::new(package.extension_id)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend))?;
    let requirements = runtime_credentials(capability, package)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend))?
        .into_iter()
        .filter(|credential| credential.required)
        .filter_map(|credential| {
            credential.product_auth_requirement_for(requester_extension.clone())
        })
        .collect::<Vec<_>>();
    if requirements.is_empty() {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Backend,
        ));
    }
    Ok(requirements)
}

pub(crate) struct ProductAuthRuntimeGsuiteCredentialStager {
    runtime_ports: ProductAuthProviderRuntimePorts,
}

impl ProductAuthRuntimeGsuiteCredentialStager {
    pub(crate) fn new(runtime_ports: ProductAuthProviderRuntimePorts) -> Self {
        Self { runtime_ports }
    }
}

#[async_trait]
impl GsuiteCredentialStager for ProductAuthRuntimeGsuiteCredentialStager {
    async fn stage(
        &self,
        request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError> {
        // Both GsuiteCredentialStageError and ProductAuthCredentialStageError are
        // type aliases for ironclaw_host_api::CredentialStageError — no conversion needed.
        self.runtime_ports
            .stage_secret_from_scope_once(
                request.source_scope,
                request.target_scope,
                request.capability_id,
                request.access_secret,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_first_party_extensions::{
        GMAIL_LIST_MESSAGES_CAPABILITY_ID, GsuiteCredentialDispatchReason,
    };
    use ironclaw_host_api::RuntimeDispatchErrorKind;

    use super::*;

    #[test]
    fn gmail_auth_failure_maps_to_google_oauth_requirement() {
        let capability_id = CapabilityId::new(GMAIL_LIST_MESSAGES_CAPABILITY_ID).unwrap();
        let error = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client)
            .with_reason(GsuiteCredentialDispatchReason::MissingAccessSecret);

        let mapped = gsuite_error(error, &capability_id);

        let FirstPartyCapabilityError::AuthRequired {
            required_secrets,
            credential_requirements,
            ..
        } = mapped
        else {
            panic!("expected Gmail auth failure to map to FirstParty AuthRequired");
        };
        assert!(required_secrets.is_empty());
        assert_eq!(credential_requirements.len(), 1);
        let requirement = &credential_requirements[0];
        assert_eq!(
            requirement.provider.as_str(),
            ironclaw_auth::GOOGLE_PROVIDER_ID
        );
        assert_eq!(requirement.requester_extension.as_str(), "gmail");
        assert_eq!(
            requirement.provider_scopes,
            vec![ironclaw_auth::GOOGLE_GMAIL_READONLY_SCOPE.to_string()]
        );
        assert_eq!(
            requirement.setup,
            RuntimeCredentialAccountSetup::OAuth {
                scopes: vec![ironclaw_auth::GOOGLE_GMAIL_READONLY_SCOPE.to_string()]
            }
        );
    }
}
