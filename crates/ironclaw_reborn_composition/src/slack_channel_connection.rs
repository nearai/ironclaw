//! Per-user Slack channel-connection facade.
//!
//! Reports whether the calling WebUI user has connected their own Slack account
//! (so the extensions surface can show a "setup needed" Configure affordance
//! until they pair) and handles per-caller disconnect (identity + personal DM
//! target cleanup). Split out of `slack_connectable_channel` so that file stays
//! the connectable-channel descriptor/wiring layer.

use std::{collections::HashMap, sync::Arc};

use ironclaw_host_api::TenantId;
use ironclaw_product_workflow::{
    ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
};

use crate::{
    SlackHostBetaMounts,
    slack_actor_identity::{RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER},
    slack_host_beta::{SlackPersonalConnectionScope, SlackPersonalConnectionScopeResolver},
    slack_outbound_targets::SlackPersonalDmTargetStore,
    slack_personal_binding::RebornUserIdentityBindingDeleteStore,
};

/// Per-user channel connection facade backed by the Slack personal-binding
/// identity store. Reports whether the calling WebUI user has connected their
/// own Slack account, so the extensions surface can show a "setup needed"
/// Configure affordance until they pair.
struct SlackChannelConnectionFacade {
    tenant_id: TenantId,
    personal_connection_scope: Option<SlackPersonalConnectionScope>,
    personal_connection_scope_resolver: Option<Arc<dyn SlackPersonalConnectionScopeResolver>>,
    user_identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
}

impl SlackChannelConnectionFacade {
    async fn resolve_personal_connection_scope(
        &self,
    ) -> Result<Option<SlackPersonalConnectionScope>, RebornServicesError> {
        if let Some(resolver) = &self.personal_connection_scope_resolver {
            return resolver
                .resolve_personal_connection_scope()
                .await
                .map_err(RebornServicesError::internal_from);
        }
        Ok(self.personal_connection_scope.clone())
    }
}

