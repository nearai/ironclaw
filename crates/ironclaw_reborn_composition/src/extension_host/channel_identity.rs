//! Generic post-OAuth channel identity binding (extension-runtime §5.5,
//! §6.3–§6.4).
//!
//! One vendor-blind post-exchange hook replaces the per-vendor identity
//! binding hooks: when an OAuth callback for provider `P` carries a proven
//! [`OAuthProviderIdentity`], the hook finds the installed extension(s)
//! whose manifest declares a channel surface and authenticates against `P`,
//! validates the identity's `team_id` / `enterprise_id` / `app_id` claims
//! against that extension's configured connection-scoping values, and writes
//! an installation-scoped [`RebornUserIdentityBinding`] for the
//! authenticated caller — handing the auth engine a rollback that undoes
//! exactly that binding if callback completion fails afterwards.
//!
//! Scoping is **fail-closed**: an extension whose connection scoping is not
//! configured yet (no scope, or a scope without any expected claim values)
//! rejects the bind with a typed reason instead of binding unscoped.
//!
//! Scoping values live in the extension's non-secret administrator
//! configuration
//! fields. The mapping from config field to identity claim is by handle
//! suffix: a non-secret field whose handle is `team_id` / `enterprise_id` /
//! `app_id` (or ends with `_team_id` / `_enterprise_id` / `_app_id`)
//! declares the expected value for that claim. A channel lane with an
//! externally supplied scope source uses a
//! [`ChannelIdentityOverride`] with its own scope source instead.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProductScope, OAuthProviderIdentity};
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_host_api::{ExtensionId, TenantId, UserId};
use ironclaw_product::AdapterInstallationId;

use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;
use crate::product_auth::api::auth::{
    OAuthProviderIdentityBindingRollback, OAuthProviderIdentityCheck,
    OAuthProviderIdentityCheckFuture,
};
use crate::provider_identity::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
    RebornUserIdentityBindingStore, installation_scoped_provider_user_id,
};

/// The identity claims the OAuth token exchange can prove
/// ([`OAuthProviderIdentity`]'s optional fields).
const SCOPING_CLAIMS: [&str; 3] = ["team_id", "enterprise_id", "app_id"];

/// Factory producing one post-exchange provider-identity check for a
/// callback's vendor id and scope (or `None` when nothing needs binding).
/// Registered on the product-auth route state by composition wiring.
pub(crate) type ProviderIdentityHookFactory =
    dyn Fn(&str, &AuthProductScope) -> Option<OAuthProviderIdentityCheck> + Send + Sync;

/// One extension's connection scope: the adapter installation the bindings
/// key under plus the identity claim values a proven vendor identity must
/// match before it may bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelConnectionScope {
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) expected_team_id: Option<String>,
    pub(crate) expected_enterprise_id: Option<String>,
    pub(crate) expected_app_id: Option<String>,
}

impl ChannelConnectionScope {
    /// Whether any scoping claim value is configured. A scope with no
    /// expected claims is "not configured yet" — the bind path fails closed.
    pub(crate) fn has_expected_claims(&self) -> bool {
        self.expected_team_id.is_some()
            || self.expected_enterprise_id.is_some()
            || self.expected_app_id.is_some()
    }

    /// The installation-scoped provider-user-id prefix every binding under
    /// this scope shares (the same composite-key scheme inbound actor
    /// resolution uses).
    pub(crate) fn provider_user_id_prefix(&self) -> String {
        format!("{}:", self.installation_id.as_str())
    }
}

/// Resolves one extension's current [`ChannelConnectionScope`].
///
/// `Ok(None)` means the extension's connection scoping is not configured
/// yet; the identity-binding and connection paths fail closed on it.
#[async_trait]
pub(crate) trait ChannelConnectionScopeSource: Send + Sync {
    async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String>;
}

/// Vendor residue port: fire-and-forget provisioning after a successful
/// identity bind (for example, opening the caller's personal DM target so
/// outbound delivery can reach them). Implementations spawn their own work
/// and surface failures via logs — a provisioning failure must never fail
/// the OAuth callback that already bound the identity.
pub(crate) trait ChannelIdentityPostBind: Send + Sync {
    fn provision_after_bind(&self, user_id: UserId, external_actor_id: &str);
}

