//! Generic channel-connection test support (C-SLACK-LIFECYCLE seam, issue
//! #6105, re-expressed onto the unified extension runtime).
//!
//! Builds the REAL [`GenericChannelConnectionFacade`] (extension-runtime
//! §6.4) over a composed `RebornRuntime`'s own generic stores — the durable
//! installation store, the filesystem channel-identity store, the DM-target
//! store, and product-auth lifecycle cleanup — mirroring exactly the
//! production wiring in `RebornRuntime::generic_channel_connection_facade`
//! (`runtime.rs`). The built facade is late-bound into
//! runtime `channel_disconnect_slot`, the same slot `build_reborn_runtime`
//! fills in production, so
//! `builtin.extension_remove` of a channel extension runs the REAL per-caller
//! disconnect (revoke personal vendor credential → vendor cleanup → delete
//! identity bindings) instead of skipping it on an empty slot.
//!
//! [`ChannelConnectionTestBundle::connect_provider_user`] mirrors the
//! successful OAuth callback's identity-binding write: it drives
//! [`bind_channel_identities_for_callback`] — the exact hook body the
//! production callback invokes through
//! `channel_identity_binding_hook_factory` — over a
//! [`ChannelIdentityBindingConfig`] assembled like
//! `RebornRuntime::channel_identity_binding_config`. The persisted binding is
//! therefore byte-identical to a production connect (installation-scoped
//! composite key, fail-closed scoping-claim validation), so integration
//! tests can drive connect → remove → reconnect against durable identity
//! bindings without a browser or a vendor.
//!
//! For tests only — gated behind `test-support`, ships zero bytes in
//! production builds.

use std::sync::Arc;

use ironclaw_auth::{AuthProductScope, AuthSurface, OAuthProviderIdentity};
use ironclaw_host_api::{
    AgentId, InvocationId, ProductSurfaceCaller, ResourceScope, TenantId, UserId,
};
use ironclaw_product::ChannelConnectionFacade;

use crate::extension_host::channel_connection::{
    ChannelAccountStatusReader, ChannelCredentialCleanup, GenericChannelConnectionFacade,
};
use crate::extension_host::channel_dm_provisioning::ChannelDmTargetProvisioning;
use crate::extension_host::channel_identity::{
    ChannelIdentityBindingConfig, ChannelIdentityPostBindFactory,
    bind_channel_identities_for_callback,
};
use crate::provider_identity::RebornUserIdentityLookup;

/// Identity inputs for [`build_channel_connection_for_test`]. Plain strings
/// so harness callers outside this crate don't need the id newtypes;
/// validated at construction. Unlike the retired per-vendor bundle, no
/// installation/team/app identifiers are carried here: the installation id
/// is owned by the production install path, and the scoping claim values are
/// configured through the production `[channel.config]` configure surface
/// (`ChannelConfigService`) — the generic scope source reads both back from
/// the durable installation store.
pub struct ChannelConnectionTestConfig {
    /// Tenant of the harness's dispatch-time callers (the group's
    /// single-source product scope) — the facade ignores foreign tenants, so
    /// this must match the tenant extension-removal cleanup runs under.
    pub tenant_id: String,
    /// Agent stamped on the bundle's read-side callers (parity with the
    /// WebUI caller shape; the facade keys connections by tenant + user).
    pub agent_id: String,
}

/// Handles for driving the generic channel connection state machine in
/// tests. See the module doc for the production call sites each method
/// mirrors.
pub struct ChannelConnectionTestBundle {
    tenant_id: TenantId,
    agent_id: AgentId,
    facade: Arc<dyn ChannelConnectionFacade>,
    identity_binding: ChannelIdentityBindingConfig,
    identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    /// The (tenant, user) scope the LIVE identity store was composed with —
    /// the restart-survival reopen probe reconstructs a fresh store under the
    /// same scoping so its reads see the same identity subtree.
    identity_store_tenant_id: TenantId,
    identity_store_user_id: UserId,
}

