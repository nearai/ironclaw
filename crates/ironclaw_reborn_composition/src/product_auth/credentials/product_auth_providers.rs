//! Auth-engine composition: assembles the ONE recipe-driven
//! [`ironclaw_auth::AuthEngine`] behind the product-auth services.
//!
//! There is deliberately no per-vendor provider client, no provider spec
//! constant, and no string→client multiplexor here (checklist AUTH-1/16):
//! vendors resolve to recipe DATA through the injected
//! [`ironclaw_auth::AuthRecipeResolver`], and deployment client credentials
//! resolve through a handle-keyed data map.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthEngine, AuthEngineDeps, AuthProductError, AuthProviderClient, AuthRecipeResolver,
    EngineCallbackBase, EngineClientCredentialsSource, EngineOAuthClientMaterial, OAuthClientId,
    StaticAuthRecipeResolver,
};
use ironclaw_extension_host::{AdminConfigurationResolvedValues, AdminConfigurationServiceError};
use ironclaw_host_api::{RecipeClientCredentials, ResourceScope, RuntimeHttpEgress, SecretHandle};
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use ironclaw_secrets::SecretStore;
use secrecy::SecretString;

use crate::RebornBuildError;
use crate::extension_host::admin_configuration::ComposedAdminConfigurationService;
use crate::input::{OAuthDcrCallbackConfig, OAuthProviderBackendConfig};
use crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver;
use crate::product_auth::oauth::staged_egress::ObligationStagedAuthEgress;

/// Display name sent with RFC 7591 dynamic client registration.
const DCR_CLIENT_NAME: &str = "Ironclaw";

/// The static vendor-callback base path (`{base}/{vendor}/callback` — the
/// serve layer mounts the matching `{provider}` route).
pub(crate) const PRODUCT_AUTH_OAUTH_ROUTE_BASE: &str = "/api/reborn/product-auth/oauth";

#[derive(Clone)]
pub(crate) struct OAuthProviderComposition {
    pub(crate) engine: Option<Arc<AuthEngine>>,
    pub(crate) client: Option<Arc<dyn AuthProviderClient>>,
    pub(crate) gate_driver: Option<Arc<OAuthGateFlowDriver>>,
}

/// One resolvable value for a deployment client-credential handle.
#[derive(Clone)]
pub(crate) enum ClientCredentialValue {
    Static(SecretString),
}

#[async_trait]
trait AdminClientCredentialSource: Send + Sync + fmt::Debug {
    async fn resolve(
        &self,
        handles: &[SecretHandle],
    ) -> Result<Option<AdminConfigurationResolvedValues>, AdminConfigurationServiceError>;
}

struct ComposedAdminClientCredentialSource {
    service: Arc<ComposedAdminConfigurationService>,
    scope: ResourceScope,
}

impl fmt::Debug for ComposedAdminClientCredentialSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposedAdminClientCredentialSource")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AdminClientCredentialSource for ComposedAdminClientCredentialSource {
    async fn resolve(
        &self,
        handles: &[SecretHandle],
    ) -> Result<Option<AdminConfigurationResolvedValues>, AdminConfigurationServiceError> {
        self.service
            .resolve_values_for_handles(&self.scope, handles)
            .await
    }
}

/// Deferred manifest-admin credential source. The administrator service is
/// built after the auth engine's durable dependencies, so composition fills
/// this slot once and the engine resolves a complete revisioned value set at
/// request time.
#[derive(Clone, Default)]
pub(crate) struct AdminConfigurationCredentialSlot {
    inner: Arc<std::sync::OnceLock<Arc<dyn AdminClientCredentialSource>>>,
}

impl AdminConfigurationCredentialSlot {
    pub(crate) fn fill(
        &self,
        service: Arc<ComposedAdminConfigurationService>,
        scope: ResourceScope,
    ) {
        let _ = self
            .inner
            .set(Arc::new(ComposedAdminClientCredentialSource {
                service,
                scope,
            }));
    }

    #[cfg(test)]
    fn fill_source(&self, source: Arc<dyn AdminClientCredentialSource>) {
        let _ = self.inner.set(source);
    }

    fn get(&self) -> Option<Arc<dyn AdminClientCredentialSource>> {
        self.inner.get().cloned()
    }
}

impl fmt::Debug for AdminConfigurationCredentialSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminConfigurationCredentialSlot")
            .field("filled", &self.inner.get().is_some())
            .finish()
    }
}