/// Builds per-extension post-bind provisioning for generically-discovered
/// channel extensions (a lane override carries its own `post_bind`; the
/// generic DM-target provisioning implements this).
pub(crate) trait ChannelIdentityPostBindFactory: Send + Sync {
    fn post_bind_for_extension(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn ChannelIdentityPostBind>>;
}

/// Per-extension override for a channel lane with an external scope source:
/// the lane names the provider it binds under
/// and supplies its own scope source (and optional post-bind provisioning).
#[derive(Clone)]
pub(crate) struct ChannelIdentityOverride {
    pub(crate) extension_id: String,
    pub(crate) provider: String,
    pub(crate) scope_source: Arc<dyn ChannelConnectionScopeSource>,
    pub(crate) post_bind: Option<Arc<dyn ChannelIdentityPostBind>>,
}

/// Everything the generic post-OAuth identity binding hook needs.
///
/// Public because it crosses the `WebuiServeConfig` builder surface; hosts
/// obtain one from composition wiring rather than constructing it directly.
#[derive(Clone)]
pub struct ChannelIdentityBindingConfig {
    pub(crate) tenant_id: TenantId,
    /// Generic discovery + scoping-value source. `None` when the composed
    /// runtime has no durable installation store — only overrides bind then.
    pub(crate) installation_store: Option<Arc<dyn ExtensionInstallationStore>>,
    /// Effective manifest-driven administrator configuration. `None` means
    /// no deployment values are available and scoping fails closed.
    pub(crate) admin_configuration_resolver:
        Option<Arc<ComposedExtensionAdminConfigurationResolver>>,
    pub(crate) binding_store: Arc<dyn RebornUserIdentityBindingStore>,
    /// Undoes bindings written by the callback hook when OAuth completion
    /// fails afterwards; the binding is the user-visible "connected" signal,
    /// so it must not survive a completion failure that already deleted the
    /// token material.
    pub(crate) rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    /// Post-bind provisioning for generically-discovered extensions (e.g.
    /// DM-target provisioning). `None` = discovered binds provision nothing.
    pub(crate) post_bind_factory: Option<Arc<dyn ChannelIdentityPostBindFactory>>,
    pub(crate) overrides: Vec<ChannelIdentityOverride>,
}

impl ChannelIdentityBindingConfig {
    /// Test-support constructor exercising the generic (override-free)
    /// discovery path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        tenant_id: TenantId,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        binding_store: Arc<dyn RebornUserIdentityBindingStore>,
        rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    ) -> Self {
        Self {
            tenant_id,
            installation_store: Some(installation_store),
            admin_configuration_resolver: None,
            binding_store,
            rollback_store,
            post_bind_factory: None,
            overrides: Vec::new(),
        }
    }

    /// Test-support constructor for the production manifest-driven scoping
    /// path. Values are written through the canonical administrator
    /// configuration service; the installation store supplies membership and
    /// manifests only.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn for_test_with_admin_configuration(
        tenant_id: TenantId,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        binding_store: Arc<dyn RebornUserIdentityBindingStore>,
        rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
        values: Vec<(String, String)>,
    ) -> Result<Self, String> {
        use ironclaw_extension_host::{
            AdminConfigurationService, FilesystemAdminConfigurationStore,
        };
        use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
        use ironclaw_host_api::{InvocationId, ResourceScope};
        use ironclaw_secrets::{FilesystemSecretStore, SecretStore};

        let manifests = installation_store
            .list_manifests()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|record| Arc::new(record.resolved().clone()))
            .collect::<Vec<_>>();
        let descriptors = manifests
            .iter()
            .flat_map(|manifest| manifest.admin_configuration.clone())
            .collect::<Vec<_>>();
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let admin = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                    filesystem,
                    crate::invocation_mount_view,
                ))),
                secrets,
                descriptors.clone(),
            )
            .map_err(|error| error.to_string())?,
        );
        let mut scope = ResourceScope::local_default(
            UserId::new("admin-configuration-test").map_err(|error| error.to_string())?,
            InvocationId::new(),
        )
        .map_err(|error| error.to_string())?;
        scope.tenant_id = tenant_id.clone();
        let admin_configuration_resolver = Arc::new(
            ComposedExtensionAdminConfigurationResolver::new(admin, scope, manifests),
        );
        for descriptor in descriptors {
            let group_values = values
                .iter()
                .filter(|(handle, _)| {
                    descriptor
                        .fields
                        .iter()
                        .any(|field| field.handle.as_str() == handle)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !group_values.is_empty() {
                admin_configuration_resolver
                    .configure_admin_group_for_test(descriptor.group_id.as_str(), group_values)
                    .await?;
            }
        }
        let mut config =
            Self::for_test(tenant_id, installation_store, binding_store, rollback_store);
        config.admin_configuration_resolver = Some(admin_configuration_resolver);
        Ok(config)
    }
}

