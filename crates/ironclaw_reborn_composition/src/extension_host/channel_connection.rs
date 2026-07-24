//! Generic per-user channel connection facade (extension-runtime §6.4).
//!
//! One vendor-blind [`ChannelConnectionFacade`] replaces the per-vendor
//! facades: every installed extension whose manifest declares a channel
//! surface is discovered. OAuth connection state is derived from the
//! identity-binding store; proof-code connection state comes from the generic
//! pairing registry. Disconnect runs the owner-specific cleanup before the
//! installation disappears: revoke any personal vendor credential → pairing
//! or per-extension residue cleanup → delete the caller's identity bindings.
//! The binding is the "connected" signal and deletes last (commit point);
//! the credential revokes first so a mid-sequence failure leaves the caller
//! visibly connected with every step retryable.
//!
//! Channel lanes whose configure surface predates `[channel.config]`
//! register a [`ChannelConnectionEntry`] carrying their own scope source and
//! cleanup port; pure-manifest extensions are discovered generically over
//! the durable installation store.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountStatus, SecretCleanupAction,
    SecretCleanupReport, SecretCleanupRequest,
};
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_host_api::{
    ExtensionId, InvocationId, ProductSurfaceCaller, ProductSurfaceError, ResourceScope, TenantId,
};
use ironclaw_product::{ChannelAuthAccountState, ChannelConnectionFacade};

use crate::extension_host::channel_identity::{
    ChannelConnectionScope, ChannelConnectionScopeSource, channel_config_connection_scope_source,
    discover_channel_extensions,
};
use crate::provider_identity::{RebornUserIdentityBindingDeleteStore, RebornUserIdentityLookup};
use ironclaw_extension_host::FilesystemChannelDmTargetStore;

/// Narrow disconnect-side port over product-auth lifecycle cleanup, so the
/// per-user channel disconnect can revoke the caller's personal vendor
/// credential without depending on the whole product-auth bundle (and so
/// tests can record the issued cleanup). Production forwards to
/// [`crate::RebornProductAuthServices::cleanup_credentials_for_lifecycle`],
/// the guardrail-sanctioned lifecycle cleanup entry point.
#[async_trait]
pub(crate) trait ChannelCredentialCleanup: Send + Sync {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError>;
}

#[async_trait]
impl ChannelCredentialCleanup for crate::RebornProductAuthServices {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        crate::RebornProductAuthServices::cleanup_credentials_for_lifecycle(self, request)
            .await
            .map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "channel credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}

/// Read-side port over the caller's durable credential-account status for a
/// vendor, so the extensions wire can project each channel account's real
/// §6.3 state (`connected` / `expired` / `disconnected`) instead of the
/// connected/disconnected collapse the identity-binding bool alone permits.
/// Production forwards to the product-auth
/// [`crate::RebornProductAuthServices::credential_account_record_source`]; the
/// facade leaves the live-flow (`authenticating`) axis to the auth-flow
/// projection that owns thread-scoped setup flows.
#[async_trait]
pub(crate) trait ChannelAccountStatusReader: Send + Sync {
    /// The caller's durable credential-account status for `provider`, or `None`
    /// when the caller holds no account for that vendor.
    async fn account_status_for_caller(
        &self,
        caller: &ProductSurfaceCaller,
        provider: &str,
    ) -> Result<Option<CredentialAccountStatus>, ProductSurfaceError>;
}

#[async_trait]
impl ChannelAccountStatusReader for crate::RebornProductAuthServices {
    async fn account_status_for_caller(
        &self,
        caller: &ProductSurfaceCaller,
        provider: &str,
    ) -> Result<Option<CredentialAccountStatus>, ProductSurfaceError> {
        let provider_id = AuthProviderId::new(provider)
            .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
        let scope = AuthProductScope::credential_owner(
            &ResourceScope {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
                agent_id: caller.agent_id.clone(),
                project_id: caller.project_id.clone(),
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        );
        let accounts = self
            .credential_account_record_source()
            .accounts_for_owner(&scope)
            .await
            .map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "channel account status lookup failed: {error:?}"
                ))
            })?;
        Ok(accounts
            .into_iter()
            .find(|account| account.provider == provider_id)
            .map(|account| account.status))
    }
}

/// Vendor residue port: per-caller cleanup a channel lane still owns when
/// the caller disconnects (for example, deleting personal delivery targets
/// keyed by the installation). Runs only when the extension's connection
/// scope resolves — targets keyed by an installation are unreachable while
/// no scope exists.
#[async_trait]
pub(crate) trait ChannelDisconnectCleanup: Send + Sync {
    async fn cleanup_disconnected_caller(
        &self,
        caller: &ProductSurfaceCaller,
        scope: &ChannelConnectionScope,
    ) -> Result<(), String>;
}