/// Handle-keyed deployment client-credential data. Recipes name their
/// `client_credentials` handles; the live manifest-declared administrator
/// configuration is authoritative, with composition-registered boot values
/// retained as a compatibility fallback. This keeps provider setup generic
/// and lets operator changes take effect on the next auth operation.
#[derive(Clone, Default)]
pub(crate) struct CompositionClientCredentials {
    values: BTreeMap<String, ClientCredentialValue>,
    admin_configuration: Option<AdminConfigurationCredentialSlot>,
}

impl CompositionClientCredentials {
    pub(crate) fn register_static(&mut self, handle: impl Into<String>, value: SecretString) {
        self.values
            .insert(handle.into(), ClientCredentialValue::Static(value));
    }

    pub(crate) fn with_admin_configuration_source(
        &mut self,
        slot: AdminConfigurationCredentialSlot,
    ) {
        self.admin_configuration = Some(slot);
    }

    fn static_value(&self, handle: &SecretHandle) -> Option<SecretString> {
        self.values.get(handle.as_str()).map(|value| match value {
            ClientCredentialValue::Static(value) => value.clone(),
        })
    }
}

impl fmt::Debug for CompositionClientCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompositionClientCredentials")
            .field("handles", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[async_trait]
impl EngineClientCredentialsSource for CompositionClientCredentials {
    async fn resolve(
        &self,
        vendor: &str,
        credentials: &RecipeClientCredentials,
    ) -> Result<EngineOAuthClientMaterial, AuthProductError> {
        use secrecy::ExposeSecret as _;
        let mut handles = vec![credentials.client_id_handle.clone()];
        if let Some(client_secret_handle) = &credentials.client_secret_handle {
            handles.push(client_secret_handle.clone());
        }
        if let Some(source) = self
            .admin_configuration
            .as_ref()
            .and_then(AdminConfigurationCredentialSlot::get)
        {
            let snapshot = source.resolve(&handles).await.map_err(|error| {
                tracing::warn!(
                    %error,
                    vendor,
                    "operator admin-configuration client-credential lookup failed"
                );
                AuthProductError::BackendUnavailable
            })?;
            if let Some(snapshot) = snapshot
                && !snapshot.values.is_empty()
            {
                let Some(client_id) = snapshot.values.get(&credentials.client_id_handle) else {
                    return Err(AuthProductError::MalformedConfig);
                };
                let client_secret = match &credentials.client_secret_handle {
                    Some(handle) => Some(
                        snapshot
                            .values
                            .get(handle)
                            .cloned()
                            .ok_or(AuthProductError::MalformedConfig)?,
                    ),
                    None => None,
                };
                tracing::debug!(
                    vendor,
                    revision = snapshot.revision,
                    "resolved OAuth client material from manifest administrator configuration"
                );
                return Ok(EngineOAuthClientMaterial {
                    client_id: OAuthClientId::new(client_id.expose_secret())?,
                    client_secret,
                });
            }
        }

        let Some(client_id) = self.static_value(&credentials.client_id_handle) else {
            tracing::debug!(
                vendor,
                handle = credentials.client_id_handle.as_str(),
                "vendor OAuth client id is not configured"
            );
            return Err(AuthProductError::MalformedConfig);
        };
        let client_secret = match &credentials.client_secret_handle {
            None => None,
            Some(handle) => self.static_value(handle),
        };
        if credentials.client_secret_handle.is_some() && client_secret.is_none() {
            return Err(AuthProductError::MalformedConfig);
        }
        Ok(EngineOAuthClientMaterial {
            client_id: OAuthClientId::new(client_id.expose_secret())?,
            client_secret,
        })
    }
}

/// Compose the auth engine from deployment inputs: the recipe catalog comes
/// from the bundled first-party manifests, deployment client material from
/// the vendor-keyed configs, and the static vendor-callback base from the
/// DCR callback origin or any configured redirect URI.
pub(crate) fn compose_provider_client(
    configs: Vec<OAuthProviderBackendConfig>,
    dcr_callback: Option<OAuthDcrCallbackConfig>,
    secret_store: Arc<dyn SecretStore>,
    runtime_ports: ProductAuthProviderRuntimePorts,
    admin_configuration_credentials: AdminConfigurationCredentialSlot,
    first_party_bundles: &[crate::extension_host::first_party::FirstPartyPackageBundle],
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let recipes: Arc<dyn AuthRecipeResolver> = Arc::new(StaticAuthRecipeResolver::new(
        crate::extension_host::available_extensions::AvailableExtensionCatalog::bundled_vendor_recipes(
            first_party_bundles,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("bundled vendor auth recipes could not be resolved: {error}"),
        })?,
    ));

    let mut client_credentials = CompositionClientCredentials::default();
    for config in &configs {
        register_vendor_client_config(&mut client_credentials, recipes.as_ref(), config);
    }
    client_credentials.with_admin_configuration_source(admin_configuration_credentials);
    let callback_base = dcr_callback
        .map(|dcr| {
            EngineCallbackBase::new(format!(
                "{}{PRODUCT_AUTH_OAUTH_ROUTE_BASE}",
                dcr.callback_origin.trim_end_matches('/')
            ))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("OAuth callback origin rejected: {error}"),
            })
        })
        .transpose()?
        .or_else(|| {
            configs
                .iter()
                .find_map(|config| callback_base_from_redirect(config.client.redirect_uri.as_str()))
        });

    compose_auth_engine(
        recipes,
        client_credentials,
        callback_base,
        secret_store,
        runtime_ports,
    )
}

