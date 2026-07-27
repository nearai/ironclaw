//! Test-support first-party handler registrars that mirror the concrete
//! executors the `ironclaw_reborn_cli` binary assembles in production.

/// Test-support first-party handler registrars (GSuite + web tooling) mirroring
/// the concrete executors the `ironclaw_reborn_cli` binary assembles in
/// production (`crates/ironclaw_reborn_cli/src/first_party/`).
///
/// Composition names `ironclaw_first_party_extensions` in production nowhere
/// (extension-runtime DEL-7); the binary supplies these registrars on the build
/// input. Tests re-derive the same wiring here through the test-support
/// feature so they can install/activate/dispatch the first-party extensions
/// through the production registrar seam without the binary.
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    CredentialAccount, CredentialAccountSelectionRequest, RuntimeCredentialAccountVisibilityPolicy,
};
use ironclaw_extension_host::{FirstPartyHandlerRegistrar, FirstPartyRegistrarContext};
use ironclaw_first_party_extensions::{
    FIRST_PARTY_WEB_GET_CONTENT_CAPABILITY_ID, FIRST_PARTY_WEB_SEARCH_CAPABILITY_ID,
    FirstPartyWebDispatchError, FirstPartyWebDispatchRequest, FirstPartyWebExecutor,
    GOOGLE_PROVIDER_ID, GsuiteCapabilitySpec, GsuiteCredentialDispatchReason,
    GsuiteCredentialStageError, GsuiteCredentialStageRequest, GsuiteCredentialStager,
    GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor, GsuitePackageSpec,
    find_gsuite_capability, gsuite_google_account_visible_to_requester, gsuite_package_specs,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, HostApiError, NetworkScheme, NetworkTargetPattern,
    RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, RuntimeCredentialRequirement,
    RuntimeCredentialRequirementSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind,
    SecretHandle, VendorId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ProductAuthProviderRuntimePorts,
};

/// The full set of first-party handler registrars a local-dev/test build
/// needs, mirroring `ironclaw_reborn_cli::first_party::bundled_first_party_registrars`.
pub fn bundled_first_party_registrars() -> Vec<Arc<dyn FirstPartyHandlerRegistrar>> {
    vec![
        Arc::new(GsuiteFirstPartyRegistrar),
        Arc::new(WebToolFirstPartyRegistrar),
    ]
}

/// The GSuite Google-account visibility policy, mirroring the binary's
/// `first_party_credential_account_visibility_policy()`.
pub fn bundled_credential_account_visibility_policy()
-> Arc<dyn RuntimeCredentialAccountVisibilityPolicy> {
    Arc::new(GsuiteRuntimeCredentialAccountVisibilityPolicy)
}

struct GsuiteFirstPartyRegistrar;

impl FirstPartyHandlerRegistrar for GsuiteFirstPartyRegistrar {
    fn register(
        &self,
        registry: &mut FirstPartyCapabilityRegistry,
        context: &FirstPartyRegistrarContext,
    ) -> Result<(), HostApiError> {
        let handler = Arc::new(GsuiteFirstPartyHandler {
            executor: GsuiteExecutor::new(
                context.credential_account_service.clone(),
                context.credential_account_record_source.clone(),
                Arc::new(ProductAuthRuntimeGsuiteCredentialStager::new(
                    context.product_auth_runtime_ports.clone(),
                )),
            ),
            google_oauth_configured: context.oauth_backend_configured,
        });
        for package in gsuite_package_specs() {
            for capability in package.capabilities {
                registry.insert_handler(CapabilityId::new(capability.id)?, Arc::clone(&handler));
            }
        }
        Ok(())
    }
}

struct GsuiteFirstPartyHandler {
    executor: GsuiteExecutor,
    google_oauth_configured: bool,
}

#[async_trait]
impl FirstPartyCapabilityHandler for GsuiteFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        if !self.google_oauth_configured {
            return Err(FirstPartyCapabilityError::dispatch_with_host_remediation(
                RuntimeDispatchErrorKind::OperationFailed,
                None,
                ironclaw_reborn_config::HostRemediationText::GoogleNotConfigured.text(),
            ));
        }
        let egress = request
            .services
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
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

fn runtime_credentials(
    capability: &GsuiteCapabilitySpec,
    spec: &GsuitePackageSpec,
) -> Result<Vec<RuntimeCredentialRequirement>, HostApiError> {
    let provider_scopes = capability
        .required_scopes
        .iter()
        .map(|scope| (*scope).to_string())
        .collect::<Vec<_>>();
    Ok(vec![RuntimeCredentialRequirement {
        handle: SecretHandle::new(spec.credential_handle)?,
        source: RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: VendorId::new(GOOGLE_PROVIDER_ID)?,
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
        None => match error.reason() {
            Some(GsuiteCredentialDispatchReason::BackendAuth) => {
                FirstPartyCapabilityError::dispatch_with_host_remediation(
                    error.kind(),
                    Some(
                        "Google OAuth is configured but the provider rejected the credentials"
                            .to_string(),
                    ),
                    ironclaw_reborn_config::HostRemediationText::GoogleBackendAuth.text(),
                )
            }
            _ => FirstPartyCapabilityError::new(error.kind()),
        },
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

struct ProductAuthRuntimeGsuiteCredentialStager {
    runtime_ports: ProductAuthProviderRuntimePorts,
}

impl ProductAuthRuntimeGsuiteCredentialStager {
    fn new(runtime_ports: ProductAuthProviderRuntimePorts) -> Self {
        Self { runtime_ports }
    }
}

#[async_trait]
impl GsuiteCredentialStager for ProductAuthRuntimeGsuiteCredentialStager {
    async fn stage(
        &self,
        request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError> {
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

struct GsuiteRuntimeCredentialAccountVisibilityPolicy;

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

struct WebToolFirstPartyRegistrar;

impl FirstPartyHandlerRegistrar for WebToolFirstPartyRegistrar {
    fn register(
        &self,
        registry: &mut FirstPartyCapabilityRegistry,
        _context: &FirstPartyRegistrarContext,
    ) -> Result<(), HostApiError> {
        let handler = Arc::new(WebToolFirstPartyHandler {
            executor: FirstPartyWebExecutor::default(),
        });
        registry.insert_handler(
            CapabilityId::new(FIRST_PARTY_WEB_SEARCH_CAPABILITY_ID)?,
            Arc::clone(&handler),
        );
        registry.insert_handler(
            CapabilityId::new(FIRST_PARTY_WEB_GET_CONTENT_CAPABILITY_ID)?,
            Arc::clone(&handler),
        );
        Ok(())
    }
}

struct WebToolFirstPartyHandler {
    executor: FirstPartyWebExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for WebToolFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(FirstPartyWebDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(web_tool_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn web_tool_error(error: FirstPartyWebDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}