impl std::fmt::Debug for ChannelIdentityBindingConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelIdentityBindingConfig")
            .field("tenant_id", &self.tenant_id)
            .field(
                "overrides",
                &self
                    .overrides
                    .iter()
                    .map(|entry| (&entry.extension_id, &entry.provider))
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// One extension the callback's provider maps onto.
struct ChannelIdentityTarget {
    extension_id: String,
    scope_source: Arc<dyn ChannelConnectionScopeSource>,
    post_bind: Option<Arc<dyn ChannelIdentityPostBind>>,
}

/// The generic administrator-configuration-backed scope source: the
/// installation record supplies the adapter installation id; non-secret
/// manifest fields whose handles carry a claim suffix supply expected claims.
struct AdminConfigurationConnectionScopeSource {
    installation_store: Arc<dyn ExtensionInstallationStore>,
    extension_id: ExtensionId,
    admin_configuration_resolver: Option<Arc<ComposedExtensionAdminConfigurationResolver>>,
}

#[async_trait]
impl ChannelConnectionScopeSource for AdminConfigurationConnectionScopeSource {
    async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String> {
        let Some(record) = self
            .installation_store
            .get_manifest(&self.extension_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        if record.resolved().channel.is_none() {
            return Ok(None);
        }
        let installation = self
            .installation_store
            .list_installations()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|installation| installation.extension_id() == &self.extension_id);
        let Some(installation) = installation else {
            return Ok(None);
        };
        let installation_id = AdapterInstallationId::new(installation.installation_id().as_str())
            .map_err(|error| error.to_string())?;
        let values = if let Some(admin_configuration_resolver) = &self.admin_configuration_resolver
        {
            admin_configuration_resolver
                .effective_non_secret_config(&self.extension_id)
                .await
                .map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };
        let expected = |claim: &str| -> Option<String> {
            record
                .resolved()
                .admin_configuration
                .iter()
                .flat_map(|descriptor| &descriptor.fields)
                .filter(|field| !field.secret)
                .find(|field| handle_declares_claim(field.handle.as_str(), claim))
                .and_then(|field| {
                    values
                        .iter()
                        .find(|(handle, _)| handle == field.handle.as_str())
                        .map(|(_, value)| value.clone())
                })
                .filter(|value| !value.trim().is_empty())
        };
        Ok(Some(ChannelConnectionScope {
            installation_id,
            expected_team_id: expected("team_id"),
            expected_enterprise_id: expected("enterprise_id"),
            expected_app_id: expected("app_id"),
        }))
    }
}

/// Handle-suffix convention: `{claim}` or `*_{claim}` declares the expected
/// value for that identity claim.
fn handle_declares_claim(handle: &str, claim: &str) -> bool {
    handle == claim
        || handle
            .strip_suffix(claim)
            .is_some_and(|prefix| prefix.ends_with('_'))
}

/// The generic scope source for one extension over the durable installation
/// store — also used by the generic connection facade so the connect-report
/// prefix and the bind prefix can never diverge.
pub(crate) fn admin_configuration_connection_scope_source(
    installation_store: Arc<dyn ExtensionInstallationStore>,
    extension_id: ExtensionId,
    admin_configuration_resolver: Option<Arc<ComposedExtensionAdminConfigurationResolver>>,
) -> Arc<dyn ChannelConnectionScopeSource> {
    Arc::new(AdminConfigurationConnectionScopeSource {
        installation_store,
        extension_id,
        admin_configuration_resolver,
    })
}

/// A generically-discovered channel extension: its id and the auth vendors
/// its manifest declares. Shared by the identity hook and the connection
/// facade.
pub(crate) struct DiscoveredChannelExtension {
    pub(crate) extension_id: String,
    pub(crate) providers: Vec<String>,
}

/// Installed extensions whose manifest declares a channel surface, excluding
/// `overridden` extension ids (their lane owns identity binding). OAuth
/// channels expose one or more auth vendors; proof-code paired channels expose
/// none but must still be discoverable by the generic connection facade so
/// removal can revoke their pairing-owned state.
pub(crate) async fn discover_channel_extensions(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    overridden: &BTreeSet<String>,
) -> Result<Vec<DiscoveredChannelExtension>, String> {
    let manifests = installation_store
        .list_manifests()
        .await
        .map_err(|error| error.to_string())?;
    let mut discovered = Vec::new();
    for record in manifests {
        let resolved = record.resolved();
        if resolved.channel.is_none() {
            continue;
        }
        let extension_id = resolved.id.as_str().to_string();
        if overridden.contains(&extension_id) {
            continue;
        }
        discovered.push(DiscoveredChannelExtension {
            extension_id,
            providers: resolved
                .auth
                .iter()
                .map(|surface| surface.vendor.as_str().to_string())
                .collect(),
        });
    }
    Ok(discovered)
}

/// Build the provider-identity hook factory product-auth serve registers:
/// vendor-blind, resolving the callback's provider against the installed
/// channel extensions (plus lane overrides) at callback time.
pub fn channel_identity_binding_hook_factory(
    config: ChannelIdentityBindingConfig,
) -> Arc<ProviderIdentityHookFactory> {
    Arc::new(move |provider: &str, callback_scope: &AuthProductScope| {
        let config = config.clone();
        let provider = provider.to_string();
        let callback_scope = callback_scope.clone();
        Some(
            Box::new(move |provider_identity: Option<OAuthProviderIdentity>| {
                Box::pin(async move {
                    bind_channel_identities_for_callback(
                        &config,
                        &provider,
                        &callback_scope,
                        provider_identity.as_ref(),
                    )
                    .await
                }) as OAuthProviderIdentityCheckFuture
            }) as OAuthProviderIdentityCheck,
        )
    })
}

/// The hook body: find the provider's channel extensions, validate the
/// proven identity against each one's connection scope, bind, and return a
/// rollback confined to exactly the bindings this callback wrote.
/// `Ok(None)` when the provider maps to no channel extension — vendors
/// without a channel identity concept complete their callback untouched.
pub(crate) async fn bind_channel_identities_for_callback(
    config: &ChannelIdentityBindingConfig,
    provider: &str,
    callback_scope: &AuthProductScope,
    provider_identity: Option<&OAuthProviderIdentity>,
) -> Result<Option<OAuthProviderIdentityBindingRollback>, AuthProductError> {
    let targets = channel_identity_targets(config, provider).await?;
    if targets.is_empty() {
        return Ok(None);
    }
    let identity = provider_identity.ok_or(AuthProductError::MalformedCallback)?;
    if callback_scope.resource.tenant_id != config.tenant_id {
        return Err(AuthProductError::MalformedCallback);
    }
    let user_id = callback_scope.resource.user_id.clone();

    let mut bound: Vec<RebornIdentityProviderUserId> = Vec::new();
    for target in &targets {
        match bind_one_target(config, provider, target, identity, &user_id).await {
            Ok(provider_user_id) => bound.push(provider_user_id),
            Err(error) => {
                // A later target failing must not leave earlier bindings in
                // place: the callback is about to fail as a whole.
                roll_back_bindings(config, provider, &user_id, &bound).await;
                return Err(error);
            }
        }
    }

    let rollback_store = Arc::clone(&config.rollback_store);
    let rollback_provider = provider.to_string();
    Ok(Some(Box::pin(async move {
        for provider_user_id in &bound {
            // Passing the full provider_user_id as the prefix confines the
            // delete to the bindings this exact callback wrote. Best-effort
            // by contract: a rollback failure only errs toward "shows
            // connected without a credential", which disconnect repairs.
            if let Err(error) = rollback_store
                .delete_user_identity_bindings_for_user(
                    &rollback_provider,
                    &user_id,
                    Some(provider_user_id.as_str()),
                )
                .await
            {
                tracing::warn!(
                    %error,
                    provider = %rollback_provider,
                    "failed to roll back channel identity binding after OAuth completion failure"
                );
            }
        }
    })))
}

/// Resolve which extensions the callback's provider binds identities for:
/// lane overrides first, then generic discovery over the installation store.
async fn channel_identity_targets(
    config: &ChannelIdentityBindingConfig,
    provider: &str,
) -> Result<Vec<ChannelIdentityTarget>, AuthProductError> {
    let mut targets: Vec<ChannelIdentityTarget> = config
        .overrides
        .iter()
        .filter(|entry| entry.provider == provider)
        .map(|entry| ChannelIdentityTarget {
            extension_id: entry.extension_id.clone(),
            scope_source: Arc::clone(&entry.scope_source),
            post_bind: entry.post_bind.clone(),
        })
        .collect();
    let overridden: BTreeSet<String> = config
        .overrides
        .iter()
        .map(|entry| entry.extension_id.clone())
        .collect();
    if let Some(installation_store) = &config.installation_store {
        let discovered = discover_channel_extensions(installation_store, &overridden)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "channel extension discovery failed during OAuth callback");
                AuthProductError::BackendUnavailable
            })?;
        for extension in discovered {
            if !extension.providers.iter().any(|vendor| vendor == provider) {
                continue;
            }
            let extension_id = match ExtensionId::new(&extension.extension_id) {
                Ok(extension_id) => extension_id,
                Err(_) => continue,
            };
            let post_bind = config
                .post_bind_factory
                .as_ref()
                .and_then(|factory| factory.post_bind_for_extension(&extension.extension_id));
            targets.push(ChannelIdentityTarget {
                extension_id: extension.extension_id,
                scope_source: admin_configuration_connection_scope_source(
                    Arc::clone(installation_store),
                    extension_id,
                    config.admin_configuration_resolver.clone(),
                ),
                post_bind,
            });
        }
    }
    Ok(targets)
}

