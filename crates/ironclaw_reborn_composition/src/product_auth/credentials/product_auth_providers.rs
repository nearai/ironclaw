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
use crate::input::{OAuthDcrCallbackConfig, OAuthProviderBackendConfig};
use crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver;
use crate::product_auth::oauth::staged_egress::ObligationStagedAuthEgress;
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack::slack_setup::SlackPersonalSetupServiceSlot;

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
    /// Resolved at request time (e.g. operator-entered setup secrets that
    /// arrive after startup).
    Dynamic(Arc<dyn DynamicClientCredentialLookup>),
}

/// Request-time lookup for one client-credential handle.
#[async_trait]
pub(crate) trait DynamicClientCredentialLookup: Send + Sync + fmt::Debug {
    async fn resolve(&self) -> Result<SecretString, AuthProductError>;
}

/// Handle-keyed deployment client-credential data. Recipes name their
/// `client_credentials` handles; composition registers values for those
/// handles (env config, setup services) — data, never a vendor code path.
#[derive(Clone, Default)]
pub(crate) struct CompositionClientCredentials {
    values: BTreeMap<String, ClientCredentialValue>,
}

impl CompositionClientCredentials {
    pub(crate) fn register_static(&mut self, handle: impl Into<String>, value: SecretString) {
        self.values
            .insert(handle.into(), ClientCredentialValue::Static(value));
    }

    pub(crate) fn register_dynamic(
        &mut self,
        handle: impl Into<String>,
        lookup: Arc<dyn DynamicClientCredentialLookup>,
    ) {
        self.values
            .insert(handle.into(), ClientCredentialValue::Dynamic(lookup));
    }

    async fn resolve_handle(&self, handle: &str) -> Result<Option<SecretString>, AuthProductError> {
        match self.values.get(handle) {
            None => Ok(None),
            Some(ClientCredentialValue::Static(value)) => Ok(Some(value.clone())),
            Some(ClientCredentialValue::Dynamic(lookup)) => lookup.resolve().await.map(Some),
        }
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
    #[cfg(feature = "slack-v2-host-beta")] slack_personal_oauth_slot: Option<
        SlackPersonalSetupServiceSlot,
    >,
    #[cfg(not(feature = "slack-v2-host-beta"))] _slack_personal_oauth_slot: Option<()>,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let recipes: Arc<dyn AuthRecipeResolver> = Arc::new(StaticAuthRecipeResolver::new(
        crate::extension_host::available_extensions::AvailableExtensionCatalog::bundled_vendor_recipes()
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("bundled vendor auth recipes could not be resolved: {error}"),
            })?,
    ));

    let mut client_credentials = CompositionClientCredentials::default();
    for config in &configs {
        register_vendor_client_config(&mut client_credentials, recipes.as_ref(), config);
    }
    #[cfg(feature = "slack-v2-host-beta")]
    let slack_slot_redirect = slack_personal_oauth_slot
        .as_ref()
        .map(|slot| slot.redirect_uri().as_str().to_string());
    #[cfg(not(feature = "slack-v2-host-beta"))]
    let slack_slot_redirect: Option<String> = None;
    #[cfg(feature = "slack-v2-host-beta")]
    if let Some(slot) = slack_personal_oauth_slot {
        crate::slack::slack_personal_oauth::register_slack_personal_client_credentials(
            &mut client_credentials,
            recipes.as_ref(),
            slot,
        );
    }

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
        })
        .or_else(|| {
            slack_slot_redirect
                .as_deref()
                .and_then(callback_base_from_redirect)
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
