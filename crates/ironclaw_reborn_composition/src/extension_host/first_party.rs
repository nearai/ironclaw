//! Composition-neutral first-party package data + handler-registrar seam
//! (extension-runtime DEL-7).
//!
//! Composition must not name `ironclaw_first_party_extensions` in production
//! code, so the binary (`ironclaw_reborn_cli`) converts the concrete
//! `ironclaw_first_party_extensions::packages::PackageBundle` inventory into
//! these neutral, data-only [`FirstPartyPackageBundle`]s and injects them on the
//! [`crate::RebornHostBindings`]. Likewise it supplies concrete first-party
//! capability executors (GSuite, web tooling) as [`FirstPartyHandlerRegistrar`]s;
//! composition owns the generic registration loop and the
//! [`FirstPartyRegistrarContext`] each registrar consumes.

use std::sync::Arc;

use ironclaw_auth::{CredentialAccountRecordSource, CredentialAccountService};
use ironclaw_host_api::{EffectKind, HostApiError};
use ironclaw_host_runtime::{FirstPartyCapabilityRegistry, ProductAuthProviderRuntimePorts};

/// Byte content of one asset shipped inside a first-party package.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// A package's user-facing onboarding copy, carried as plain data (mirrors
/// `ironclaw_first_party_extensions::packages::PackageOnboarding`).
#[derive(Debug, Clone)]
pub struct FirstPartyPackageOnboarding {
    pub instructions: String,
    pub credential_instructions: Option<String>,
    pub setup_url: Option<String>,
    pub credential_next_step: String,
}

/// A bespoke OAuth-*setup* credential requirement replacing the manifest-derived
/// one (mirrors `ironclaw_first_party_extensions::packages::PackageOAuthSetup`).
#[derive(Debug, Clone)]
pub struct FirstPartyPackageOAuthSetup {
    pub requirement_name: String,
    pub provider: String,
    pub scopes: Vec<String>,
}

/// An opaque, data-only first-party package the binary hands composition. Host
/// code consumes this without naming the concrete package; the concrete
/// identity lives only in the injecting binary.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageBundle {
    pub id: String,
    pub display_name: String,
    pub manifest_toml: String,
    pub assets: Vec<FirstPartyPackageAsset>,
    /// Bespoke onboarding copy, `None` for packages that need no setup guidance.
    pub onboarding: Option<FirstPartyPackageOnboarding>,
    /// A bespoke OAuth-setup credential requirement replacing the
    /// manifest-derived one, `None` when the derived requirement is correct.
    pub oauth_setup: Option<FirstPartyPackageOAuthSetup>,
    /// Host authority effects this package is granted in the built-in trust
    /// policy (defense in depth; not derived from the manifest). `None` for
    /// packages whose trust comes from the WASM extension registry.
    pub trust_effects: Option<Vec<EffectKind>>,
    /// Extra catalog search aliases folded in by the injecting binary (e.g. the
    /// GSuite family's "google"/"workspace" terms), so composition search does
    /// not special-case any concrete id.
    pub search_aliases: Vec<String>,
}

/// The context composition supplies to each [`FirstPartyHandlerRegistrar`] so
/// the binary-owned registrar can build its concrete executor wrappers with the
/// host-mediated ports.
pub struct FirstPartyRegistrarContext {
    pub credential_account_service: Arc<dyn CredentialAccountService>,
    pub credential_account_record_source: Arc<dyn CredentialAccountRecordSource>,
    pub product_auth_runtime_ports: ProductAuthProviderRuntimePorts,
    /// Whether a Google OAuth backend was registered at build time. Gates a
    /// pre-dispatch "not configured" tool result (see the GSuite handler).
    pub google_oauth_configured: bool,
}

/// A binary-assembled first-party capability handler installer. Composition
/// runs every registrar once against the shared registry before installing it
/// via `with_first_party_capabilities`; the concrete executors and capability
/// ids live in the binary, never composition.
pub trait FirstPartyHandlerRegistrar: Send + Sync {
    fn register(
        &self,
        registry: &mut FirstPartyCapabilityRegistry,
        context: &FirstPartyRegistrarContext,
    ) -> Result<(), HostApiError>;
}

/// The reserved host-bundled extension ids contributed by the injected first
/// party bundle set: a filesystem/uploaded extension must never shadow one of
/// these ids. The NEAR AI host-managed extension id is reserved separately by
/// the catalog (it is not part of the injected inventory).
pub(crate) fn first_party_reserved_extension_ids(
    bundles: &[FirstPartyPackageBundle],
) -> Vec<String> {
    bundles.iter().map(|bundle| bundle.id.clone()).collect()
}