/// Fill the vendor recipe's client-credential handles from deployment config.
fn register_vendor_client_config(
    credentials: &mut CompositionClientCredentials,
    recipes: &dyn AuthRecipeResolver,
    config: &OAuthProviderBackendConfig,
) {
    use secrecy::ExposeSecret as _;
    let Some(resolved) = recipes.recipe_for_vendor(&config.vendor) else {
        tracing::warn!(
            vendor = config.vendor,
            "no bundled recipe for configured OAuth vendor; client material not wired"
        );
        return;
    };
    let ironclaw_host_api::VendorAuthRecipe::Oauth2Code(recipe) = &resolved.recipe else {
        tracing::warn!(
            vendor = config.vendor,
            "configured OAuth vendor's recipe is not oauth2_code; client material not wired"
        );
        return;
    };
    let Some(handles) = &recipe.client_credentials else {
        tracing::debug!(
            vendor = config.vendor,
            "vendor recipe uses dynamic client registration; static client material ignored"
        );
        return;
    };
    credentials.register_static(
        handles.client_id_handle.as_str(),
        SecretString::from(config.client.client_id.as_str().to_string()),
    );
    if let (Some(secret_handle), Some(secret)) =
        (&handles.client_secret_handle, &config.client.client_secret)
    {
        credentials.register_static(
            secret_handle.as_str(),
            SecretString::from(secret.expose_secret().to_string()),
        );
    }
}

/// Derive the static callback base from a configured vendor redirect URI of
/// the `{base}/{vendor}/callback` shape.
fn callback_base_from_redirect(redirect: &str) -> Option<EngineCallbackBase> {
    let prefix = redirect.strip_suffix("/callback")?;
    let (base, _vendor) = prefix.rsplit_once('/')?;
    EngineCallbackBase::new(base).ok()
}

