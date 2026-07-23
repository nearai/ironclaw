//! Binary-assembled GSuite first-party capability wiring (extension-runtime
//! DEL-7).
//!
//! Composition owns the generic `FirstPartyHandlerRegistrar` seam and the
//! shared context; the concrete GSuite executor, credential stager, error
//! mapping, and Google-account visibility policy live here in the assembling
//! binary. Every host-api / host-runtime / auth type is reached through the
//! `ironclaw_reborn_composition` facade re-exports so the CLI's exact-deps
//! allow-list stays frozen to the facade set plus `ironclaw_first_party_extensions`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_first_party_extensions::{
    GOOGLE_PROVIDER_ID, GsuiteCapabilitySpec, GsuiteCredentialDispatchReason,
    GsuiteCredentialStageError, GsuiteCredentialStageRequest, GsuiteCredentialStager,
    GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor, GsuitePackageSpec,
    find_gsuite_capability, gsuite_google_account_visible_to_requester, gsuite_package_specs,
};
use ironclaw_reborn_composition::{
    CapabilityId, CredentialAccount, CredentialAccountSelectionRequest, ExtensionId,
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, FirstPartyHandlerRegistrar,
    FirstPartyRegistrarContext, HostApiError, NetworkScheme, NetworkTargetPattern,
    ProductAuthProviderRuntimePorts, RuntimeCredentialAccountSetup,
    RuntimeCredentialAccountVisibilityPolicy, RuntimeCredentialAuthRequirement,
    RuntimeCredentialRequirement, RuntimeCredentialRequirementSource, RuntimeCredentialTarget,
    RuntimeDispatchErrorKind, SecretHandle, VendorId,
};

/// Installs the GSuite first-party capability handlers into the shared registry.
pub(crate) struct GsuiteFirstPartyRegistrar;

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
            google_oauth_configured: context.google_oauth_configured,
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
        // Pre-dispatch check: every GSuite capability requires a Google OAuth
        // account, so a missing build-time OAuth backend means no dispatch can
        // ever succeed. Short-circuit with a remediation tool result instead of
        // a silent auth-gate stall.
        if !self.google_oauth_configured {
            return Err(google_oauth_not_configured_error());
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

/// Tool-result error for a GSuite capability dispatched with no Google OAuth
/// backend configured at all — distinct from `AuthRequired`. Rides the trusted
/// HOST-REMEDIATION channel because the text names `config set` keys containing
/// "secret" and console URLs that `safe_summary` / the untrusted diagnostic
/// channel would reject or collapse.
fn google_oauth_not_configured_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::dispatch_with_host_remediation(
        RuntimeDispatchErrorKind::OperationFailed,
        None,
        ironclaw_reborn_config::HostRemediationText::GoogleNotConfigured.text(),
    )
}

fn runtime_credentials(
    capability: &GsuiteCapabilitySpec,
    spec: &GsuitePackageSpec,
) -> Result<Vec<RuntimeCredentialRequirement>, HostApiError> {
    let provider_scopes = required_provider_scopes(capability);
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
        None => match error.reason() {
            // `BackendAuth` means the account resolved but the provider rejected
            // the request while exchanging/refreshing the token — configured,
            // but rejected. Distinct from `AuthRequired` and the not-configured
            // pre-dispatch check.
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

/// The binary-supplied credential stager that copies a resolved product-auth
/// access secret into the capability dispatch scope.
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
        // type aliases for the host-api CredentialStageError — no conversion needed.
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

/// The GSuite Google-account visibility policy (extension-runtime DEL-7): for
/// the Google provider it applies the family-aware visibility rule; for every
/// other provider it defers to the plain requester authorization. Injected on
/// the build input so composition names no concrete first-party extension.
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

#[cfg(test)]
mod tests {
    use ironclaw_first_party_extensions::GMAIL_LIST_MESSAGES_CAPABILITY_ID;
    use ironclaw_reborn_composition::RuntimeDispatchErrorKind;

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
        assert_eq!(requirement.provider.as_str(), GOOGLE_PROVIDER_ID);
        assert_eq!(requirement.requester_extension.as_str(), "gmail");
    }
}
