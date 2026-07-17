//! Slack channel-connection test support (C-SLACK-LIFECYCLE seam, issue #6105).
//!
//! Builds the REAL [`SlackChannelConnectionFacade`] over the composed
//! local-dev runtime's durable Slack host state and late-binds it into
//! `RebornLocalRuntimeServices::channel_connection_facade_slot`, mirroring the
//! production wiring in
//! `slack_connectable_channel::build_webui_services_with_slack_host_beta_mounts`
//! (facade construction + `set_channel_connection_facade`). With the slot
//! filled, `builtin.extension_remove` of the Slack extension runs the real
//! `SlackPersonalConnectionCleanupAdapter` → `disconnect_channel_for_caller`
//! path, exactly as a hosted deployment does.
//!
//! [`SlackChannelConnectionTestBundle::connect_personal_user`] mirrors the
//! successful `slack_personal` OAuth callback
//! (`slack_host_beta/runtime_setup.rs` — `begin_connection` then
//! `bind_personal_user_for_epoch_with_rollback`), so integration tests can
//! drive connect → disconnect → reconnect against durable identity bindings
//! without a browser or Slack.
//!
//! For tests only — gated behind `test-support`, ships zero bytes in
//! production builds.

use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_product_workflow::{ChannelConnectionFacade, WebUiAuthenticatedCaller};

use crate::factory::RebornServices;
use crate::slack::slack_actor_identity::{RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER};
use crate::slack::slack_channel_connection::{
    SlackChannelConnectionFacadeTestParts, SlackPersonalCredentialCleanup,
    slack_channel_connection_facade_from_test_parts,
};
use crate::slack::slack_host_beta::SlackPersonalConnectionScope;
use crate::slack::slack_host_state::FilesystemSlackHostState;
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingStore, SlackConnectionEpoch,
    SlackConnectionOwner, SlackPersonalBindingInstallation, SlackPersonalBindingPrincipal,
    SlackPersonalUserBindingRequest, SlackPersonalUserBindingService,
    SlackUserBindingLifecycleStore,
};
use crate::slack::slack_serve::{
    SlackApiAppId, SlackInstallationSelector, SlackTeamId, SlackUserId,
};

/// Identity inputs for [`build_slack_channel_connection_for_test`]. Plain
/// strings so harness callers outside this crate don't need the internal
/// Slack id newtypes; validated at construction.
pub struct SlackChannelConnectionTestConfig {
    pub tenant_id: String,
    /// Host/runtime user that owns the durable Slack host state
    /// (`SlackHostBetaConfig::user_id` in production).
    pub host_user_id: String,
    pub agent_id: String,
    pub installation_id: String,
    pub team_id: String,
    pub api_app_id: String,
}

/// Handles for driving the Slack personal connection state machine in tests.
/// See the module doc for the production call sites each method mirrors.
pub struct SlackChannelConnectionTestBundle {
    tenant_id: TenantId,
    host_user_id: UserId,
    agent_id: AgentId,
    installation_id: ironclaw_product_adapters::AdapterInstallationId,
    team_id: SlackTeamId,
    api_app_id: SlackApiAppId,
    facade: Arc<dyn ChannelConnectionFacade>,
    binding_service: SlackPersonalUserBindingService,
    lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore>,
    user_identity_lookup: Arc<dyn RebornUserIdentityLookup>,
}

