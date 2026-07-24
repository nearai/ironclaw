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
use ironclaw_host_api::{RecipeClientCredentials, RuntimeHttpEgress};
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use ironclaw_secrets::SecretStore;
use secrecy::SecretString;

use crate::RebornBuildError;
use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;
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

/// Deferred handle source over administrator configuration
/// (`[admin_configuration]`): the resolver is built after the auth
/// engine (its durable stores land later in factory assembly), so the
/// engine holds this slot and resolves handles through it at request time.
/// Unfilled (startup window, or a composition path without the configure
/// surface) it resolves nothing — the engine's existing not-configured
/// path applies.
#[derive(Clone, Default)]
pub(crate) struct AdminConfigurationCredentialSlot {
    inner: Arc<std::sync::OnceLock<Arc<ComposedExtensionAdminConfigurationResolver>>>,
}

impl AdminConfigurationCredentialSlot {
    pub(crate) fn fill(&self, service: Arc<ComposedExtensionAdminConfigurationResolver>) {
        let _ = self.inner.set(service);
    }

    fn get(&self) -> Option<Arc<ComposedExtensionAdminConfigurationResolver>> {
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
/// `client_credentials` handles; composition registers values for those
/// handles (env config) — data, never a vendor code path. Handles without a
/// registered value fall back to the operator channel configuration, so
/// recipe client material saved through the generic configure surface
/// resolves with no per-vendor wiring.
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

    /// Attach the administrator-configuration fallback for unregistered handles.
    pub(crate) fn with_admin_configuration(&mut self, slot: AdminConfigurationCredentialSlot) {
        self.admin_configuration = Some(slot);
    }

    async fn resolve_handle(&self, handle: &str) -> Result<Option<SecretString>, AuthProductError> {
        match self.values.get(handle) {
            Some(ClientCredentialValue::Static(value)) => return Ok(Some(value.clone())),
            None => {}
        }
        let Some(service) = self
            .admin_configuration
            .as_ref()
            .and_then(|slot| slot.get())
        else {
            return Ok(None);
        };
        service
            .credential_handle_value(handle)
            .await
            .map_err(|error| {
                tracing::warn!(
                    %error,
                    handle,
                    "administrator client-credential lookup failed"
                );
                AuthProductError::BackendUnavailable
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
        let Some(client_id) = self
            .resolve_handle(credentials.client_id_handle.as_str())
            .await?
        else {
            tracing::debug!(
                vendor,
                handle = credentials.client_id_handle.as_str(),
                "vendor OAuth client id is not configured"
            );
            return Err(AuthProductError::MalformedConfig);
        };
        let client_secret = match &credentials.client_secret_handle {
            None => None,
            Some(handle) => self.resolve_handle(handle.as_str()).await?,
        };
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
    client_credentials.with_admin_configuration(admin_configuration_credentials);
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