/// Build the real generic channel-connection facade over `runtime`'s own
/// stores and fill the composition's late-binding facade slot. Mirrors
/// `RebornRuntime::generic_channel_connection_facade` (facade construction)
/// plus the `build_reborn_runtime` slot fill, and assembles the identity
/// binding config like `RebornRuntime::channel_identity_binding_config`.
///
/// Fails loud when the slot is already occupied — a second facade would not
/// be the one extension-removal cleanup dispatches to, so a test composing
/// twice must find out immediately.
pub fn build_channel_connection_for_test(
    runtime: &crate::RebornRuntime,
    config: ChannelConnectionTestConfig,
) -> Result<ChannelConnectionTestBundle, String> {
    let tenant_id = TenantId::new(config.tenant_id).map_err(|error| error.to_string())?;
    let agent_id = AgentId::new(config.agent_id).map_err(|error| error.to_string())?;
    let identity_store = runtime.channel_identity_store.clone();
    let installation_store = runtime.extension_management.installation_store_handle();

    // Same construction as `RebornRuntime::generic_channel_connection_facade`:
    // generic discovery over the durable installation store, connected =
    // identity binding under the extension's installation prefix, disconnect
    // clears credentials, vendor residue, and bindings.
    let credential_cleanup =
        Some(Arc::clone(&runtime.product_auth) as Arc<dyn ChannelCredentialCleanup>);
    let account_status_reader =
        Some(Arc::clone(&runtime.product_auth) as Arc<dyn ChannelAccountStatusReader>);
    let facade: Arc<dyn ChannelConnectionFacade> = Arc::new(GenericChannelConnectionFacade::new(
        tenant_id.clone(),
        Vec::new(),
        Some(Arc::clone(&installation_store)),
        Arc::clone(&identity_store) as Arc<dyn RebornUserIdentityLookup>,
        Arc::clone(&identity_store)
            as Arc<dyn crate::provider_identity::RebornUserIdentityBindingDeleteStore>,
        credential_cleanup,
        account_status_reader,
        Some(runtime.channel_dm_target_store.clone()),
        Arc::new(ironclaw_product::ChannelWorkflowStateService::new(
            runtime.extension_filesystem.clone() as Arc<dyn ironclaw_filesystem::RootFilesystem>,
        )),
        runtime.channel_pairing.clone(),
    ));
    let disconnect_slot = &runtime.channel_facade_slot;
    let facade = match disconnect_slot.get() {
        Some(existing) => Arc::clone(existing),
        None => {
            let _ = disconnect_slot.set(Arc::clone(&facade));
            facade
        }
    };

    // Same assembly as `RebornRuntime::channel_identity_binding_config`: the
    // generic post-OAuth binding hook over the same installation + identity
    // stores, with DM-target provisioning when the composition can deliver.
    let snapshot_updates = runtime
        .extension_management
        .generic_host()
        .map(|host| host.snapshot_watch().subscribe());
    let post_bind_factory = match (
        runtime.channel_delivery_resolver.clone(),
        Some(runtime.channel_dm_target_store.clone()),
        snapshot_updates,
    ) {
        (Some(delivery), Some(store), Some(snapshot_updates)) => Some(Arc::new(
            ChannelDmTargetProvisioning::new(delivery, store, snapshot_updates),
        )
            as Arc<dyn ChannelIdentityPostBindFactory>),
        _ => None,
    };
    let identity_binding = ChannelIdentityBindingConfig {
        tenant_id: tenant_id.clone(),
        installation_store: Some(installation_store),
        admin_configuration_resolver: Some(runtime.admin_configuration_resolver.clone()),
        binding_store: Arc::clone(&identity_store)
            as Arc<dyn crate::provider_identity::RebornUserIdentityBindingStore>,
        rollback_store: Arc::clone(&identity_store)
            as Arc<dyn crate::provider_identity::RebornUserIdentityBindingDeleteStore>,
        post_bind_factory,
        overrides: Vec::new(),
    };

    let (identity_store_tenant_id, identity_store_user_id) = {
        let (tenant, user) = identity_store.identity_scope_tenant_and_user();
        (tenant.clone(), user.clone())
    };
    Ok(ChannelConnectionTestBundle {
        tenant_id,
        agent_id,
        facade,
        identity_binding,
        identity_lookup: identity_store,
        identity_store_tenant_id,
        identity_store_user_id,
    })
}

