//! Per-user Slack channel-connection facade.
//!
//! Reports whether the calling WebUI user has connected their own Slack account
//! (so the extensions surface can show a "setup needed" Configure affordance
//! until they connect) and handles per-caller disconnect (identity + personal DM
//! target cleanup). Split out of `slack_connectable_channel` so that file stays
//! the connectable-channel descriptor/wiring layer.

use std::{collections::HashMap, sync::Arc};

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, SLACK_PERSONAL_PROVIDER_ID, SecretCleanupAction,
    SecretCleanupReport, SecretCleanupRequest,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, TenantId};
use ironclaw_product_workflow::{
    ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
};

use crate::{
    RebornProductAuthServices, SlackHostBetaMounts,
    extension_host::available_extensions::SLACK_BOT_EXTENSION_ID,
    slack_actor_identity::{RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER},
    slack_host_beta::{SlackPersonalConnectionScope, SlackPersonalConnectionScopeResolver},
    slack_outbound_targets::SlackPersonalDmTargetStore,
    slack_personal_binding::RebornUserIdentityBindingDeleteStore,
};

/// Narrow disconnect-side port over product-auth lifecycle cleanup, so the
/// per-user Slack disconnect can revoke the caller's `slack_personal`
/// credential without depending on the whole product-auth bundle (and so tests
/// can record the issued cleanup). Production forwards to
/// [`RebornProductAuthServices::cleanup_credentials_for_lifecycle`], the
/// guardrail-sanctioned lifecycle cleanup entry point.
#[async_trait::async_trait]
pub(crate) trait SlackPersonalCredentialCleanup: Send + Sync {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, RebornServicesError>;
}