/// One channel lane's registration with the generic facade: the extension
/// id, the auth vendors whose bindings mean "connected", the lane's scope
/// source, and its optional disconnect-side cleanup.
#[derive(Clone)]
pub(crate) struct ChannelConnectionEntry {
    pub(crate) extension_id: String,
    pub(crate) providers: Vec<String>,
    pub(crate) scope_source: Arc<dyn ChannelConnectionScopeSource>,
    pub(crate) disconnect_cleanup: Option<Arc<dyn ChannelDisconnectCleanup>>,
}

/// The generic per-user channel connection facade.
pub(crate) struct GenericChannelConnectionFacade {
    tenant_id: TenantId,
    /// Lane-registered entries; win over generic discovery for their ids.
    entries: Vec<ChannelConnectionEntry>,
    /// Generic discovery + scope source. `None` when the composed runtime
    /// has no durable installation store — only lane entries report then.
    installation_store: Option<Arc<dyn ExtensionInstallationStore>>,
    identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    /// Genuinely optional: compositions without product auth cannot have
    /// minted a personal vendor credential in the first place, so there is
    /// nothing to revoke on disconnect.
    credential_cleanup: Option<Arc<dyn ChannelCredentialCleanup>>,
    /// Read-side per-caller credential-account status, so the extensions wire
    /// can project `expired` / `disconnected` (with a typed reason) rather than
    /// the connected/disconnected collapse. `None` when product auth is not
    /// composed; the wire then falls back to the identity-binding bool.
    account_status_reader: Option<Arc<dyn ChannelAccountStatusReader>>,
    /// Generic DM-target store: discovered entries get a disconnect cleanup
    /// that drops the caller's provisioned DM target. `None` when the
    /// composed runtime carries no durable channel storage.
    dm_target_store: Option<Arc<FilesystemChannelDmTargetStore>>,
    /// Pairing services for `WebGeneratedCode` channels: their connected
    /// state and disconnect semantics (codes + DM target + conversation-actor
    /// cleanup) are owned by the pairing service, not the OAuth lane.
    channel_pairing: Option<Arc<crate::extension_host::channel_pairing::ChannelPairingRegistry>>,
}