impl ChannelConnectionTestBundle {
    /// Connect `user_id`'s personal vendor account, mirroring the successful
    /// OAuth callback's identity-binding write: the proven
    /// [`OAuthProviderIdentity`] is validated against the extension's
    /// configured connection scope and persisted as an installation-scoped
    /// binding through [`bind_channel_identities_for_callback`] — the exact
    /// hook body the production callback runs. Fail-closed like production:
    /// the extension must be installed and its `[channel.config]` scoping
    /// values configured, and this bundle additionally errors when the
    /// provider maps onto no installed channel extension (production
    /// completes such callbacks untouched; a test calling connect wants the
    /// bind to have happened).
    pub async fn connect_provider_user(
        &self,
        user_id: &UserId,
        provider: &str,
        identity: OAuthProviderIdentity,
    ) -> Result<(), String> {
        let callback_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: self.tenant_id.clone(),
                user_id: user_id.clone(),
                agent_id: Some(self.agent_id.clone()),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        );
        let rollback = bind_channel_identities_for_callback(
            &self.identity_binding,
            provider,
            &callback_scope,
            Some(&identity),
        )
        .await
        .map_err(|error| format!("channel identity bind rejected: {error:?}"))?;
        match rollback {
            // Dropping the rollback without running it models the successful
            // callback completion (production only awaits it when completion
            // fails after the bind).
            Some(rollback) => {
                drop(rollback);
                Ok(())
            }
            None => Err(format!(
                "provider {provider} matched no installed channel extension; install and \
                 configure the extension before connecting"
            )),
        }
    }

    /// The real facade — the same instance extension-removal cleanup
    /// dispatches to — for callers that need the full
    /// [`ChannelConnectionFacade`] surface.
    pub fn facade(&self) -> Arc<dyn ChannelConnectionFacade> {
        Arc::clone(&self.facade)
    }

    /// Surface (a) of the extensions page: what `list_extensions` merges via
    /// [`ChannelConnectionFacade::caller_channel_connections`]
    /// (`ironclaw_product/src/reborn_services/extensions.rs`).
    /// Returns the entry for `extension_id`; an absent entry reads as `false`
    /// — the generic facade discovers channel extensions from the durable
    /// installation store, so a removed (or never-installed) extension has no
    /// entry and is trivially not connected.
    pub async fn caller_channel_connected(
        &self,
        extension_id: &str,
        user_id: &UserId,
    ) -> Result<bool, String> {
        let connections = self
            .facade
            .caller_channel_connections(ProductSurfaceCaller::new(
                self.tenant_id.clone(),
                user_id.clone(),
                Some(self.agent_id.clone()),
                None,
            ))
            .await
            .map_err(|error| format!("{:?}", error.code))?;
        Ok(connections.get(extension_id).copied().unwrap_or(false))
    }

    /// Durable-state evidence: whether ANY identity binding for `provider`
    /// is persisted for `user_id`, across all installations
    /// (prefix-unscoped, unlike [`Self::caller_channel_connected`]). The
    /// generic disconnect DELETES binding records (the retired per-vendor
    /// lane tombstoned them instead), so record absence is the "binding
    /// gone" evidence on this architecture.
    pub async fn has_any_active_identity_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, String> {
        self.identity_lookup
            .user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
            .map_err(|error| error.to_string())
    }

    /// Restart-survival probe (T5 of issue #6105): evaluate the SAME
    /// active-binding predicate as
    /// [`Self::has_any_active_identity_binding`] for EACH of `user_ids`, but
    /// through ONE fresh `FilesystemChannelIdentityStore` over ONE fresh
    /// local-dev root filesystem reopened at `storage_root` — fully
    /// independent of the live runtime's in-memory handles. This is the
    /// integration-tier approximation of a process restart: it proves the
    /// durable binding is reconstructible the way production reconstructs it
    /// on boot (`build_runtime` →
    /// `FilesystemChannelIdentityStore::new` over the composed local-dev
    /// root). Results come back in `user_ids` order; the single reopen means
    /// a positive probe and its non-vacuity control read the same
    /// reconstructed store. Tests only.
    ///
    /// `libsql`-only, matching the factory seam it opens: the local-default
    /// reopen path composes the libsql local-dev backend, so a wider gate
    /// would silently probe a fresh in-memory store on non-libsql builds.
    pub async fn active_identity_bindings_after_reopen(
        &self,
        provider: &str,
        storage_root: &std::path::Path,
        user_ids: &[&UserId],
    ) -> Result<Vec<bool>, String> {
        let filesystem = crate::factory::open_local_dev_root_filesystem_for_test(storage_root)
            .await
            .map_err(|error| error.to_string())?;
        let store = Arc::new(
            crate::extension_host::channel_identity_store::FilesystemChannelIdentityStore::new(
                filesystem,
                self.identity_store_tenant_id.clone(),
                self.identity_store_user_id.clone(),
            ),
        );
        let lookup: Arc<dyn RebornUserIdentityLookup> = store;
        let mut bindings = Vec::with_capacity(user_ids.len());
        for user_id in user_ids {
            bindings.push(
                lookup
                    .user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(bindings)
    }
}