/// Build the real Slack channel-connection facade over `services`' local-dev
/// Slack host state and fill the extension-lifecycle handler's late-binding
/// facade slot. Mirrors `build_webui_services_with_slack_host_beta_mounts`
/// (`slack_connectable_channel.rs`): same `FilesystemSlackHostState`
/// construction as `build_slack_host_beta_mounts`, same
/// `personal_credential_cleanup` sourced from `services.product_auth`, same
/// slot fill production performs via
/// `RebornRuntime::set_channel_connection_facade`.
///
/// Fails loud when the slot is already occupied — a second facade would not
/// be the one extension-removal cleanup dispatches to, so a test composing
/// twice must find out immediately.
pub fn build_slack_channel_connection_for_test(
    services: &RebornServices,
    config: SlackChannelConnectionTestConfig,
) -> Result<SlackChannelConnectionTestBundle, String> {
    let local_runtime = services
        .local_runtime
        .as_ref()
        .ok_or("slack channel-connection test support requires a local-dev runtime")?;
    let tenant_id = TenantId::new(config.tenant_id).map_err(|error| error.to_string())?;
    let host_user_id = UserId::new(config.host_user_id).map_err(|error| error.to_string())?;
    let agent_id = AgentId::new(config.agent_id).map_err(|error| error.to_string())?;
    let installation_id =
        ironclaw_product_adapters::AdapterInstallationId::new(config.installation_id)
            .map_err(|error| error.to_string())?;
    let team_id = SlackTeamId::new(config.team_id);
    let api_app_id = SlackApiAppId::new(config.api_app_id);

    // Same durable host-state construction as `build_slack_host_beta_mounts`
    // (`slack_host_beta.rs`), over the SAME `host_state_filesystem` the
    // production Slack host uses.
    let state = Arc::new(FilesystemSlackHostState::new(
        Arc::clone(&local_runtime.host_state_filesystem),
        tenant_id.clone(),
        host_user_id.clone(),
        agent_id.clone(),
        None,
    ));
    let binding_store: Arc<dyn RebornUserIdentityBindingStore> = state.clone();
    let binding_service = SlackPersonalUserBindingService::new(
        [SlackPersonalBindingInstallation {
            tenant_id: tenant_id.clone(),
            installation_id: installation_id.clone(),
            selector: SlackInstallationSelector::app_team(
                api_app_id.as_str().to_string(),
                team_id.as_str().to_string(),
            ),
        }],
        binding_store,
    );
    let lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore> = state.clone();
    let identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore> = state.clone();
    let user_identity_lookup: Arc<dyn RebornUserIdentityLookup> = state.clone();
    // Mirrors `build_webui_services_with_slack_host_beta_mounts`: the caller's
    // `slack_personal` credential is revoked through the same product-auth
    // lifecycle cleanup production disconnect uses.
    let personal_credential_cleanup = services
        .product_auth
        .clone()
        .map(|auth| auth as Arc<dyn SlackPersonalCredentialCleanup>);
    let facade =
        slack_channel_connection_facade_from_test_parts(SlackChannelConnectionFacadeTestParts {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: SlackPersonalConnectionScope {
                installation_id: installation_id.clone(),
            },
            user_identity_lookup: Arc::clone(&user_identity_lookup),
            user_identity_delete_store: identity_delete_store,
            user_binding_lifecycle_store: Arc::clone(&lifecycle_store),
            conversation_actor_pairings: Arc::new(
                ironclaw_conversations::InMemoryConversationServices::default(),
            ),
            personal_dm_target_store: state,
            personal_credential_cleanup,
        });
    if local_runtime
        .channel_connection_facade_slot
        .set(Arc::clone(&facade))
        .is_err()
    {
        return Err(
            "channel connection facade slot is already occupied; extension-removal cleanup \
             would dispatch to a different facade than this bundle"
                .to_string(),
        );
    }
    Ok(SlackChannelConnectionTestBundle {
        tenant_id,
        host_user_id,
        agent_id,
        installation_id,
        team_id,
        api_app_id,
        facade,
        binding_service,
        lifecycle_store,
        user_identity_lookup,
    })
}