/// Convert the concrete `ironclaw_first_party_extensions` package inventory into
/// neutral [`FirstPartyPackageBundle`]s. Test-support only: this is the one
/// composition-side spot allowed to name the concrete inventory, mirroring the
/// conversion the binary performs in production, so unit tests can build the
/// catalog and trust policy without the binary. Production sources the bundles
/// from the injected build input.
///
/// Gated to crate tests and `test-support`: it names the concrete
/// `ironclaw_first_party_extensions` crate, which is a dev-dependency for
/// composition's own tests and an optional dependency enabled only by
/// `test-support` for downstream integration harnesses.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn first_party_bundles_from_inventory() -> Vec<FirstPartyPackageBundle> {
    use ironclaw_first_party_extensions::is_gsuite_extension_id;
    use ironclaw_first_party_extensions::packages::{PackageAssetContent, bundled_packages};
    use ironclaw_host_api::ExtensionId;

    bundled_packages()
        .into_iter()
        .map(|bundle| {
            let assets = bundle
                .assets
                .into_iter()
                .map(|asset| {
                    let PackageAssetContent::Bytes(bytes) = asset.content;
                    FirstPartyPackageAsset {
                        path: asset.path,
                        bytes,
                    }
                })
                .collect();
            // Fold the GSuite family's catalog search aliases into the bundle so
            // composition search never special-cases a concrete id (mirrors the
            // binary-side conversion).
            let search_aliases = if ExtensionId::new(bundle.id)
                .map(|id| is_gsuite_extension_id(&id))
                .unwrap_or(false)
            {
                [
                    "google",
                    "gsuite",
                    "g suite",
                    "workspace",
                    "google workspace",
                ]
                .into_iter()
                .map(str::to_string)
                .collect()
            } else {
                Vec::new()
            };
            FirstPartyPackageBundle {
                id: bundle.id.to_string(),
                display_name: bundle.display_name.to_string(),
                manifest_toml: bundle.manifest_toml.into_owned(),
                assets,
                onboarding: bundle.onboarding.map(|copy| FirstPartyPackageOnboarding {
                    instructions: copy.instructions,
                    credential_instructions: copy.credential_instructions,
                    setup_url: copy.setup_url,
                    credential_next_step: copy.credential_next_step,
                }),
                // #6442×#6520 reconciliation: the source `PackageBundle` no
                // longer carries a bespoke `oauth_setup` override (#6520 folded
                // first-party OAuth setup into the manifest's credential
                // requirements). No bundle-level override is minted here; the
                // manifest-derived requirement is authoritative.
                oauth_setup: None,
                trust_effects: bundle.trust_effects,
                search_aliases,
            }
        })
        .collect()
}

/// Test-support first-party handler registrars (GSuite + web tooling) mirroring
/// the concrete executors the `ironclaw_reborn_cli` binary assembles in
/// production (`crates/ironclaw_reborn_cli/src/first_party/`).
///
/// Composition names `ironclaw_first_party_extensions` in production nowhere
/// (extension-runtime DEL-7); the binary supplies these registrars on the build
/// input. Composition's own unit tests re-derive the same wiring here through
/// the dev-dependency so a test can install/activate/dispatch the first-party
/// extensions through the production registrar seam without the binary. Gated
/// for the same reason as `first_party_bundles_from_inventory`.
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_auth::{CredentialAccount, CredentialAccountSelectionRequest};
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
        RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement,
        RuntimeCredentialRequirement, RuntimeCredentialRequirementSource, RuntimeCredentialTarget,
        RuntimeDispatchErrorKind, SecretHandle, VendorId,
    };
    use ironclaw_host_runtime::{
        FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
        FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ProductAuthProviderRuntimePorts,
    };

    use super::{FirstPartyHandlerRegistrar, FirstPartyRegistrarContext};
    use crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy;

    /// The full set of first-party handler registrars a local-dev/test build
    /// needs, mirroring `ironclaw_reborn_cli::first_party::bundled_first_party_registrars`.
    pub(crate) fn bundled_first_party_registrars() -> Vec<Arc<dyn FirstPartyHandlerRegistrar>> {
        vec![
            Arc::new(GsuiteFirstPartyRegistrar),
            Arc::new(WebToolFirstPartyRegistrar),
        ]
    }

    /// The GSuite Google-account visibility policy, mirroring the binary's
    /// `first_party_credential_account_visibility_policy()`.
    pub(crate) fn bundled_credential_account_visibility_policy()
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
                google_oauth_configured: context.google_oauth_configured,
            });
            for package in gsuite_package_specs() {
                for capability in package.capabilities {
                    registry
                        .insert_handler(CapabilityId::new(capability.id)?, Arc::clone(&handler));
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
                .ok_or_else(|| {
                    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied)
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
                Ok(credential_requirements) => {
                    FirstPartyCapabilityError::auth_required_with_context(
                        required_secrets,
                        credential_requirements,
                    )
                }
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
}