/// Validate the proven identity against one extension's connection scope
/// and write the installation-scoped binding. Returns the bound
/// provider-user id for rollback bookkeeping.
async fn bind_one_target(
    config: &ChannelIdentityBindingConfig,
    provider: &str,
    target: &ChannelIdentityTarget,
    identity: &OAuthProviderIdentity,
    user_id: &UserId,
) -> Result<RebornIdentityProviderUserId, AuthProductError> {
    let scope = target
        .scope_source
        .resolve_connection_scope()
        .await
        .map_err(|error| {
            tracing::warn!(
                %error,
                extension_id = %target.extension_id,
                "channel connection scope resolution failed during OAuth callback"
            );
            AuthProductError::BackendUnavailable
        })?;
    let Some(scope) = scope else {
        tracing::warn!(
            extension_id = %target.extension_id,
            "channel connection scoping is not configured yet; refusing identity bind"
        );
        return Err(AuthProductError::BackendUnavailable);
    };
    if !scope.has_expected_claims() {
        tracing::warn!(
            extension_id = %target.extension_id,
            "channel connection scoping values are not configured yet; refusing identity bind"
        );
        return Err(AuthProductError::BackendUnavailable);
    }
    let claims = [
        (
            SCOPING_CLAIMS[0],
            &scope.expected_team_id,
            &identity.team_id,
        ),
        (
            SCOPING_CLAIMS[1],
            &scope.expected_enterprise_id,
            &identity.enterprise_id,
        ),
        (SCOPING_CLAIMS[2], &scope.expected_app_id, &identity.app_id),
    ];
    for (claim, expected, proven) in claims {
        let Some(expected) = expected else { continue };
        if proven.as_deref() != Some(expected.as_str()) {
            tracing::warn!(
                extension_id = %target.extension_id,
                claim,
                "proven vendor identity does not match the configured connection scope"
            );
            return Err(AuthProductError::MalformedCallback);
        }
    }

    let binding = RebornUserIdentityBinding {
        provider: RebornIdentityProviderId::new(provider)
            .map_err(|_| AuthProductError::MalformedCallback)?,
        provider_user_id: RebornIdentityProviderUserId::new(installation_scoped_provider_user_id(
            &scope.installation_id,
            identity.subject.as_str(),
        ))
        .map_err(|_| AuthProductError::MalformedCallback)?,
        user_id: user_id.clone(),
    };
    let provider_user_id = binding.provider_user_id.clone();
    config
        .binding_store
        .bind_user_identity(binding)
        .await
        .map_err(|error| match error {
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound => {
                AuthProductError::ProviderIdentityAlreadyConnected
            }
            RebornUserIdentityBindingError::InvalidIdentityField { .. } => {
                AuthProductError::MalformedCallback
            }
            RebornUserIdentityBindingError::Backend(_) => AuthProductError::BackendUnavailable,
        })?;
    if let Some(post_bind) = &target.post_bind {
        post_bind.provision_after_bind(user_id.clone(), identity.subject.as_str());
    }
    Ok(provider_user_id)
}