impl GenericChannelConnectionFacade {
    // arch-exempt: too_many_args, needs a ChannelConnectionFacadeDeps bundle for the distinct discovery/identity/cleanup/status ports, plan #5905
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        tenant_id: TenantId,
        entries: Vec<ChannelConnectionEntry>,
        installation_store: Option<Arc<dyn ExtensionInstallationStore>>,
        identity_lookup: Arc<dyn RebornUserIdentityLookup>,
        identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
        credential_cleanup: Option<Arc<dyn ChannelCredentialCleanup>>,
        account_status_reader: Option<Arc<dyn ChannelAccountStatusReader>>,
        dm_target_store: Option<Arc<FilesystemChannelDmTargetStore>>,
        channel_pairing: Option<
            Arc<crate::extension_host::channel_pairing::ChannelPairingRegistry>,
        >,
    ) -> Self {
        Self {
            tenant_id,
            entries,
            installation_store,
            identity_lookup,
            identity_delete_store,
            credential_cleanup,
            account_status_reader,
            dm_target_store,
            channel_pairing,
        }
    }

    fn pairing_service_for(
        &self,
        extension_id: &str,
    ) -> Option<Arc<crate::extension_host::channel_pairing::ChannelPairingService>> {
        self.channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(extension_id))
    }

    /// The lane entries plus generically-discovered channel extensions.
    async fn connection_entries(&self) -> Result<Vec<ChannelConnectionEntry>, ProductSurfaceError> {
        let mut entries = self.entries.clone();
        let Some(installation_store) = &self.installation_store else {
            return Ok(entries);
        };
        let overridden: BTreeSet<String> = entries
            .iter()
            .map(|entry| entry.extension_id.clone())
            .collect();
        let discovered = discover_channel_extensions(installation_store, &overridden)
            .await
            .map_err(ProductSurfaceError::internal_from)?;
        for extension in discovered {
            let Ok(extension_id) = ExtensionId::new(&extension.extension_id) else {
                continue;
            };
            let disconnect_cleanup = self.dm_target_store.as_ref().map(|store| {
                Arc::new(ChannelDmTargetDisconnectCleanup {
                    extension_id: extension.extension_id.clone(),
                    store: Arc::clone(store),
                }) as Arc<dyn ChannelDisconnectCleanup>
            });
            entries.push(ChannelConnectionEntry {
                extension_id: extension.extension_id,
                providers: extension.providers,
                scope_source: channel_config_connection_scope_source(
                    Arc::clone(installation_store),
                    extension_id,
                    None,
                ),
                disconnect_cleanup,
            });
        }
        Ok(entries)
    }

    async fn entry_scope(
        &self,
        entry: &ChannelConnectionEntry,
    ) -> Result<Option<ChannelConnectionScope>, ProductSurfaceError> {
        entry
            .scope_source
            .resolve_connection_scope()
            .await
            .map_err(ProductSurfaceError::internal_from)
    }

    async fn caller_connected(
        &self,
        entry: &ChannelConnectionEntry,
        caller: &ProductSurfaceCaller,
        scope: &ChannelConnectionScope,
    ) -> Result<bool, ProductSurfaceError> {
        let prefix = scope.provider_user_id_prefix();
        for provider in &entry.providers {
            let connected = self
                .identity_lookup
                .user_has_provider_binding_with_provider_user_id_prefix(
                    provider,
                    &caller.user_id,
                    Some(prefix.as_str()),
                )
                .await
                .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
            if connected {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn revoke_personal_credentials(
        &self,
        entry: &ChannelConnectionEntry,
        caller: &ProductSurfaceCaller,
    ) -> Result<(), ProductSurfaceError> {
        let Some(cleanup) = &self.credential_cleanup else {
            return Ok(());
        };
        for provider in &entry.providers {
            cleanup
                .cleanup_credentials_for_lifecycle(personal_credential_cleanup_request(
                    caller,
                    &entry.extension_id,
                    provider,
                )?)
                .await?;
        }
        Ok(())
    }

    async fn delete_identity_bindings(
        &self,
        entry: &ChannelConnectionEntry,
        caller: &ProductSurfaceCaller,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<(), ProductSurfaceError> {
        for provider in &entry.providers {
            self.identity_delete_store
                .delete_user_identity_bindings_for_user(
                    provider,
                    &caller.user_id,
                    provider_user_id_prefix,
                )
                .await
                .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
        }
        Ok(())
    }
}

/// Generic disconnect cleanup for discovered channel extensions: drop the
/// caller's provisioned DM target so outbound targets no longer offer a
/// stale direct conversation after disconnect.
struct ChannelDmTargetDisconnectCleanup {
    extension_id: String,
    store: Arc<FilesystemChannelDmTargetStore>,
}

#[async_trait]
impl ChannelDisconnectCleanup for ChannelDmTargetDisconnectCleanup {
    async fn cleanup_disconnected_caller(
        &self,
        caller: &ProductSurfaceCaller,
        _scope: &ChannelConnectionScope,
    ) -> Result<(), String> {
        self.store
            .delete(&self.extension_id, &caller.user_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl ChannelConnectionFacade for GenericChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<HashMap<String, bool>, ProductSurfaceError> {
        let entries = self.connection_entries().await?;
        let mut connections = HashMap::with_capacity(entries.len());
        for entry in &entries {
            let connected = if caller.tenant_id != self.tenant_id {
                false
            } else if let Some(pairing) = self.pairing_service_for(&entry.extension_id) {
                pairing
                    .status_for(&caller.user_id)
                    .await
                    .map_err(|error| {
                        ProductSurfaceError::internal_from(format!(
                            "channel pairing status unavailable: {error}"
                        ))
                    })?
                    .connected
            } else {
                match self.entry_scope(entry).await? {
                    Some(scope) => self.caller_connected(entry, &caller, &scope).await?,
                    None => false,
                }
            };
            connections.insert(entry.extension_id.clone(), connected);
        }
        Ok(connections)
    }

    async fn caller_channel_account_states(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<HashMap<String, ChannelAuthAccountState>, ProductSurfaceError> {
        let Some(reader) = &self.account_status_reader else {
            return Ok(HashMap::new());
        };
        if caller.tenant_id != self.tenant_id {
            return Ok(HashMap::new());
        }
        let entries = self.connection_entries().await?;
        let mut states = HashMap::new();
        for entry in &entries {
            // The first provider whose account the caller holds decides the
            // vendor account state (length ≤ 1 today). `active_flow_status`
            // stays `None`: the mid-flow `authenticating` signal is projected
            // from thread-scoped setup flows owned by the auth-flow read model,
            // not this per-caller connection facade.
            let mut account_status = None;
            for provider in &entry.providers {
                if let Some(status) = reader.account_status_for_caller(&caller, provider).await? {
                    account_status = Some(status);
                    break;
                }
            }
            if let Some(account_status) = account_status {
                states.insert(
                    entry.extension_id.clone(),
                    ChannelAuthAccountState {
                        account_status: Some(account_status),
                        active_flow_status: None,
                    },
                );
            }
        }
        Ok(states)
    }

    async fn disconnect_channel_for_caller(
        &self,
        caller: ProductSurfaceCaller,
        channel: &str,
    ) -> Result<(), ProductSurfaceError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(());
        }
        let entries = self.connection_entries().await?;
        let Some(entry) = entries.iter().find(|entry| entry.extension_id == channel) else {
            return Ok(());
        };
        if let Some(pairing) = self.pairing_service_for(&entry.extension_id) {
            // Pairing-owned disconnect: pending codes, identity bindings, the
            // DM target, and conversation-actor pairings drop together. The
            // generic lane below then no-ops (no vendor credential, and the
            // bindings are already gone) but stays for defense in depth.
            pairing.unpair(&caller.user_id).await.map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "channel pairing disconnect failed: {error}"
                ))
            })?;
        }
        let Some(scope) = self.entry_scope(entry).await? else {
            // No connection scope means there is no installation to key the
            // vendor cleanup or prefix-scoped binding deletes — the state of
            // a fresh instance, or one whose setup was deleted. Refusing
            // here used to fail extension uninstall before the channel was
            // ever configured. Instead: still revoke the caller's
            // provider-scoped credentials, then drop the caller's own
            // bindings without an installation prefix (the delete stays
            // tenant + caller-user bound). Vendor cleanup is skipped — its
            // records are keyed by installation and unreachable while no
            // scope exists.
            self.revoke_personal_credentials(entry, &caller).await?;
            self.delete_identity_bindings(entry, &caller, None).await?;
            return Ok(());
        };
        // Ordering: credential revoke → vendor cleanup → identity binding.
        // The binding is the "connected" signal and deletes last (commit
        // point); the credential revokes first so a mid-sequence failure
        // leaves the caller visibly connected with every step retryable —
        // deleting vendor delivery state before a failing revoke would
        // silently break proactive delivery while the UI still shows
        // connected.
        self.revoke_personal_credentials(entry, &caller).await?;
        if let Some(cleanup) = &entry.disconnect_cleanup {
            cleanup
                .cleanup_disconnected_caller(&caller, &scope)
                .await
                .map_err(ProductSurfaceError::internal_from)?;
        }
        let prefix = scope.provider_user_id_prefix();
        self.delete_identity_bindings(entry, &caller, Some(prefix.as_str()))
            .await?;
        Ok(())
    }
}

