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
use ironclaw_conversations::{AdapterKind, ConversationActorPairingService, ExternalActorRef};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, TenantId, UserId};
use ironclaw_product_workflow::{
    ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

use crate::{
    RebornProductAuthServices, SlackHostBetaMounts,
    extension_host::available_extensions::SLACK_BOT_EXTENSION_ID,
    slack::slack_actor_identity::{
        RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER,
        parse_slack_user_identity_provider_user_id,
    },
    slack::slack_host_beta::{SlackPersonalConnectionScope, SlackPersonalConnectionScopeResolver},
    slack::slack_outbound_targets::SlackPersonalDmTargetStore,
    slack::slack_personal_binding::{
        RebornUserIdentityBinding, RebornUserIdentityBindingDeleteStore,
    },
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
    conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
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

    async fn unpair_slack_identity_bindings(
        &self,
        bindings: &[RebornUserIdentityBinding],
    ) -> Result<(), RebornServicesError> {
        let adapter_kind = AdapterKind::new(SLACK_V2_ADAPTER_ID)
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        for binding in bindings {
            let Some((installation_id, external_actor_ref)) =
                slack_identity_conversation_actor(binding)?
            else {
                continue;
            };
            self.conversation_actor_pairings
                .unpair_external_actor(
                    self.tenant_id.clone(),
                    adapter_kind.clone(),
                    installation_id,
                    external_actor_ref,
                )
                .await
                .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        }
        Ok(())
    }

    async fn delete_slack_identity_bindings_after_unpair(
        &self,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<(), RebornServicesError> {
        let bindings = self
            .user_identity_delete_store
            .user_identity_bindings_for_user(
                SLACK_IDENTITY_PROVIDER,
                user_id,
                provider_user_id_prefix,
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        self.unpair_slack_identity_bindings(&bindings).await?;

        let deleted = self
            .user_identity_delete_store
            .delete_user_identity_bindings_for_user(
                SLACK_IDENTITY_PROVIDER,
                user_id,
                provider_user_id_prefix,
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;

        self.unpair_slack_identity_bindings(&deleted).await
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
            self.delete_slack_identity_bindings_after_unpair(&caller.user_id, None)
                .await?;
            return Ok(());
        };
        // Ordering: credential revoke → DM targets → conversation unpair →
        // identity binding. The identity binding is the "connected" signal and
        // deletes last (commit point); actor bindings are revoked before and
        // after the delete so a concurrent inbound that re-pairs from a binding
        // deleted in the same call is cleaned up too.
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
        self.delete_slack_identity_bindings_after_unpair(
            &caller.user_id,
            Some(provider_user_id_prefix.as_str()),
        )
        .await?;
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
        conversation_actor_pairings: mounts.conversation_actor_pairings.clone(),
        personal_dm_target_store: mounts.personal_dm_target_store.clone(),
        personal_credential_cleanup,
    })
}

fn slack_identity_conversation_actor(
    binding: &RebornUserIdentityBinding,
) -> Result<
    Option<(
        ironclaw_conversations::AdapterInstallationId,
        ExternalActorRef,
    )>,
    RebornServicesError,
> {
    if binding.provider.as_str() != SLACK_IDENTITY_PROVIDER {
        return Ok(None);
    }
    let Some((installation_id, slack_user_id)) =
        parse_slack_user_identity_provider_user_id(binding.provider_user_id.as_str())
    else {
        tracing::warn!(
            provider_user_id = binding.provider_user_id.as_str(),
            "skipping Slack conversation unpair for malformed identity provider user id"
        );
        return Ok(None);
    };
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new(installation_id.as_str())
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
    let external_actor_ref = ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id)
        .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
    Ok(Some((installation_id, external_actor_ref)))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_conversations::AdapterInstallationId as ConversationAdapterInstallationId;
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;

    use super::*;
    use crate::{
        slack::slack_actor_identity::{
            RebornUserIdentityLookupError, slack_user_identity_provider_user_id,
        },
        slack::slack_outbound_targets::{
            InMemorySlackPersonalDmTargetStore, SlackPersonalDmTarget, SlackPersonalDmTargetKey,
        },
        slack::slack_personal_binding::RebornUserIdentityBindingError,
        slack::slack_serve::SlackTeamId,
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
        let actor_pairings = Arc::new(RecordingConversationActorPairingService::default());
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
                    crate::slack::slack_serve::SlackUserId::new("U123"),
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
            conversation_actor_pairings: actor_pairings.clone(),
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
        assert_eq!(
            actor_pairings.unpairs(),
            vec![
                (
                    expected_tenant_id.clone(),
                    AdapterKind::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
                    ConversationAdapterInstallationId::new("install-alpha")
                        .expect("installation id"),
                    ExternalActorRef::new(SLACK_USER_ACTOR_KIND, "U123").expect("actor")
                ),
                (
                    expected_tenant_id.clone(),
                    AdapterKind::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
                    ConversationAdapterInstallationId::new("install-alpha")
                        .expect("installation id"),
                    ExternalActorRef::new(SLACK_USER_ACTOR_KIND, "U123").expect("actor")
                )
            ],
            "disconnect must revoke before and after identity deletion to close re-pair races"
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
        assert_eq!(
            actor_pairings.unpairs().len(),
            2,
            "repeat disconnect has no remaining identity records to unpair"
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
            conversation_actor_pairings: Arc::new(
                RecordingConversationActorPairingService::default(),
            ),
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
    async fn slack_channel_connection_facade_keeps_identity_when_conversation_unpair_fails() {
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
            conversation_actor_pairings: Arc::new(FailingConversationActorPairingService),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
            personal_credential_cleanup: None,
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), "slack")
                .await
                .is_err(),
            "conversation unpair failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after failed disconnect"),
            HashMap::from([("slack".to_string(), true)]),
            "identity binding must remain until conversation bindings are revoked"
        );
        assert_eq!(
            identity_store.deletes(),
            Vec::new(),
            "identity delete is the commit point and must not run after an unpair failure"
        );
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_retries_after_identity_delete_failure() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let slack_provider_user_id = slack_user_identity_provider_user_id(&installation_id, "U123");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            slack_provider_user_id,
            user_id.clone(),
        )]));
        identity_store.fail_next_delete();
        let actor_pairings = Arc::new(RecordingConversationActorPairingService::default());
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id,
                team_id,
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            conversation_actor_pairings: actor_pairings.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
            personal_credential_cleanup: None,
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id, user_id.clone(), None::<AgentId>, None);

        assert!(
            facade
                .disconnect_channel_for_caller(caller.clone(), "slack")
                .await
                .is_err(),
            "identity delete failure must fail the disconnect"
        );
        assert_eq!(
            facade
                .caller_channel_connections(caller.clone())
                .await
                .expect("connection lookup after failed delete"),
            HashMap::from([("slack".to_string(), true)]),
            "identity remains connected so a removal retry can run the full cleanup"
        );

        facade
            .disconnect_channel_for_caller(caller.clone(), "slack")
            .await
            .expect("retry succeeds after transient identity delete failure");

        assert_eq!(
            facade
                .caller_channel_connections(caller)
                .await
                .expect("connection lookup after retry"),
            HashMap::from([("slack".to_string(), false)])
        );
        assert_eq!(
            actor_pairings.unpairs().len(),
            3,
            "retry should unpair before identity delete, then unpair deleted identities again"
        );
    }

    #[tokio::test]
    async fn slack_channel_connection_facade_unpairs_identity_added_during_delete() {
        let tenant_id = TenantId::new("tenant:test").expect("tenant");
        let installation_id = AdapterInstallationId::new("install-alpha").expect("installation id");
        let team_id = SlackTeamId::new("T123");
        let user_id = UserId::new("user:alice").expect("user");
        let original_provider_user_id =
            slack_user_identity_provider_user_id(&installation_id, "U123");
        let concurrent_provider_user_id =
            slack_user_identity_provider_user_id(&installation_id, "U999");
        let identity_store = Arc::new(RecordingSlackIdentityStore::new([(
            original_provider_user_id,
            user_id.clone(),
        )]));
        identity_store.insert_before_next_delete([(concurrent_provider_user_id, user_id.clone())]);
        let actor_pairings = Arc::new(RecordingConversationActorPairingService::default());
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: Some(SlackPersonalConnectionScope {
                installation_id,
                team_id,
            }),
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            conversation_actor_pairings: actor_pairings.clone(),
            personal_dm_target_store: Arc::new(InMemorySlackPersonalDmTargetStore::new()),
            personal_credential_cleanup: None,
        };
        let caller =
            WebUiAuthenticatedCaller::new(tenant_id.clone(), user_id, None::<AgentId>, None);

        facade
            .disconnect_channel_for_caller(caller, "slack")
            .await
            .expect("disconnect succeeds");

        let mut unpaired_actor_ids = actor_pairings
            .unpairs()
            .into_iter()
            .map(|(_, _, _, actor)| actor.id().to_string())
            .collect::<Vec<_>>();
        unpaired_actor_ids.sort();
        assert_eq!(
            unpaired_actor_ids,
            vec!["U123".to_string(), "U123".to_string(), "U999".to_string()],
            "same identities are unpaired before and after delete, and identities added during delete are unpaired too"
        );
    }

    #[tokio::test]
    async fn slack_identity_conversation_actor_parses_delimited_installation_ids() {
        let installation_id =
            AdapterInstallationId::new("org:install-alpha").expect("installation id");
        let binding = RebornUserIdentityBinding {
            provider: crate::slack::slack_personal_binding::RebornIdentityProviderId::new(
                SLACK_IDENTITY_PROVIDER,
            )
            .expect("provider"),
            provider_user_id:
                crate::slack::slack_personal_binding::RebornIdentityProviderUserId::new(
                    slack_user_identity_provider_user_id(&installation_id, "U123"),
                )
                .expect("provider user id"),
            user_id: UserId::new("user:alice").expect("user"),
        };

        let parsed = slack_identity_conversation_actor(&binding)
            .expect("parse succeeds")
            .expect("slack binding");

        assert_eq!(
            parsed.0,
            ConversationAdapterInstallationId::new("org:install-alpha")
                .expect("conversation installation id")
        );
        assert_eq!(
            parsed.1,
            ExternalActorRef::new(SLACK_USER_ACTOR_KIND, "U123").expect("actor")
        );
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
            conversation_actor_pairings: Arc::new(
                RecordingConversationActorPairingService::default(),
            ),
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
        let actor_pairings = Arc::new(RecordingConversationActorPairingService::default());
        let facade = SlackChannelConnectionFacade {
            tenant_id: tenant_id.clone(),
            personal_connection_scope: None,
            personal_connection_scope_resolver: None,
            user_identity_lookup: identity_store.clone(),
            user_identity_delete_store: identity_store.clone(),
            conversation_actor_pairings: actor_pairings.clone(),
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
        assert_eq!(
            actor_pairings.unpairs(),
            vec![
                (
                    expected_tenant_id.clone(),
                    AdapterKind::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
                    ConversationAdapterInstallationId::new("install-alpha")
                        .expect("installation id"),
                    ExternalActorRef::new(SLACK_USER_ACTOR_KIND, "U123").expect("actor")
                ),
                (
                    expected_tenant_id,
                    AdapterKind::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
                    ConversationAdapterInstallationId::new("install-alpha")
                        .expect("installation id"),
                    ExternalActorRef::new(SLACK_USER_ACTOR_KIND, "U123").expect("actor")
                )
            ],
            "no-scope disconnect still revokes conversation actor state before and after delete"
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
            conversation_actor_pairings: Arc::new(
                RecordingConversationActorPairingService::default(),
            ),
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
            conversation_actor_pairings: Arc::new(
                RecordingConversationActorPairingService::default(),
            ),
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

    type RecordedUnpair = (
        TenantId,
        AdapterKind,
        ConversationAdapterInstallationId,
        ExternalActorRef,
    );

    #[derive(Default)]
    struct RecordingConversationActorPairingService {
        unpairs: Mutex<Vec<RecordedUnpair>>,
    }

    impl RecordingConversationActorPairingService {
        fn unpairs(&self) -> Vec<RecordedUnpair> {
            self.unpairs.lock().expect("lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl ConversationActorPairingService for RecordingConversationActorPairingService {
        async fn pair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: ConversationAdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor(
            &self,
            tenant_id: TenantId,
            adapter_kind: AdapterKind,
            adapter_installation_id: ConversationAdapterInstallationId,
            external_actor_ref: ExternalActorRef,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            self.unpairs.lock().expect("lock").push((
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            ));
            Ok(())
        }
    }

    struct FailingConversationActorPairingService;

    #[async_trait::async_trait]
    impl ConversationActorPairingService for FailingConversationActorPairingService {
        async fn pair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: ConversationAdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: ConversationAdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Err(ironclaw_conversations::InboundTurnError::DurableState {
                reason: "conversation unpair unavailable".to_string(),
            })
        }
    }

    #[derive(Default)]
    struct RecordingSlackIdentityStore {
        bindings: Mutex<HashMap<String, UserId>>,
        deletes: Mutex<Vec<(String, UserId, Option<String>)>>,
        fail_delete_once: Mutex<bool>,
        insert_before_delete: Mutex<Vec<(String, UserId)>>,
    }

    impl RecordingSlackIdentityStore {
        fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
            Self {
                bindings: Mutex::new(bindings.into_iter().collect()),
                deletes: Mutex::new(Vec::new()),
                fail_delete_once: Mutex::new(false),
                insert_before_delete: Mutex::new(Vec::new()),
            }
        }

        fn deletes(&self) -> Vec<(String, UserId, Option<String>)> {
            self.deletes.lock().expect("lock").clone()
        }

        fn fail_next_delete(&self) {
            *self.fail_delete_once.lock().expect("lock") = true;
        }

        fn insert_before_next_delete(&self, bindings: impl IntoIterator<Item = (String, UserId)>) {
            self.insert_before_delete
                .lock()
                .expect("lock")
                .extend(bindings);
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
        async fn user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<
            Vec<crate::slack::slack_personal_binding::RebornUserIdentityBinding>,
            RebornUserIdentityBindingError,
        > {
            let bindings = self.bindings.lock().expect("lock");
            Ok(bindings
                .iter()
                .filter(|(provider_user_id, bound_user_id)| {
                    let prefix_matches = provider_user_id_prefix
                        .map(|prefix| provider_user_id.starts_with(prefix))
                        .unwrap_or(true);
                    provider == SLACK_IDENTITY_PROVIDER
                        && *bound_user_id == user_id
                        && prefix_matches
                })
                .map(|(provider_user_id, bound_user_id)| {
                    crate::slack::slack_personal_binding::RebornUserIdentityBinding {
                        provider:
                            crate::slack::slack_personal_binding::RebornIdentityProviderId::new(
                                provider,
                            )
                            .expect("provider"),
                        provider_user_id:
                            crate::slack::slack_personal_binding::RebornIdentityProviderUserId::new(
                                provider_user_id.clone(),
                            )
                            .expect("provider user id"),
                        user_id: bound_user_id.clone(),
                    }
                })
                .collect())
        }

        async fn delete_user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<
            Vec<crate::slack::slack_personal_binding::RebornUserIdentityBinding>,
            RebornUserIdentityBindingError,
        > {
            self.deletes.lock().expect("lock").push((
                provider.to_string(),
                user_id.clone(),
                provider_user_id_prefix.map(ToString::to_string),
            ));
            let mut fail_delete_once = self.fail_delete_once.lock().expect("lock");
            if *fail_delete_once {
                *fail_delete_once = false;
                return Err(RebornUserIdentityBindingError::Backend(
                    "identity delete unavailable".to_string(),
                ));
            }
            drop(fail_delete_once);
            let mut bindings = self.bindings.lock().expect("lock");
            bindings.extend(self.insert_before_delete.lock().expect("lock").drain(..));
            let mut deleted = Vec::new();
            bindings.retain(|provider_user_id, bound_user_id| {
                let prefix_matches = provider_user_id_prefix
                    .map(|prefix| provider_user_id.starts_with(prefix))
                    .unwrap_or(true);
                let should_delete = bound_user_id == user_id && prefix_matches;
                if should_delete {
                    deleted.push(crate::slack::slack_personal_binding::RebornUserIdentityBinding {
                        provider: crate::slack::slack_personal_binding::RebornIdentityProviderId::new(provider)
                            .expect("provider"),
                        provider_user_id:
                            crate::slack::slack_personal_binding::RebornIdentityProviderUserId::new(
                                provider_user_id.clone(),
                            )
                            .expect("provider user id"),
                        user_id: bound_user_id.clone(),
                    });
                }
                !should_delete
            });
            Ok(deleted)
        }
    }

    #[derive(Debug)]
    struct FailingSlackPersonalDmTargetStore;

    #[async_trait::async_trait]
    impl SlackPersonalDmTargetStore for FailingSlackPersonalDmTargetStore {
        async fn load_personal_dm_target(
            &self,
            _key: &crate::slack::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<
            Option<crate::slack::slack_outbound_targets::SlackPersonalDmTarget>,
            crate::slack::slack_outbound_targets::SlackPersonalDmTargetError,
        > {
            Ok(None)
        }

        async fn upsert_personal_dm_target(
            &self,
            target: crate::slack::slack_outbound_targets::SlackPersonalDmTarget,
        ) -> Result<
            crate::slack::slack_outbound_targets::SlackPersonalDmTarget,
            crate::slack::slack_outbound_targets::SlackPersonalDmTargetError,
        > {
            Ok(target)
        }

        async fn delete_personal_dm_target(
            &self,
            _key: &crate::slack::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<bool, crate::slack::slack_outbound_targets::SlackPersonalDmTargetError>
        {
            Err(crate::slack::slack_outbound_targets::SlackPersonalDmTargetError::StoreUnavailable)
        }

        async fn delete_personal_dm_targets_for_user(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
            _installation_id: &AdapterInstallationId,
            _team_id: &SlackTeamId,
        ) -> Result<usize, crate::slack::slack_outbound_targets::SlackPersonalDmTargetError>
        {
            Err(crate::slack::slack_outbound_targets::SlackPersonalDmTargetError::StoreUnavailable)
        }
    }
}