/// Compose the auth engine and the blocked-gate driver.
///
/// `callback_base` is the deployment's static vendor-callback base
/// (`.../product-auth/oauth`); without it (no public callback configured) no
/// engine is composed and OAuth connect flows stay unavailable, matching the
/// previous no-providers-configured behavior.
pub(crate) fn compose_auth_engine(
    recipes: Arc<dyn AuthRecipeResolver>,
    client_credentials: CompositionClientCredentials,
    callback_base: Option<EngineCallbackBase>,
    secret_store: Arc<dyn SecretStore>,
    runtime_ports: ProductAuthProviderRuntimePorts,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let Some(callback_base) = callback_base else {
        tracing::debug!("no OAuth callback base configured; auth engine not composed");
        return Ok(OAuthProviderComposition {
            engine: None,
            client: None,
            gate_driver: None,
        });
    };
    let egress: Arc<dyn RuntimeHttpEgress> = Arc::new(ObligationStagedAuthEgress::new(
        runtime_ports.runtime_http_egress(),
        runtime_ports.obligation_handler(),
    ));
    let engine = Arc::new(AuthEngine::new(AuthEngineDeps {
        recipes,
        client_credentials: Arc::new(client_credentials),
        egress,
        secret_store: Arc::clone(&secret_store),
        callback_base,
        dcr_client_name: DCR_CLIENT_NAME.to_string(),
    }));
    let gate_driver = Arc::new(OAuthGateFlowDriver::new(
        Arc::clone(&engine),
        Arc::clone(&secret_store),
    ));
    tracing::debug!("product-auth auth engine composed");
    Ok(OAuthProviderComposition {
        client: Some(Arc::clone(&engine) as Arc<dyn AuthProviderClient>),
        engine: Some(engine),
        gate_driver: Some(gate_driver),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_secrets::SecretMaterial;
    use secrecy::ExposeSecret as _;

    use super::*;

    #[derive(Debug)]
    struct StubAdminSource {
        result: Result<Option<AdminConfigurationResolvedValues>, AdminConfigurationServiceError>,
        calls: AtomicUsize,
    }

    impl StubAdminSource {
        fn new(
            result: Result<
                Option<AdminConfigurationResolvedValues>,
                AdminConfigurationServiceError,
            >,
        ) -> Self {
            Self {
                result,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AdminClientCredentialSource for StubAdminSource {
        async fn resolve(
            &self,
            _handles: &[SecretHandle],
        ) -> Result<Option<AdminConfigurationResolvedValues>, AdminConfigurationServiceError>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
    }

    fn recipe_credentials() -> RecipeClientCredentials {
        RecipeClientCredentials {
            client_id_handle: SecretHandle::new("client_id").expect("client-id handle"),
            client_secret_handle: Some(
                SecretHandle::new("client_secret").expect("client-secret handle"),
            ),
        }
    }

    fn boot_credentials() -> CompositionClientCredentials {
        let mut credentials = CompositionClientCredentials::default();
        credentials.register_static("client_id", SecretString::from("boot-id".to_string()));
        credentials.register_static(
            "client_secret",
            SecretString::from("boot-secret".to_string()),
        );
        credentials
    }

    fn admin_snapshot(revision: u64, values: &[(&str, &str)]) -> AdminConfigurationResolvedValues {
        AdminConfigurationResolvedValues {
            revision,
            values: values
                .iter()
                .map(|(handle, value)| {
                    (
                        SecretHandle::new(*handle).expect("admin handle"),
                        SecretMaterial::from((*value).to_string()),
                    )
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn admin_configuration_pair_overrides_boot_values_with_one_snapshot_read() {
        let source = Arc::new(StubAdminSource::new(Ok(Some(admin_snapshot(
            7,
            &[("client_id", "admin-id"), ("client_secret", "admin-secret")],
        )))));
        let slot = AdminConfigurationCredentialSlot::default();
        slot.fill_source(Arc::clone(&source) as Arc<dyn AdminClientCredentialSource>);
        let mut credentials = boot_credentials();
        credentials.with_admin_configuration_source(slot);

        let resolved = credentials
            .resolve("example", &recipe_credentials())
            .await
            .expect("admin pair resolves");

        assert_eq!(resolved.client_id.as_str(), "admin-id");
        assert_eq!(
            resolved
                .client_secret
                .expect("admin client secret")
                .expose_secret(),
            "admin-secret"
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn empty_admin_configuration_falls_back_to_complete_boot_pair() {
        let source = Arc::new(StubAdminSource::new(Ok(Some(admin_snapshot(0, &[])))));
        let slot = AdminConfigurationCredentialSlot::default();
        slot.fill_source(source as Arc<dyn AdminClientCredentialSource>);
        let mut credentials = boot_credentials();
        credentials.with_admin_configuration_source(slot);

        let resolved = credentials
            .resolve("example", &recipe_credentials())
            .await
            .expect("boot pair resolves");

        assert_eq!(resolved.client_id.as_str(), "boot-id");
        assert_eq!(
            resolved
                .client_secret
                .expect("boot client secret")
                .expose_secret(),
            "boot-secret"
        );
    }

    #[tokio::test]
    async fn partial_admin_configuration_never_mixes_with_boot_values() {
        let source = Arc::new(StubAdminSource::new(Ok(Some(admin_snapshot(
            3,
            &[("client_id", "admin-id")],
        )))));
        let slot = AdminConfigurationCredentialSlot::default();
        slot.fill_source(source as Arc<dyn AdminClientCredentialSource>);
        let mut credentials = boot_credentials();
        credentials.with_admin_configuration_source(slot);

        let error = credentials
            .resolve("example", &recipe_credentials())
            .await
            .expect_err("partial administrator revision fails closed");

        assert_eq!(error, AuthProductError::MalformedConfig);
    }

    #[tokio::test]
    async fn admin_configuration_failure_never_falls_back_to_boot_values() {
        let source = Arc::new(StubAdminSource::new(Err(
            AdminConfigurationServiceError::Unavailable,
        )));
        let slot = AdminConfigurationCredentialSlot::default();
        slot.fill_source(source as Arc<dyn AdminClientCredentialSource>);
        let mut credentials = boot_credentials();
        credentials.with_admin_configuration_source(slot);

        let error = credentials
            .resolve("example", &recipe_credentials())
            .await
            .expect_err("administrator read failure fails closed");

        assert_eq!(error, AuthProductError::BackendUnavailable);
    }
}