#[async_trait::async_trait]
impl ChannelConnectionFacade for SlackChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<HashMap<String, bool>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(HashMap::from([("slack".to_string(), false)]));
        }
        let Some(scope) = self.resolve_personal_connection_scope().await? else {
            return Ok(HashMap::from([("slack".to_string(), false)]));
        };
        let provider_user_id_prefix = format!("{}:", scope.installation_id.as_str());
        let connected = self
            .user_identity_lookup
            .user_has_provider_binding_with_provider_user_id_prefix(
                SLACK_IDENTITY_PROVIDER,
                &caller.user_id,
                Some(provider_user_id_prefix.as_str()),
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        Ok(HashMap::from([("slack".to_string(), connected)]))
    }

    async fn disconnect_channel_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        channel: &str,
    ) -> Result<(), RebornServicesError> {
        if channel != "slack" || caller.tenant_id != self.tenant_id {
            return Ok(());
        }
        let Some(scope) = self.resolve_personal_connection_scope().await? else {
            return Err(RebornServicesError::internal_from(
                "Slack personal connection scope is unavailable; refusing unscoped disconnect",
            ));
        };
        self.personal_dm_target_store
            .delete_personal_dm_targets_for_user(
                &self.tenant_id,
                &caller.user_id,
                &scope.installation_id,
                &scope.team_id,
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        let provider_user_id_prefix = format!("{}:", scope.installation_id.as_str());
        self.user_identity_delete_store
            .delete_user_identity_bindings_for_user(
                SLACK_IDENTITY_PROVIDER,
                &caller.user_id,
                Some(provider_user_id_prefix.as_str()),
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        Ok(())
    }
}

pub(crate) fn slack_channel_connection_facade(
    mounts: &SlackHostBetaMounts,
) -> Arc<dyn ChannelConnectionFacade> {
    Arc::new(SlackChannelConnectionFacade {
        tenant_id: mounts.tenant_id.clone(),
        personal_connection_scope: mounts.personal_connection_scope.clone(),
        personal_connection_scope_resolver: Some(mounts.personal_connection_scope_resolver.clone()),
        user_identity_lookup: mounts.user_identity_lookup.clone(),
        user_identity_delete_store: mounts.user_identity_delete_store.clone(),
        personal_dm_target_store: mounts.personal_dm_target_store.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;

    use super::*;
    use crate::{
        slack_actor_identity::{
            RebornUserIdentityLookupError, slack_user_identity_provider_user_id,
        },
        slack_outbound_targets::{
            InMemorySlackPersonalDmTargetStore, SlackPersonalDmTarget, SlackPersonalDmTargetKey,
        },
        slack_personal_binding::RebornUserIdentityBindingError,
        slack_serve::SlackTeamId,
    };

    #[tokio::test]
    async fn slack_channel_connection_facade_disconnects_identity_and_personal_dm_target() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        let dm_target_store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let dm_target_key = SlackPersonalDmTargetKey::new(
            tenant_id.clone(),
            installation_id.clone(),
            team_id.clone(),
            user_id.clone(),
        )
        .expect("dm target key");
        dm_target_store
            .upsert_personal_dm_target(
                SlackPersonalDmTarget::new(
                    dm_target_key.clone(),
                    crate::slack_serve::SlackUserId::new("U123"),
                    "D123".to_string(),
                )
                .expect("dm target"),
            )
            .await
            .expect("seed dm target");
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id: installation_id.clone(),
                team_id: team_id.clone(),
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: dm_target_store.clone(),
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([("slack".to_string(), true)])
        );

        facade
            .disconnect_channel_for_caller(caller.clone(), "slack")
            .await
            .expect("disconnect succeeds");

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup after disconnect"),
            HashMap::from([("slack".to_string(), false)])
        );
        assert_eq!(
            identity_store.deletes(),
            vec![(
                "slack".to_string(),
                user_id,
                Some("install-alpha:".to_string())
            )]
        );
        assert_eq!(
            dm_target_store
                .load_personal_dm_target(&dm_target_key)
                .await
                .expect("dm target lookup after disconnect"),
            None
        );

        // Retry convergence for extension removal: `remove_extension` runs the
        // caller disconnect before `ExtensionRemove`, so a failed removal
        // retries the disconnect for a caller who is already unpaired. That
        // repeat disconnect must stay an idempotent no-op success (the scope
        // resolves from the installation, and deleting zero records is Ok),
        // not an error that would wedge the removal retry.
        facade
            .disconnect_channel_for_caller(caller.clone(), "slack")
            .await
            .expect("repeat disconnect for an unpaired caller is an idempotent no-op");
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after repeat disconnect"),
            HashMap::from([("slack".to_string(), false)])
        );
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_keeps_identity_when_dm_target_delete_fails() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id,
                team_id,
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: Arc::new(FailingSlackPersonalDmTargetStore),
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), "slack")
                .await
                .is_err(),
            "DM target cleanup failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after failed disconnect"),
            HashMap::from([("slack".to_string(), true)]),
            "identity binding must remain until outbound delivery target cleanup succeeds"
        );
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_requires_current_installation_scope() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let other_installation_id =
            AdapterInstallationId::new("install-beta").expect("other installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let other_installation_provider_user_id =
            slack_user_identity_provider_user_id(&other_installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            other_installation_provider_user_id,
            user_id.clone(),
        )]));
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id,
                team_id,
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup"),
            HashMap::from([("slack".to_string(), false)])
        );
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_refuses_unscoped_disconnect() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let user_id = UserId::new("user:alice").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: None,
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        };
        let caller = WebUiAuthenticatedCaller::new(tenant_id, user_id, None::<AgentId>, None);

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([("slack".to_string(), false)])
        );
        assert!(
            facade
                .disconnect_channel_for_caller(caller, "slack")
                .await
                .is_err(),
            "disconnect must fail closed when no Slack installation scope is available"
        );
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_requires_current_tenant() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        let facade = SlackChannelConnectionFacade {
            tenant_id,
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id,
                team_id,
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        };
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("tenant:other").expect("other tenant"),
            user_id,
            None::<AgentId>,
            None,
        );

        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup"),
            HashMap::from([("slack".to_string(), false)])
        );
    }

    #[derive(Default)]
    struct RecordingSlackIdentityStore {
        bindings: Mutex<HashMap<String, UserId>>,
        deletes: Mutex<Vec<(String, UserId, Option<String>)>>,
    }

    impl RecordingSlackIdentityStore {
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

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for RecordingSlackIdentityStore {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            if provider != SLACK_IDENTITY_PROVIDER {
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
            if provider != SLACK_IDENTITY_PROVIDER {
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

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingDeleteStore for RecordingSlackIdentityStore {
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

    #[derive(Debug)]
    struct FailingSlackPersonalDmTargetStore;

    #[async_trait::async_trait]
    impl SlackPersonalDmTargetStore for FailingSlackPersonalDmTargetStore {
        async fn load_personal_dm_target(
            &self,
            _key: &crate::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<
            Option<crate::slack_outbound_targets::SlackPersonalDmTarget>,
            crate::slack_outbound_targets::SlackPersonalDmTargetError,
        > {
            Ok(None)
        }

        async fn upsert_personal_dm_target(
            &self,
            target: crate::slack_outbound_targets::SlackPersonalDmTarget,
        ) -> Result<
            crate::slack_outbound_targets::SlackPersonalDmTarget,
            crate::slack_outbound_targets::SlackPersonalDmTargetError,
        > {
            Ok(target)
        }

        async fn delete_personal_dm_target(
            &self,
            _key: &crate::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<bool, crate::slack_outbound_targets::SlackPersonalDmTargetError> {
            Err(crate::slack_outbound_targets::SlackPersonalDmTargetError::StoreUnavailable)
        }

        async fn delete_personal_dm_targets_for_user(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
            _installation_id: &AdapterInstallationId,
            _team_id: &SlackTeamId,
        ) -> Result<usize, crate::slack_outbound_targets::SlackPersonalDmTargetError> {
            Err(crate::slack_outbound_targets::SlackPersonalDmTargetError::StoreUnavailable)
        }
    }
}