/// Best-effort deletion of bindings already written by a callback whose
/// later target failed — the callback fails as a whole, so no partial
/// binding may survive it.
async fn roll_back_bindings(
    config: &ChannelIdentityBindingConfig,
    provider: &str,
    user_id: &UserId,
    bound: &[RebornIdentityProviderUserId],
) {
    for provider_user_id in bound {
        if let Err(error) = config
            .rollback_store
            .delete_user_identity_bindings_for_user(
                provider,
                user_id,
                Some(provider_user_id.as_str()),
            )
            .await
        {
            tracing::warn!(
                %error,
                "failed to roll back channel identity binding after a partial bind failure"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ironclaw_extension_host::{AdminConfigurationService, FilesystemAdminConfigurationStore};
    use ironclaw_extensions::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionManifestRecord,
        ExtensionManifestRef, FilesystemExtensionInstallationStore, ManifestSource,
    };
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{InvocationId, ResourceScope};
    use ironclaw_secrets::{FilesystemSecretStore, SecretStore};

    use super::*;
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

    /// An invented channel + auth extension: the vendor id is `acmechat`,
    /// the administrator schema declares two non-secret scoping fields keyed by
    /// the claim-suffix convention.
    const CHANNEL_AUTH_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acmechat"
name = "AcmeChat"
version = "0.1.0"
description = "channel identity binding fixture"
trust = "first_party_requested"

[admin_configuration]
group_id = "extension.acmechat"
display_name = "AcmeChat deployment configuration"
fields = [
  { handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true, required = false },
  { handle = "acmechat_team_id", label = "Workspace ID", secret = false, required = false },
  { handle = "acmechat_app_id", label = "App ID", secret = false, required = false },
  { handle = "acmechat_oauth_client_id", label = "OAuth client ID", secret = false, required = false },
]

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

# The [auth.acmechat] recipe must be referenced by a credential; the tool
# surface below is that reference (mirrors real channel+auth packages).
[[tools]]
id = "acmechat.read_messages"
description = "Read AcmeChat messages"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acmechat/read_messages.input.v1.json"

[[tools.credentials]]
handle = "acmechat_user_token"
vendor = "acmechat"
scopes = ["messages.read"]
audience = { scheme = "https", host = "api.acmechat.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "AcmeChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "acmechat_webhook_secret"
header = "X-AcmeChat-Secret"

[channel.presentation]
supports_markdown = false
supports_threads = false

[auth.acmechat]
method = "oauth2_code"
display_name = "AcmeChat account"
authorization_endpoint = "https://auth.acmechat.example/authorize"
token_endpoint = "https://auth.acmechat.example/token"
scopes = ["messages.read"]
client_credentials = { client_id_handle = "acmechat_oauth_client_id" }

[auth.acmechat.token_response]
access_token = "/access_token"

[auth.acmechat.identity]
account_id = "/authed_user/id"
team_id = "/team/id"
app_id = "/app_id"
"#;

    const FIXTURE_INSTALLATION_ID: &str = "acmechat-install-1";

    async fn installed_fixture_store() -> Arc<FilesystemExtensionInstallationStore> {
        let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        let record = ExtensionManifestRecord::from_toml(
            CHANNEL_AUTH_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let extension_id = ExtensionId::new("acmechat").expect("extension id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(FIXTURE_INSTALLATION_ID.to_string())
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extensions::InstallationOwner::user(
                        UserId::new("user:operator").expect("fixture user"),
                    ),
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
        store
    }

    async fn store_scoping_values(
        admin_configuration_resolver: &ComposedExtensionAdminConfigurationResolver,
    ) {
        admin_configuration_resolver
            .configure_admin_group_for_test(
                "extension.acmechat",
                vec![
                    ("acmechat_team_id".to_string(), "T-team".to_string()),
                    ("acmechat_app_id".to_string(), "A-app".to_string()),
                ],
            )
            .await
            .expect("store scoping values");
    }

    fn identity(team: &str, app: &str) -> OAuthProviderIdentity {
        OAuthProviderIdentity::new("U123", Some(team.to_string()), None, Some(app.to_string()))
            .expect("identity")
    }

    fn callback_scope(tenant: &TenantId, user: &str) -> AuthProductScope {
        let mut resource =
            ResourceScope::local_default(UserId::new(user).expect("user id"), InvocationId::new())
                .expect("resource scope");
        resource.tenant_id = tenant.clone();
        AuthProductScope::new(resource, ironclaw_auth::AuthSurface::Callback)
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-alpha").expect("tenant")
    }

    struct Fixture {
        config: ChannelIdentityBindingConfig,
        identity_store: Arc<RecordingIdentityStore>,
        admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    }

    async fn fixture() -> Fixture {
        let installation_store = installed_fixture_store().await;
        let manifest = Arc::new(
            installation_store
                .get_manifest(&ExtensionId::new("acmechat").expect("extension id"))
                .await
                .expect("load manifest")
                .expect("installed manifest")
                .resolved()
                .clone(),
        );
        let scope = ResourceScope::local_default(
            UserId::new("operator").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let admin = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                    filesystem,
                    crate::invocation_mount_view,
                ))),
                secrets,
                manifest.admin_configuration.clone(),
            )
            .expect("admin configuration service"),
        );
        let admin_configuration_resolver = Arc::new(
            ComposedExtensionAdminConfigurationResolver::new(admin, scope, [manifest]),
        );
        let identity_store = Arc::new(RecordingIdentityStore::default());
        let config = ChannelIdentityBindingConfig {
            tenant_id: tenant(),
            installation_store: Some(
                Arc::clone(&installation_store) as Arc<dyn ExtensionInstallationStore>
            ),
            admin_configuration_resolver: Some(Arc::clone(&admin_configuration_resolver)),
            binding_store: identity_store.clone(),
            rollback_store: identity_store.clone(),
            post_bind_factory: None,
            overrides: Vec::new(),
        };
        Fixture {
            config,
            identity_store,
            admin_configuration_resolver,
        }
    }

    #[tokio::test]
    async fn matching_identity_binds_installation_scoped_and_rollback_undoes_it() {
        let mut fixture = fixture().await;
        store_scoping_values(&fixture.admin_configuration_resolver).await;
        // Generic post-bind provisioning: the factory serves discovered
        // extensions; a successful bind must hand it the caller + subject.
        let post_bind = Arc::new(RecordingPostBind::default());
        fixture.config.post_bind_factory = Some(Arc::new(StaticPostBindFactory {
            post_bind: post_bind.clone(),
        }));

        let rollback = bind_channel_identities_for_callback(
            &fixture.config,
            "acmechat",
            &callback_scope(&tenant(), "user-alice"),
            Some(&identity("T-team", "A-app")),
        )
        .await
        .expect("bind succeeds")
        .expect("a channel extension bind returns a rollback");

        assert_eq!(
            fixture.identity_store.bindings(),
            vec![RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("acmechat").expect("provider"),
                provider_user_id: RebornIdentityProviderUserId::new(format!(
                    "{FIXTURE_INSTALLATION_ID}:U123"
                ))
                .expect("provider user id"),
                user_id: UserId::new("user-alice").expect("user"),
            }],
            "the binding must be keyed by the installation-scoped composite id"
        );

        assert_eq!(
            post_bind.calls(),
            vec![(UserId::new("user-alice").expect("user"), "U123".to_string())],
            "a discovered-extension bind must fire the factory's post-bind provisioning"
        );

        // The returned rollback (callback completion failed afterwards) must
        // delete exactly the binding this callback wrote.
        rollback.await;
        assert_eq!(
            fixture.identity_store.deletes(),
            vec![(
                "acmechat".to_string(),
                UserId::new("user-alice").expect("user"),
                Some(format!("{FIXTURE_INSTALLATION_ID}:U123")),
            )]
        );
    }

    #[tokio::test]
    async fn claim_mismatch_rejects_without_write() {
        let fixture = fixture().await;
        store_scoping_values(&fixture.admin_configuration_resolver).await;

        for wrong in [identity("T-other", "A-app"), identity("T-team", "A-other")] {
            let error = expect_reject(
                bind_channel_identities_for_callback(
                    &fixture.config,
                    "acmechat",
                    &callback_scope(&tenant(), "user-alice"),
                    Some(&wrong),
                )
                .await,
                "scope mismatch is rejected",
            );
            assert!(matches!(error, AuthProductError::MalformedCallback));
        }
        // A proven identity missing a claim the scope expects is a mismatch.
        let missing_claim =
            OAuthProviderIdentity::new("U123", None, None, Some("A-app".to_string()))
                .expect("identity");
        let error = expect_reject(
            bind_channel_identities_for_callback(
                &fixture.config,
                "acmechat",
                &callback_scope(&tenant(), "user-alice"),
                Some(&missing_claim),
            )
            .await,
            "missing proven claim is rejected",
        );
        assert!(matches!(error, AuthProductError::MalformedCallback));
        assert_eq!(fixture.identity_store.bindings(), Vec::new());
    }

    #[tokio::test]
    async fn missing_scoping_config_rejects_instead_of_binding_unscoped() {
        // No scoping values were saved: the extension is "not configured
        // yet" and the bind must fail closed, never bind without scoping.
        let fixture = fixture().await;

        let error = expect_reject(
            bind_channel_identities_for_callback(
                &fixture.config,
                "acmechat",
                &callback_scope(&tenant(), "user-alice"),
                Some(&identity("T-team", "A-app")),
            )
            .await,
            "unconfigured scoping must reject",
        );
        assert!(matches!(error, AuthProductError::BackendUnavailable));
        assert_eq!(fixture.identity_store.bindings(), Vec::new());
    }

    #[tokio::test]
    async fn provider_without_channel_extension_is_a_no_op() {
        let fixture = fixture().await;
        store_scoping_values(&fixture.admin_configuration_resolver).await;

        let rollback = bind_channel_identities_for_callback(
            &fixture.config,
            "unrelated-vendor",
            &callback_scope(&tenant(), "user-alice"),
            Some(&identity("T-team", "A-app")),
        )
        .await
        .expect("non-channel provider callbacks complete untouched");
        assert!(rollback.is_none());
        assert_eq!(fixture.identity_store.bindings(), Vec::new());
    }

    #[tokio::test]
    async fn missing_identity_and_foreign_tenant_reject() {
        let fixture = fixture().await;
        store_scoping_values(&fixture.admin_configuration_resolver).await;

        let error = expect_reject(
            bind_channel_identities_for_callback(
                &fixture.config,
                "acmechat",
                &callback_scope(&tenant(), "user-alice"),
                None,
            )
            .await,
            "a channel provider callback without proven identity is rejected",
        );
        assert!(matches!(error, AuthProductError::MalformedCallback));

        let other_tenant = TenantId::new("tenant-other").expect("tenant");
        let error = expect_reject(
            bind_channel_identities_for_callback(
                &fixture.config,
                "acmechat",
                &callback_scope(&other_tenant, "user-alice"),
                Some(&identity("T-team", "A-app")),
            )
            .await,
            "foreign tenant is rejected",
        );
        assert!(matches!(error, AuthProductError::MalformedCallback));
        assert_eq!(fixture.identity_store.bindings(), Vec::new());
    }

    #[tokio::test]
    async fn already_bound_identity_maps_to_already_connected() {
        let fixture = fixture().await;
        store_scoping_values(&fixture.admin_configuration_resolver).await;
        fixture.identity_store.seed(
            format!("{FIXTURE_INSTALLATION_ID}:U123"),
            UserId::new("user-bob").expect("user"),
        );

        let error = expect_reject(
            bind_channel_identities_for_callback(
                &fixture.config,
                "acmechat",
                &callback_scope(&tenant(), "user-alice"),
                Some(&identity("T-team", "A-app")),
            )
            .await,
            "an identity bound to a different user is rejected",
        );
        assert!(matches!(
            error,
            AuthProductError::ProviderIdentityAlreadyConnected
        ));
    }

    #[tokio::test]
    async fn override_scope_source_wins_and_post_bind_fires() {
        // A lane override binds under its own scope (its configure surface
        // uses an external scope source) and receives the post-bind signal.
        let identity_store = Arc::new(RecordingIdentityStore::default());
        let post_bind = Arc::new(RecordingPostBind::default());
        let config = ChannelIdentityBindingConfig {
            tenant_id: tenant(),
            installation_store: None,
            admin_configuration_resolver: None,
            binding_store: identity_store.clone(),
            rollback_store: identity_store.clone(),
            post_bind_factory: None,
            overrides: vec![ChannelIdentityOverride {
                extension_id: "acmechat".to_string(),
                provider: "acmechat".to_string(),
                scope_source: Arc::new(StaticScopeSource(Some(ChannelConnectionScope {
                    installation_id: AdapterInstallationId::new("lane-install")
                        .expect("installation"),
                    expected_team_id: Some("T-team".to_string()),
                    expected_enterprise_id: None,
                    expected_app_id: Some("A-app".to_string()),
                }))),
                post_bind: Some(post_bind.clone()),
            }],
        };

        let rollback = bind_channel_identities_for_callback(
            &config,
            "acmechat",
            &callback_scope(&tenant(), "user-alice"),
            Some(&identity("T-team", "A-app")),
        )
        .await
        .expect("bind succeeds")
        .expect("rollback returned");
        drop(rollback);

        assert_eq!(
            fixture_binding_ids(&identity_store),
            vec!["lane-install:U123".to_string()],
            "the override's scope keys the binding, not the generic installation id"
        );
        assert_eq!(
            post_bind.calls(),
            vec![(UserId::new("user-alice").expect("user"), "U123".to_string())]
        );
    }

    #[tokio::test]
    async fn scope_without_expected_claims_rejects() {
        let identity_store = Arc::new(RecordingIdentityStore::default());
        let config = ChannelIdentityBindingConfig {
            tenant_id: tenant(),
            installation_store: None,
            admin_configuration_resolver: None,
            binding_store: identity_store.clone(),
            rollback_store: identity_store.clone(),
            post_bind_factory: None,
            overrides: vec![ChannelIdentityOverride {
                extension_id: "acmechat".to_string(),
                provider: "acmechat".to_string(),
                scope_source: Arc::new(StaticScopeSource(Some(ChannelConnectionScope {
                    installation_id: AdapterInstallationId::new("lane-install")
                        .expect("installation"),
                    expected_team_id: None,
                    expected_enterprise_id: None,
                    expected_app_id: None,
                }))),
                post_bind: None,
            }],
        };

        let error = expect_reject(
            bind_channel_identities_for_callback(
                &config,
                "acmechat",
                &callback_scope(&tenant(), "user-alice"),
                Some(&identity("T-team", "A-app")),
            )
            .await,
            "a scope without expected claims is 'not configured yet'",
        );
        assert!(matches!(error, AuthProductError::BackendUnavailable));
        assert_eq!(identity_store.bindings(), Vec::new());
    }

    #[test]
    fn handle_suffix_convention_matches_claim_handles_only() {
        assert!(handle_declares_claim("team_id", "team_id"));
        assert!(handle_declares_claim("acmechat_team_id", "team_id"));
        assert!(handle_declares_claim("acmechat_api_app_id", "app_id"));
        assert!(!handle_declares_claim("acmechat_steam_id", "team_id"));
        assert!(!handle_declares_claim("acmechat_webhook_secret", "team_id"));
    }

    fn fixture_binding_ids(store: &RecordingIdentityStore) -> Vec<String> {
        store
            .bindings()
            .into_iter()
            .map(|binding| binding.provider_user_id.as_str().to_string())
            .collect()
    }

    /// `expect_err` needs `Debug` on the success payload; the rollback
    /// future has none, so unwrap rejections manually.
    fn expect_reject(
        result: Result<Option<OAuthProviderIdentityBindingRollback>, AuthProductError>,
        context: &str,
    ) -> AuthProductError {
        match result {
            Ok(_) => panic!("{context}: expected a rejection"),
            Err(error) => error,
        }
    }

    struct StaticScopeSource(Option<ChannelConnectionScope>);

    /// Serves one recording post-bind for every discovered extension.
    struct StaticPostBindFactory {
        post_bind: Arc<RecordingPostBind>,
    }

    impl ChannelIdentityPostBindFactory for StaticPostBindFactory {
        fn post_bind_for_extension(
            &self,
            _extension_id: &str,
        ) -> Option<Arc<dyn ChannelIdentityPostBind>> {
            Some(Arc::clone(&self.post_bind) as Arc<dyn ChannelIdentityPostBind>)
        }
    }

    #[async_trait]
    impl ChannelConnectionScopeSource for StaticScopeSource {
        async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String> {
            Ok(self.0.clone())
        }
    }

    #[derive(Default)]
    struct RecordingPostBind {
        calls: Mutex<Vec<(UserId, String)>>,
    }

    impl RecordingPostBind {
        fn calls(&self) -> Vec<(UserId, String)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl ChannelIdentityPostBind for RecordingPostBind {
        fn provision_after_bind(&self, user_id: UserId, external_actor_id: &str) {
            self.calls
                .lock()
                .expect("lock")
                .push((user_id, external_actor_id.to_string()));
        }
    }

    #[derive(Default)]
    pub(crate) struct RecordingIdentityStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
        existing: Mutex<HashMap<String, UserId>>,
        deletes: Mutex<Vec<(String, UserId, Option<String>)>>,
    }

    impl RecordingIdentityStore {
        fn seed(&self, provider_user_id: String, user_id: UserId) {
            self.existing
                .lock()
                .expect("lock")
                .insert(provider_user_id, user_id);
        }

        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings.lock().expect("lock").clone()
        }

        fn deletes(&self) -> Vec<(String, UserId, Option<String>)> {
            self.deletes.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl RebornUserIdentityBindingStore for RecordingIdentityStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            if let Some(existing) = self
                .existing
                .lock()
                .expect("lock")
                .get(binding.provider_user_id.as_str())
                && existing != &binding.user_id
            {
                return Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound);
            }
            self.bindings.lock().expect("lock").push(binding);
            Ok(())
        }
    }

    #[async_trait]
    impl RebornUserIdentityBindingDeleteStore for RecordingIdentityStore {
        async fn delete_user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<usize, RebornUserIdentityBindingError> {
            self.deletes.lock().expect("lock").push((
                provider.to_string(),
                user_id.clone(),
                provider_user_id_prefix.map(ToString::to_string),
            ));
            let mut bindings = self.bindings.lock().expect("lock");
            let before = bindings.len();
            bindings.retain(|binding| {
                let prefix_matches = provider_user_id_prefix
                    .map(|prefix| binding.provider_user_id.as_str().starts_with(prefix))
                    .unwrap_or(true);
                !(binding.provider.as_str() == provider
                    && &binding.user_id == user_id
                    && prefix_matches)
            });
            Ok(before - bindings.len())
        }
    }
}