impl SlackChannelConnectionTestBundle {
    /// Connect `user_id`'s personal Slack account, mirroring the successful
    /// `slack_personal` OAuth callback: `begin_connection` on the durable
    /// lifecycle store, then the same
    /// `bind_personal_user_for_epoch_with_rollback` the production callback
    /// identity hook calls (`slack_host_beta/runtime_setup.rs`). A fresh
    /// connection epoch is minted per call, so calling again after a
    /// disconnect models a real reconnect.
    pub async fn connect_personal_user(
        &self,
        user_id: &UserId,
        slack_user_id: &str,
    ) -> Result<(), String> {
        let epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        self.lifecycle_store
            .begin_connection(
                &SlackConnectionOwner::new(
                    self.tenant_id.clone(),
                    user_id.clone(),
                    self.installation_id.clone(),
                ),
                epoch,
                chrono::Utc::now() + chrono::Duration::minutes(5),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.binding_service
            .bind_personal_user_for_epoch_with_rollback(
                SlackPersonalBindingPrincipal {
                    tenant_id: self.tenant_id.clone(),
                    user_id: user_id.clone(),
                },
                SlackPersonalUserBindingRequest {
                    installation_id: self.installation_id.clone(),
                    slack_user_id: SlackUserId::new(slack_user_id),
                    team_id: self.team_id.clone(),
                    enterprise_id: None,
                    api_app_id: self.api_app_id.clone(),
                },
                epoch,
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// The real facade, for callers that need the full
    /// [`ChannelConnectionFacade`] surface.
    pub fn facade(&self) -> Arc<dyn ChannelConnectionFacade> {
        Arc::clone(&self.facade)
    }

    /// Restart-survival probe (T5 of issue #6105): read the SAME active-state
    /// identity-binding predicate as
    /// [`Self::has_any_active_identity_binding`] for EACH of `user_ids`, but
    /// through ONE fresh `FilesystemSlackHostState` over ONE fresh local-dev
    /// root filesystem reopened at `storage_root` — fully independent of the
    /// live runtime's in-memory handles. This is the integration-tier
    /// approximation of a process restart: it proves the durable binding is
    /// reconstructible the way production reconstructs it on boot
    /// (`build_reborn_services` → `local_dev_slack_host_state_filesystem` →
    /// Slack host-state mounts). Results come back in `user_ids` order; the
    /// single reopen means a positive probe and its non-vacuity control read
    /// the same reconstructed store. Tests only.
    ///
    /// `libsql`-only, matching the factory seam it opens: the local-default
    /// reopen path composes the libsql local-dev backend, so a wider gate
    /// would silently probe a fresh in-memory store on non-libsql builds.
    #[cfg(feature = "libsql")]
    pub async fn active_identity_bindings_after_reopen(
        &self,
        storage_root: &std::path::Path,
        user_ids: &[&UserId],
    ) -> Result<Vec<bool>, String> {
        let host_state_filesystem =
            crate::factory::open_local_dev_slack_host_state_filesystem_for_test(storage_root)
                .await
                .map_err(|error| error.to_string())?;
        let state = Arc::new(FilesystemSlackHostState::new(
            host_state_filesystem,
            self.tenant_id.clone(),
            self.host_user_id.clone(),
            self.agent_id.clone(),
            None,
        ));
        let lookup: Arc<dyn RebornUserIdentityLookup> = state;
        let mut bindings = Vec::with_capacity(user_ids.len());
        for user_id in user_ids {
            bindings.push(
                lookup
                    .user_has_provider_binding_with_provider_user_id_prefix(
                        SLACK_IDENTITY_PROVIDER,
                        user_id,
                        None,
                    )
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(bindings)
    }

    /// Surface (a) of the extensions page: what
    /// `list_extensions` merges via
    /// `ChannelConnectionFacade::caller_channel_connections`
    /// (`ironclaw_product_workflow/src/reborn_services/extensions.rs`).
    /// Returns the `"slack"` entry for `user_id`.
    pub async fn caller_channel_connected(&self, user_id: &UserId) -> Result<bool, String> {
        let connections = self
            .facade
            .caller_channel_connections(WebUiAuthenticatedCaller::new(
                self.tenant_id.clone(),
                user_id.clone(),
                Some(self.agent_id.clone()),
                None,
            ))
            .await
            .map_err(|error| format!("{:?}", error.code))?;
        connections
            .get("slack")
            .copied()
            .ok_or_else(|| "facade response is missing the slack channel entry".to_string())
    }

    /// Durable-state evidence: whether ANY active Slack identity binding is
    /// persisted for `user_id`, across all installations (prefix-unscoped,
    /// unlike [`Self::caller_channel_connected`]). Uses the same
    /// active-state predicate the production connected read uses; disconnect
    /// TOMBSTONES identity records rather than deleting rows (the
    /// cleanup-enumeration store surface still lists tombstones for retry),
    /// so active-state is the correct "binding gone" evidence.
    pub async fn has_any_active_identity_binding(&self, user_id: &UserId) -> Result<bool, String> {
        self.user_identity_lookup
            .user_has_provider_binding_with_provider_user_id_prefix(
                SLACK_IDENTITY_PROVIDER,
                user_id,
                None,
            )
            .await
            .map_err(|error| error.to_string())
    }
}