// OAuth-minted personal credentials carry no extension ownership/grants, so
// the provider selector is what actually reaches the caller's personal
// vendor account. Shared by the scoped and no-scope disconnect arms so the
// revoke request cannot drift between them.
fn personal_credential_cleanup_request(
    caller: &ProductSurfaceCaller,
    extension_id: &str,
    provider: &str,
) -> Result<SecretCleanupRequest, ProductSurfaceError> {
    Ok(SecretCleanupRequest {
        scope: AuthProductScope::new(
            ResourceScope {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
                agent_id: caller.agent_id.clone(),
                project_id: caller.project_id.clone(),
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        ),
        extension_id: ExtensionId::new(extension_id)
            .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?,
        provider: Some(
            AuthProviderId::new(provider)
                .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?,
        ),
        lifecycle_package: None,
        action: SecretCleanupAction::Uninstall,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ironclaw_host_api::{AgentId, UserId};
    use ironclaw_product::AdapterInstallationId;

    use super::*;
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;
    use crate::provider_identity::{
        RebornUserIdentityBindingError, RebornUserIdentityLookupError,
        installation_scoped_provider_user_id,
    };

    const VENDOR: &str = "acmechat";
    const EXTENSION: &str = "acmechat";

    fn tenant() -> TenantId {
        TenantId::new("tenant:test").expect("tenant")
    }

    fn caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            tenant(),
            UserId::new("user:alice").expect("user"),
            None::<AgentId>,
            None,
        )
    }

    fn scope(installation: &str) -> ChannelConnectionScope {
        ChannelConnectionScope {
            installation_id: AdapterInstallationId::new(installation).expect("installation"),
            expected_team_id: Some("T123".to_string()),
            expected_enterprise_id: None,
            expected_app_id: Some("A123".to_string()),
        }
    }

    fn facade(
        scope: Option<ChannelConnectionScope>,
        identity_store: Arc<RecordingIdentityStore>,
        disconnect_cleanup: Option<Arc<dyn ChannelDisconnectCleanup>>,
        credential_cleanup: Option<Arc<dyn ChannelCredentialCleanup>>,
    ) -> GenericChannelConnectionFacade {
        GenericChannelConnectionFacade::new(
            tenant(),
            vec![ChannelConnectionEntry {
                extension_id: EXTENSION.to_string(),
                providers: vec![VENDOR.to_string()],
                scope_source: Arc::new(StaticScopeSource(scope)),
                disconnect_cleanup,
            }],
            None,
            identity_store.clone(),
            identity_store,
            credential_cleanup,
            None,
            None,
            None,
        )
    }

    fn bound_identity_store(installation: &str) -> Arc<RecordingIdentityStore> {
        let installation_id = AdapterInstallationId::new(installation).expect("installation");
        Arc::new(RecordingIdentityStore::new([(
            installation_scoped_provider_user_id(&installation_id, "U123"),
            UserId::new("user:alice").expect("user"),
        )]))
    }

    #[tokio::test]
    async fn facade_disconnects_identity_and_vendor_state_in_order() {
        let identity_store = bound_identity_store("install-alpha");
        let vendor_cleanup = Arc::new(RecordingDisconnectCleanup::default());
        let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
        let facade = facade(
            Some(scope("install-alpha")),
            identity_store.clone(),
            Some(vendor_cleanup.clone()),
            Some(credential_cleanup.clone()),
        );
        let caller = caller();

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([(EXTENSION.to_string(), true)])
        );

        facade
            .disconnect_channel_for_caller(caller.clone(), EXTENSION)
            .await
            .expect("disconnect succeeds");

        // Disconnect must revoke the caller's personal credential through
        // the product-auth lifecycle cleanup port, scoped to exactly this
        // tenant + caller, the extension, and its vendor.
        let requests = credential_cleanup.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].extension_id.as_str(), EXTENSION);
        assert_eq!(
            requests[0].provider.as_ref().map(|p| p.as_str()),
            Some(VENDOR),
            "the provider selector is what reaches the grant-less OAuth account"
        );
        assert_eq!(requests[0].action, SecretCleanupAction::Uninstall);
        assert_eq!(&requests[0].scope.resource.tenant_id, &tenant());
        assert_eq!(&requests[0].scope.resource.user_id, &caller.user_id);

        assert_eq!(
            vendor_cleanup.calls(),
            vec![(caller.user_id.clone(), "install-alpha".to_string())],
            "vendor disconnect cleanup runs with the resolved scope"
        );
        assert_eq!(
            identity_store.deletes(),
            vec![(
                VENDOR.to_string(),
                caller.user_id.clone(),
                Some("install-alpha:".to_string())
            )]
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup after disconnect"),
            HashMap::from([(EXTENSION.to_string(), false)])
        );

        // Retry convergence for extension removal: `remove_extension` runs
        // the caller disconnect before `ExtensionRemove`, so a failed
        // removal retries the disconnect for a caller who is already
        // disconnected. That repeat disconnect must stay an idempotent
        // no-op success, not an error that would wedge the removal retry.
        facade
            .disconnect_channel_for_caller(caller.clone(), EXTENSION)
            .await
            .expect("repeat disconnect for a disconnected caller is an idempotent no-op");
        assert_eq!(
            credential_cleanup.requests().len(),
            2,
            "the removal-retry repeat disconnect re-issues the (idempotent) credential cleanup"
        );
    }

    #[tokio::test]
    async fn facade_keeps_identity_when_vendor_cleanup_fails() {
        let identity_store = bound_identity_store("install-alpha");
        let facade = facade(
            Some(scope("install-alpha")),
            identity_store.clone(),
            Some(Arc::new(FailingDisconnectCleanup)),
            None,
        );
        let caller = caller();

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), EXTENSION)
                .await
                .is_err(),
            "vendor cleanup failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after failed disconnect"),
            HashMap::from([(EXTENSION.to_string(), true)]),
            "identity binding must remain until vendor cleanup succeeds"
        );
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    #[tokio::test]
    async fn facade_keeps_identity_when_credential_cleanup_fails() {
        let identity_store = bound_identity_store("install-alpha");
        let facade = facade(
            Some(scope("install-alpha")),
            identity_store.clone(),
            None,
            Some(Arc::new(FailingCredentialCleanup)),
        );
        let caller = caller();

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), EXTENSION)
                .await
                .is_err(),
            "credential cleanup failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after failed disconnect"),
            HashMap::from([(EXTENSION.to_string(), true)]),
            "identity binding must remain until credential cleanup succeeds, so the removal retry re-runs the full disconnect"
        );
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    #[tokio::test]
    async fn facade_requires_current_installation_scope_for_connected() {
        // A binding under a different installation than the current scope
        // must not report connected.
        let identity_store = bound_identity_store("install-beta");
        let facade = facade(Some(scope("install-alpha")), identity_store, None, None);

        assert_eq!(
            facade
                .caller_channel_connections(caller())
                .await
                .expect("connection lookup"),
            HashMap::from([(EXTENSION.to_string(), false)])
        );
    }

    #[tokio::test]
    async fn facade_disconnects_without_a_connection_scope() {
        // A fresh instance (or one whose setup was deleted) has no
        // connection scope. Uninstall/disconnect must still succeed and
        // clean the caller's own bindings without an installation prefix
        // while staying caller-bound; vendor cleanup is skipped (its records
        // are keyed by installation and unreachable).
        let identity_store = bound_identity_store("install-alpha");
        let vendor_cleanup = Arc::new(RecordingDisconnectCleanup::default());
        let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
        let facade = facade(
            None,
            identity_store.clone(),
            Some(vendor_cleanup.clone()),
            Some(credential_cleanup.clone()),
        );
        let caller = caller();

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([(EXTENSION.to_string(), false)])
        );
        facade
            .disconnect_channel_for_caller(caller.clone(), EXTENSION)
            .await
            .expect("disconnect succeeds without a connection scope");
        assert_eq!(
            credential_cleanup.requests().len(),
            1,
            "no-scope disconnect must still revoke the caller's credential"
        );
        assert!(vendor_cleanup.calls().is_empty());
        assert_eq!(
            identity_store.deletes(),
            vec![(VENDOR.to_string(), caller.user_id, None)],
            "caller's bindings are cleaned without an installation prefix"
        );
    }

    #[tokio::test]
    async fn facade_ignores_foreign_tenants_and_unknown_channels() {
        let identity_store = bound_identity_store("install-alpha");
        let credential_cleanup = Arc::new(RecordingCredentialCleanup::default());
        let facade = facade(
            Some(scope("install-alpha")),
            identity_store.clone(),
            None,
            Some(credential_cleanup.clone()),
        );
        let foreign_caller = ProductSurfaceCaller::new(
            TenantId::new("tenant:other").expect("tenant"),
            UserId::new("user:alice").expect("user"),
            None::<AgentId>,
            None,
        );

        assert_eq!(
            facade
                .caller_channel_connections(foreign_caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([(EXTENSION.to_string(), false)])
        );
        facade
            .disconnect_channel_for_caller(foreign_caller, EXTENSION)
            .await
            .expect("foreign tenant disconnect is a no-op");
        facade
            .disconnect_channel_for_caller(caller(), "unknown-channel")
            .await
            .expect("unknown channel disconnect is a no-op");
        assert!(credential_cleanup.requests().is_empty());
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    /// The generic facade projects the caller's durable credential-account
    /// status per vendor through the injected reader, so the extensions wire
    /// can render `expired` instead of the connected/disconnected collapse.
    /// A foreign tenant gets no account states (fail-closed).
    #[tokio::test]
    async fn facade_projects_caller_account_status_per_vendor() {
        let identity_store = bound_identity_store("install-alpha");
        let reader = Arc::new(RecordingAccountStatusReader::new(Some(
            CredentialAccountStatus::RefreshFailed,
        )));
        let facade = GenericChannelConnectionFacade::new(
            tenant(),
            vec![ChannelConnectionEntry {
                extension_id: EXTENSION.to_string(),
                providers: vec![VENDOR.to_string()],
                scope_source: Arc::new(StaticScopeSource(Some(scope("install-alpha")))),
                disconnect_cleanup: None,
            }],
            None,
            identity_store.clone(),
            identity_store,
            None,
            Some(reader.clone()),
            None,
            None,
        );

        let states = facade
            .caller_channel_account_states(caller())
            .await
            .expect("account states");
        let state = states
            .get(EXTENSION)
            .expect("vendor account state projected");
        assert_eq!(
            state.account_status,
            Some(CredentialAccountStatus::RefreshFailed),
            "the caller's real durable status must reach the wire, not the connection bool",
        );
        assert_eq!(state.active_flow_status, None);
        assert_eq!(
            reader.calls(),
            vec![(caller().user_id, VENDOR.to_string())],
            "the reader was consulted for the caller + vendor",
        );

        let foreign = ProductSurfaceCaller::new(
            TenantId::new("tenant:other").expect("tenant"),
            UserId::new("user:alice").expect("user"),
            None::<AgentId>,
            None,
        );
        assert!(
            facade
                .caller_channel_account_states(foreign)
                .await
                .expect("account states")
                .is_empty(),
            "a foreign tenant gets no account states",
        );
    }

    struct RecordingAccountStatusReader {
        status: Option<CredentialAccountStatus>,
        calls: Mutex<Vec<(UserId, String)>>,
    }

    impl RecordingAccountStatusReader {
        fn new(status: Option<CredentialAccountStatus>) -> Self {
            Self {
                status,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(UserId, String)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl ChannelAccountStatusReader for RecordingAccountStatusReader {
        async fn account_status_for_caller(
            &self,
            caller: &ProductSurfaceCaller,
            provider: &str,
        ) -> Result<Option<CredentialAccountStatus>, ProductSurfaceError> {
            self.calls
                .lock()
                .expect("lock")
                .push((caller.user_id.clone(), provider.to_string()));
            Ok(self.status)
        }
    }

    /// Discovered-extension disconnect: the generic DM-target cleanup drops
    /// the caller's provisioned direct-conversation record between the
    /// credential revoke and the binding delete.
    #[tokio::test]
    async fn discovered_extension_disconnect_drops_the_callers_dm_target() {
        use ironclaw_extensions::{
            ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
            ExtensionManifestRecord, ExtensionManifestRef, ManifestSource,
        };
        use ironclaw_filesystem::InMemoryBackend;

        const DISCOVERED_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acmechat"
name = "AcmeChat"
version = "0.1.0"
description = "discovered disconnect fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

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

[channel.config]
fields = [
  { handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true },
  { handle = "acmechat_team_id", label = "Workspace ID", secret = false },
]

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
"#;

        let installation_store =
            Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        let record = ExtensionManifestRecord::from_toml(
            DISCOVERED_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let extension_id = ExtensionId::new(EXTENSION).expect("extension id");
        installation_store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new("install-alpha".to_string())
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(extension_id.clone(), None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extensions::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
        // Connection scoping is configured (fail-closed otherwise).
        installation_store
            .set_channel_config(
                &extension_id,
                vec![("acmechat_team_id".to_string(), "T123".to_string())],
            )
            .await
            .expect("save scoping value");

        let identity_store = bound_identity_store("install-alpha");
        let dm_store = Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::new(InMemoryBackend::new()),
            tenant(),
            UserId::new("user:operator").expect("user"),
        ));
        let caller = caller();
        dm_store
            .upsert(
                EXTENSION,
                &caller.user_id,
                "U123".to_string(),
                ironclaw_extension_host::dm_target_payload(Some("T123"), "DM-9"),
            )
            .await
            .expect("seed DM target");

        let facade = GenericChannelConnectionFacade::new(
            tenant(),
            Vec::new(),
            Some(installation_store as Arc<dyn ExtensionInstallationStore>),
            identity_store.clone(),
            identity_store.clone(),
            None,
            None,
            Some(Arc::clone(&dm_store)),
            None,
        );

        // Discovered + bound: connected.
        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([(EXTENSION.to_string(), true)])
        );

        facade
            .disconnect_channel_for_caller(caller.clone(), EXTENSION)
            .await
            .expect("disconnect succeeds");

        assert!(
            dm_store
                .load(EXTENSION, &caller.user_id)
                .await
                .expect("load")
                .is_none(),
            "disconnect must drop the caller's provisioned DM target"
        );
        assert_eq!(
            identity_store.deletes(),
            vec![(
                VENDOR.to_string(),
                caller.user_id,
                Some("install-alpha:".to_string())
            )],
            "bindings delete last, prefix-scoped to the installation"
        );
    }

    /// Proof-code channels have no auth vendor, but the connection facade
    /// still has to discover them so its pairing registry can own status and
    /// disconnect. This mirrors Telegram's manifest shape without naming a
    /// provider in production code.
    #[tokio::test]
    async fn connection_discovery_includes_channel_without_auth_vendor() {
        use ironclaw_extensions::{
            ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
            ExtensionManifestRecord, ExtensionManifestRef, ManifestSource,
        };

        const PAIRING_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "pairchat"
name = "PairChat"
version = "0.1.0"
description = "proof-code channel discovery fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "pairchat.extension/v1"

[channel]
id = "messages"
display_name = "PairChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "updates"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "pairchat_webhook_secret"
header = "X-PairChat-Secret"

[channel.config]
fields = [
  { handle = "pairchat_bot_token", label = "Bot token", secret = true },
  { handle = "pairchat_webhook_secret", label = "Webhook secret", secret = true },
]

[[channel.egress]]
scheme = "https"
host = "api.pairchat.example"
methods = ["post"]
credential_handle = "pairchat_bot_token"
injection = { type = "header", name = "authorization", prefix = "Bearer " }
"#;

        let installation_store =
            Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        let record = ExtensionManifestRecord::from_toml(
            PAIRING_CHANNEL_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("pairing channel manifest parses");
        let extension_id = ExtensionId::new("pairchat").expect("extension id");
        installation_store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new("pairchat-install").expect("installation id"),
                    extension_id.clone(),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extensions::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");

        let identity_store = bound_identity_store("pairchat-install");
        let facade = GenericChannelConnectionFacade::new(
            tenant(),
            Vec::new(),
            Some(installation_store as Arc<dyn ExtensionInstallationStore>),
            identity_store.clone(),
            identity_store,
            None,
            None,
            None,
            None,
        );

        let entries = facade
            .connection_entries()
            .await
            .expect("discover channels");
        let pairchat = entries
            .iter()
            .find(|entry| entry.extension_id == "pairchat")
            .expect("channel-only extension is discoverable");
        assert!(
            pairchat.providers.is_empty(),
            "proof-code pairing does not invent an OAuth vendor"
        );
    }

    struct StaticScopeSource(Option<ChannelConnectionScope>);

    #[async_trait]
    impl ChannelConnectionScopeSource for StaticScopeSource {
        async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String> {
            Ok(self.0.clone())
        }
    }

    #[derive(Default)]
    struct RecordingDisconnectCleanup {
        calls: Mutex<Vec<(UserId, String)>>,
    }

    impl RecordingDisconnectCleanup {
        fn calls(&self) -> Vec<(UserId, String)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl ChannelDisconnectCleanup for RecordingDisconnectCleanup {
        async fn cleanup_disconnected_caller(
            &self,
            caller: &ProductSurfaceCaller,
            scope: &ChannelConnectionScope,
        ) -> Result<(), String> {
            self.calls.lock().expect("lock").push((
                caller.user_id.clone(),
                scope.installation_id.as_str().to_string(),
            ));
            Ok(())
        }
    }

    struct FailingDisconnectCleanup;

    #[async_trait]
    impl ChannelDisconnectCleanup for FailingDisconnectCleanup {
        async fn cleanup_disconnected_caller(
            &self,
            _caller: &ProductSurfaceCaller,
            _scope: &ChannelConnectionScope,
        ) -> Result<(), String> {
            Err("vendor cleanup unavailable".to_string())
        }
    }

    #[derive(Default)]
    struct RecordingCredentialCleanup {
        requests: Mutex<Vec<SecretCleanupRequest>>,
    }

    impl RecordingCredentialCleanup {
        fn requests(&self) -> Vec<SecretCleanupRequest> {
            self.requests.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl ChannelCredentialCleanup for RecordingCredentialCleanup {
        async fn cleanup_credentials_for_lifecycle(
            &self,
            request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, ProductSurfaceError> {
            self.requests.lock().expect("lock").push(request);
            Ok(SecretCleanupReport::default())
        }
    }

    struct FailingCredentialCleanup;

    #[async_trait]
    impl ChannelCredentialCleanup for FailingCredentialCleanup {
        async fn cleanup_credentials_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, ProductSurfaceError> {
            Err(ProductSurfaceError::internal_from(
                "credential cleanup unavailable",
            ))
        }
    }

    #[derive(Default)]
    struct RecordingIdentityStore {
        bindings: Mutex<HashMap<String, UserId>>,
        deletes: Mutex<Vec<(String, UserId, Option<String>)>>,
    }

    impl RecordingIdentityStore {
        fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
            Self {
                bindings: Mutex::new(bindings.into_iter().collect()),
                deletes: Mutex::new(Vec::new()),
            }
        }

        fn deletes(&self) -> Vec<(String, UserId, Option<String>)> {
            self.deletes.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl RebornUserIdentityLookup for RecordingIdentityStore {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            if provider != VENDOR {
                return Ok(None);
            }
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .get(provider_user_id)
                .cloned())
        }

        async fn user_has_provider_binding(
            &self,
            provider: &str,
            user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
                .await
        }

        async fn user_has_provider_binding_with_provider_user_id_prefix(
            &self,
            provider: &str,
            user_id: &UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            if provider != VENDOR {
                return Ok(false);
            }
            Ok(self.bindings.lock().expect("lock").iter().any(
                |(provider_user_id, bound_user_id)| {
                    bound_user_id == user_id
                        && provider_user_id_prefix
                            .map(|prefix| provider_user_id.starts_with(prefix))
                            .unwrap_or(true)
                },
            ))
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
            bindings.retain(|provider_user_id, bound_user_id| {
                let prefix_matches = provider_user_id_prefix
                    .map(|prefix| provider_user_id.starts_with(prefix))
                    .unwrap_or(true);
                !(bound_user_id == user_id && prefix_matches)
            });
            Ok(before - bindings.len())
        }
    }
}