#[async_trait::async_trait]
impl SlackPersonalCredentialCleanup for RebornProductAuthServices {
    async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, RebornServicesError> {
        RebornProductAuthServices::cleanup_credentials_for_lifecycle(self, request)
            .await
            .map_err(|error| {
                RebornServicesError::internal_from(format!(
                    "slack personal credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}

/// Per-user channel connection facade backed by the Slack personal-binding
/// identity store. Reports whether the calling WebUI user has connected their
/// own Slack account, so the extensions surface can show a "setup needed"
/// Configure affordance until they connect.
struct SlackChannelConnectionFacade {
    tenant_id: TenantId,
    personal_connection_scope: Option<SlackPersonalConnectionScope>,
    personal_connection_scope_resolver: Option<Arc<dyn SlackPersonalConnectionScopeResolver>>,
    user_identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
    // Genuinely optional (not an `optional_arc` smell): compositions without
    // product auth cannot have minted a `slack_personal` credential in the
    // first place, so there is nothing to clean up on disconnect.
    personal_credential_cleanup: Option<Arc<dyn SlackPersonalCredentialCleanup>>,
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
            // No workspace setup means there is no installation scope to key
            // the DM-target or prefix-scoped binding deletes — the state of a
            // fresh instance, or one whose setup was deleted. Refusing here
            // used to 500 extension uninstall before Slack was ever
            // configured. Instead: still revoke the caller's provider-scoped
            // credentials, then drop the caller's own Slack bindings without
            // an installation prefix (the delete stays tenant + caller-user
            // bound). DM targets are skipped — they are keyed by installation
            // and unreachable while no setup exists.
            if let Some(cleanup) = &self.personal_credential_cleanup {
                cleanup
                    .cleanup_credentials_for_lifecycle(personal_credential_cleanup_request(
                        &caller,
                    )?)
                    .await?;
            }
            self.user_identity_delete_store
                .delete_user_identity_bindings_for_user(
                    SLACK_IDENTITY_PROVIDER,
                    &caller.user_id,
                    None,
                )
                .await
                .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
            return Ok(());
        };
        // Ordering: credential revoke → DM targets → identity binding. The
        // binding is the "connected" signal and deletes last (commit point);
        // the credential revokes first so a mid-sequence failure leaves the
        // caller visibly connected with every step retryable — deleting DM
        // targets before a failing revoke would silently break proactive DMs
        // while the UI still shows connected.
        if let Some(cleanup) = &self.personal_credential_cleanup {
            cleanup
                .cleanup_credentials_for_lifecycle(personal_credential_cleanup_request(&caller)?)
                .await?;
        }
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

// OAuth-minted personal credentials carry no extension ownership/grants, so
// the provider selector is what actually reaches the caller's
// `slack_personal` account. Shared by the scoped and no-scope disconnect
// arms so the revoke request cannot drift between them.
fn personal_credential_cleanup_request(
    caller: &WebUiAuthenticatedCaller,
) -> Result<SecretCleanupRequest, RebornServicesError> {
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
        extension_id: ExtensionId::new(SLACK_BOT_EXTENSION_ID)
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?,
        provider: Some(
            AuthProviderId::new(SLACK_PERSONAL_PROVIDER_ID)
                .map_err(|error| RebornServicesError::internal_from(error.to_string()))?,
        ),
        action: SecretCleanupAction::Uninstall,
    })
}

pub(crate) fn slack_channel_connection_facade(
    mounts: &SlackHostBetaMounts,
    personal_credential_cleanup: Option<Arc<dyn SlackPersonalCredentialCleanup>>,
) -> Arc<dyn ChannelConnectionFacade> {
    Arc::new(SlackChannelConnectionFacade {
        tenant_id: mounts.tenant_id.clone(),
        personal_connection_scope: mounts.personal_connection_scope.clone(),
        personal_connection_scope_resolver: Some(mounts.personal_connection_scope_resolver.clone()),
        user_identity_lookup: mounts.user_identity_lookup.clone(),
        user_identity_delete_store: mounts.user_identity_delete_store.clone(),
        personal_dm_target_store: mounts.personal_dm_target_store.clone(),
        personal_credential_cleanup,
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
        let expected_tenant_id = tenant_id.clone();
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
        let cleanup = Arc::new(RecordingCleanupService::default());
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
            personal_credential_cleanup: Some(cleanup.clone()),
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

        // Disconnect must also revoke the caller's `slack_personal` credential
        // through the product-auth lifecycle cleanup port, scoped to exactly
        // this tenant + caller and the public Slack extension.
        let cleanup_requests = cleanup.requests();
        assert_eq!(
            cleanup_requests.len(),
            1,
            "disconnect must issue exactly one credential cleanup"
        );
        assert_eq!(cleanup_requests[0].extension_id.as_str(), "slack_bot");
        assert_eq!(
            cleanup_requests[0].provider.as_ref().map(|p| p.as_str()),
            Some(SLACK_PERSONAL_PROVIDER_ID),
            "the provider selector is what reaches the grant-less OAuth account"
        );
        assert_eq!(cleanup_requests[0].action, SecretCleanupAction::Uninstall);
        assert_eq!(
            &cleanup_requests[0].scope.resource.tenant_id,
            &expected_tenant_id
        );
        assert_eq!(&cleanup_requests[0].scope.resource.user_id, &user_id);

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
        // retries the disconnect for a caller who is already disconnected. That
        // repeat disconnect must stay an idempotent no-op success (the scope
        // resolves from the installation, and deleting zero records is Ok),
        // not an error that would wedge the removal retry.
        facade
            .disconnect_channel_for_caller(caller.clone(), "slack")
            .await
            .expect("repeat disconnect for a disconnected caller is an idempotent no-op");
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after repeat disconnect"),
            HashMap::from([("slack".to_string(), false)])
        );
        assert_eq!(
            cleanup.requests().len(),
            2,
            "the removal-retry repeat disconnect re-issues the (idempotent) credential cleanup"
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
            personal_credential_cleanup: None,
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
            personal_credential_cleanup: None,
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
    async fn slack_channel_connection_facade_disconnects_without_setup_scope() {
        // A fresh instance (or one whose workspace setup was deleted) has no
        // installation scope. Uninstall/disconnect must still succeed —
        // refusing here used to 500 extension removal before Slack was ever
        // configured — and must clean the caller's own bindings without an
        // installation prefix while staying caller-bound.
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let expected_tenant_id = tenant_id.clone();
        let user_id = UserId::new("user:alice").expect("user");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        let cleanup = Arc::new(RecordingCleanupService::default());
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: None,
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
            personal_credential_cleanup: Some(cleanup.clone()),
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup"),
            HashMap::from([("slack".to_string(), false)])
        );
        facade
            .disconnect_channel_for_caller(caller, "slack")
            .await
            .expect("disconnect succeeds without a setup scope");
        let cleanup_requests = cleanup.requests();
        assert_eq!(
            cleanup_requests.len(),
            1,
            "no-scope disconnect must still revoke the caller's credential"
        );
        assert_eq!(
            cleanup_requests[0].provider.as_ref().map(|p| p.as_str()),
            Some(SLACK_PERSONAL_PROVIDER_ID)
        );
        assert_eq!(cleanup_requests[0].action, SecretCleanupAction::Uninstall);
        assert_eq!(
            &cleanup_requests[0].scope.resource.tenant_id,
            &expected_tenant_id
        );
        assert_eq!(&cleanup_requests[0].scope.resource.user_id, &user_id);
        assert_eq!(
            identity_store.deletes(),
            vec![(SLACK_IDENTITY_PROVIDER.to_string(), user_id, None)],
            "caller's slack bindings are cleaned without an installation prefix"
        );
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
            personal_credential_cleanup: None,
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

    #[tokio::test]
    async fn slack_channel_connection_facade_keeps_identity_when_credential_cleanup_fails() {
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
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
            personal_credential_cleanup: Some(Arc::new(FailingCleanupService)),
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), "slack")
                .await
                .is_err(),
            "credential cleanup failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after failed disconnect"),
            HashMap::from([("slack".to_string(), true)]),
            "identity binding must remain until credential cleanup succeeds, so the removal retry re-runs the full disconnect"
        );
        assert_eq!(identity_store.deletes(), Vec::new());
    }

    #[derive(Default)]
    struct RecordingCleanupService {
        requests: Mutex<Vec<SecretCleanupRequest>>,
    }

    impl RecordingCleanupService {
        fn requests(&self) -> Vec<SecretCleanupRequest> {
            self.requests.lock().expect("lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalCredentialCleanup for RecordingCleanupService {
        async fn cleanup_credentials_for_lifecycle(
            &self,
            request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, RebornServicesError> {
            self.requests.lock().expect("lock").push(request);
            Ok(SecretCleanupReport::default())
        }
    }

    struct FailingCleanupService;

    #[async_trait::async_trait]
    impl SlackPersonalCredentialCleanup for FailingCleanupService {
        async fn cleanup_credentials_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, RebornServicesError> {
            Err(RebornServicesError::internal_from(
                "credential cleanup unavailable",
            ))
        }
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
