//! Generic per-user channel connection facade (extension-runtime §6.4).
//!
//! One vendor-blind [`ChannelConnectionFacade`] replaces the per-vendor
//! facades: every installed extension whose manifest declares a channel
//! surface is discovered. OAuth connection state is derived from the
//! identity-binding store; proof-code connection state comes from the generic
//! pairing registry. Disconnect runs the owner-specific cleanup before the
//! installation disappears: pairing cleanup → revoke any personal vendor
//! credential → per-extension residue → caller-owned conversation routes →
//! identity bindings. The binding is the OAuth lane's "connected" signal and
//! deletes last (commit point); a mid-sequence failure therefore leaves the
//! caller visibly connected with every remaining step retryable.
//!
//! Channel lanes whose configure surface predates manifest administrator configuration
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
use ironclaw_product::{
    ChannelAuthAccountState, ChannelConnectionFacade, ChannelDisconnectActions,
    ChannelPairingRegistry, ChannelPairingService, ChannelWorkflowStateService,
    disconnect_channel_in_order,
};

use crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore;
use crate::extension_host::channel_identity::{
    ChannelConnectionScope, ChannelConnectionScopeSource,
    admin_configuration_connection_scope_source, discover_channel_extensions,
};
use crate::provider_identity::{RebornUserIdentityBindingDeleteStore, RebornUserIdentityLookup};

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

/// One channel lane's registration with the generic facade: the extension
/// id, the auth vendors whose bindings mean "connected", and the lane's
/// scope source.
#[derive(Clone)]
pub(crate) struct ChannelConnectionEntry {
    pub(crate) extension_id: String,
    pub(crate) providers: Vec<String>,
    pub(crate) scope_source: Arc<dyn ChannelConnectionScopeSource>,
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
    /// Mandatory product-owned durable channel state. Disconnect must revoke
    /// every caller-owned direct route before deleting the identity binding;
    /// unavailable cleanup fails closed.
    workflow_state: Arc<ChannelWorkflowStateService>,
    /// Pairing services for `WebGeneratedCode` channels: their connected
    /// state and disconnect semantics (codes + DM target + conversation-actor
    /// cleanup) are owned by the pairing service, not the OAuth lane.
    channel_pairing: Option<Arc<ChannelPairingRegistry>>,
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
        workflow_state: Arc<ChannelWorkflowStateService>,
        channel_pairing: Option<Arc<ChannelPairingRegistry>>,
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
            workflow_state,
            channel_pairing,
        }
    }

    fn pairing_service_for(&self, extension_id: &str) -> Option<Arc<ChannelPairingService>> {
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
            entries.push(ChannelConnectionEntry {
                extension_id: extension.extension_id,
                providers: extension.providers,
                scope_source: admin_configuration_connection_scope_source(
                    Arc::clone(installation_store),
                    extension_id,
                    None,
                ),
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

/// Call-local adapter over composition-owned provider/auth ports. Product
/// workflow owns the sequence; this type only supplies each concrete action.
struct ChannelDisconnectOperations<'a> {
    facade: &'a GenericChannelConnectionFacade,
    entry: &'a ChannelConnectionEntry,
    caller: &'a ProductSurfaceCaller,
    scope: Option<&'a ChannelConnectionScope>,
}

#[async_trait]
impl ChannelDisconnectActions for ChannelDisconnectOperations<'_> {
    type Error = ProductSurfaceError;

    async fn disconnect_pairing(&self) -> Result<(), Self::Error> {
        let Some(pairing) = self.facade.pairing_service_for(&self.entry.extension_id) else {
            return Ok(());
        };
        pairing.unpair(&self.caller.user_id).await.map_err(|error| {
            ProductSurfaceError::internal_from(format!(
                "channel pairing disconnect failed: {error}"
            ))
        })
    }

    async fn revoke_personal_credentials(&self) -> Result<(), Self::Error> {
        self.facade
            .revoke_personal_credentials(self.entry, self.caller)
            .await
    }

    async fn cleanup_vendor_state(&self) -> Result<(), Self::Error> {
        if let Some(store) = &self.facade.dm_target_store {
            store
                .delete(&self.entry.extension_id, &self.caller.user_id)
                .await
                .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
        }
        Ok(())
    }

    async fn cleanup_conversation_bindings(&self) -> Result<(), Self::Error> {
        let extension_id = ExtensionId::new(&self.entry.extension_id)
            .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
        self.facade
            .workflow_state
            .cleanup_conversation_bindings(
                self.caller,
                &extension_id,
                self.scope.map(|scope| &scope.installation_id),
            )
            .await
            .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))
    }

    async fn delete_identity_bindings(&self) -> Result<(), Self::Error> {
        let prefix = self
            .scope
            .map(ChannelConnectionScope::provider_user_id_prefix);
        self.facade
            .delete_identity_bindings(self.entry, self.caller, prefix.as_deref())
            .await
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
            states.insert(
                entry.extension_id.clone(),
                ChannelAuthAccountState {
                    account_status,
                    active_flow_status: None,
                },
            );
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
        let scope = self.entry_scope(entry).await?;
        disconnect_channel_in_order(&ChannelDisconnectOperations {
            facade: self,
            entry,
            caller: &caller,
            scope: scope.as_ref(),
        })
        .await
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
mod tests;
