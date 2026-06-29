use std::{collections::HashMap, sync::Arc};

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ChannelConnectionFacade, ConnectableChannelsProductFacade, RebornChannelConnectAction,
    RebornChannelConnectStrategy, RebornConnectableChannelInfo,
    RebornConnectableChannelListResponse, RebornServicesError, WebUiAuthenticatedCaller,
};

use crate::{
    RebornBuildError, RebornRuntime, RebornWebuiBundle, SlackHostBetaMounts,
    slack_actor_identity::{RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER},
    slack_host_beta::SlackPersonalConnectionScope,
    slack_outbound_targets::SlackPersonalDmTargetStore,
    slack_personal_binding::RebornUserIdentityBindingDeleteStore,
    webui::build_webui_services_with_connectable_channels,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackOperatorRouteVisibility {
    Hidden,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlackConnectableChannelVisibility {
    Hidden,
    PersonalPairing,
    PersonalPairingAndAdminChannelManagement,
}

pub fn build_webui_services_with_slack_host_beta_mounts(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    slack_mounts: Option<&SlackHostBetaMounts>,
    operator_route_visibility: SlackOperatorRouteVisibility,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let visibility = match (slack_mounts.is_some(), operator_route_visibility) {
        (false, _) => SlackConnectableChannelVisibility::Hidden,
        (true, SlackOperatorRouteVisibility::Hidden) => {
            SlackConnectableChannelVisibility::PersonalPairing
        }
        (true, SlackOperatorRouteVisibility::Visible) => {
            SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement
        }
    };
    let outbound_delivery_target_providers = slack_mounts
        .filter(|mounts| !mounts.outbound_delivery_target_provider_registered)
        .map(|mounts| vec![Arc::clone(&mounts.outbound_delivery_target_provider)])
        .unwrap_or_default();
    let connectable_channels = slack_mounts.and_then(|mounts| {
        slack_connectable_channels(
            visibility,
            mounts.channel_routes.tenant_id().clone(),
            mounts.channel_routes.operator_user_id().clone(),
        )
    });
    if slack_mounts.is_some() && runtime.outbound_delivery_target_provider().is_none() {
        return Err(RebornBuildError::InvalidConfig {
            reason: "outbound delivery target providers require local runtime services".to_string(),
        });
    }
    let channel_connection = slack_mounts.map(slack_channel_connection_facade);
    build_webui_services_with_connectable_channels(
        runtime,
        event_stream,
        connectable_channels,
        channel_connection,
        outbound_delivery_target_providers,
    )
}

#[cfg(test)]
fn build_webui_services_with_slack_connectable_channel(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    visibility: SlackConnectableChannelVisibility,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    build_webui_services_with_connectable_channels(
        runtime,
        event_stream,
        slack_connectable_channels(
            visibility,
            TenantId::new("tenant:test").expect("tenant"),
            UserId::new("user:operator").expect("operator"),
        ),
        None,
        Vec::new(),
    )
}

fn slack_connectable_channels(
    visibility: SlackConnectableChannelVisibility,
    tenant_id: TenantId,
    operator_user_id: UserId,
) -> Option<Arc<dyn ConnectableChannelsProductFacade>> {
    (visibility != SlackConnectableChannelVisibility::Hidden).then(|| {
        Arc::new(SlackConnectableChannelsProductFacade {
            visibility,
            tenant_id,
            operator_user_id,
        }) as Arc<dyn ConnectableChannelsProductFacade>
    })
}

#[derive(Debug)]
struct SlackConnectableChannelsProductFacade {
    visibility: SlackConnectableChannelVisibility,
    tenant_id: TenantId,
    operator_user_id: UserId,
}

#[async_trait::async_trait]
impl ConnectableChannelsProductFacade for SlackConnectableChannelsProductFacade {
    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        let mut channels = vec![slack_inbound_proof_code_connectable_channel()];
        if self.visibility
            == SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement
            && caller.operator_webui_config
            && caller.tenant_id == self.tenant_id
            && caller.user_id == self.operator_user_id
        {
            channels.push(slack_admin_managed_channel_connectable_channel());
        }
        Ok(RebornConnectableChannelListResponse { channels })
    }
}

/// Per-user channel connection facade backed by the Slack personal-binding
/// identity store. Reports whether the calling WebUI user has connected their
/// own Slack account, so the extensions surface can show a "setup needed"
/// Configure affordance until they pair.
struct SlackChannelConnectionFacade {
    tenant_id: TenantId,
    personal_connection_scope: Option<SlackPersonalConnectionScope>,
    user_identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
}

#[async_trait::async_trait]
impl ChannelConnectionFacade for SlackChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<HashMap<String, bool>, RebornServicesError> {
        let connected = self
            .user_identity_lookup
            .user_has_provider_binding(SLACK_IDENTITY_PROVIDER, &caller.user_id)
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
        let provider_user_id_prefix = self
            .personal_connection_scope
            .as_ref()
            .map(|scope| format!("{}:", scope.installation_id.as_str()));
        self.user_identity_delete_store
            .delete_user_identity_bindings_for_user(
                SLACK_IDENTITY_PROVIDER,
                &caller.user_id,
                provider_user_id_prefix.as_deref(),
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        let installation_id = self
            .personal_connection_scope
            .as_ref()
            .map(|scope| &scope.installation_id);
        let team_id = self
            .personal_connection_scope
            .as_ref()
            .map(|scope| scope.team_id.as_str());
        self.personal_dm_target_store
            .delete_personal_dm_targets_for_user(
                &self.tenant_id,
                &caller.user_id,
                installation_id,
                team_id,
            )
            .await
            .map_err(|error| RebornServicesError::internal_from(error.to_string()))?;
        Ok(())
    }
}

fn slack_channel_connection_facade(
    mounts: &SlackHostBetaMounts,
) -> Arc<dyn ChannelConnectionFacade> {
    Arc::new(SlackChannelConnectionFacade {
        tenant_id: mounts.tenant_id.clone(),
        personal_connection_scope: mounts.personal_connection_scope.clone(),
        user_identity_lookup: mounts.user_identity_lookup.clone(),
        user_identity_delete_store: mounts.user_identity_delete_store.clone(),
        personal_dm_target_store: mounts.personal_dm_target_store.clone(),
    })
}

fn slack_inbound_proof_code_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::InboundProofCode,
        action: RebornChannelConnectAction {
            title: "Slack account connection".to_string(),
            instructions:
                "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one."
                    .to_string(),
            input_placeholder: "Enter Slack pairing code...".to_string(),
            submit_label: "Connect".to_string(),
            success_message: "Slack account connected.".to_string(),
            error_message:
                "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one."
                    .to_string(),
        },
        command_aliases: vec![
            "slack".to_string(),
            "slack account".to_string(),
            "slack pairing".to_string(),
        ],
    }
}

fn slack_admin_managed_channel_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::AdminManagedChannels,
        action: RebornChannelConnectAction {
            title: "Slack workspace setup".to_string(),
            instructions: "Configure the Slack app, then map channels to the team agents that should answer there.".to_string(),
            input_placeholder: "C0123456789".to_string(),
            submit_label: "Save channels".to_string(),
            success_message: "Slack channels saved.".to_string(),
            error_message: "Slack channel update failed.".to_string(),
        },
        command_aliases: vec![],
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;
    use ironclaw_turns::run_profile::LoopCapabilityPort;

    use super::*;
    use crate::{
        RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
        local_dev_runtime_policy,
        slack_actor_identity::{
            RebornUserIdentityLookupError, slack_user_identity_provider_user_id,
        },
        slack_outbound_targets::{
            InMemorySlackPersonalDmTargetStore, SlackPersonalDmTarget, SlackPersonalDmTargetKey,
        },
        slack_personal_binding::RebornUserIdentityBindingError,
        slack_serve::SlackTeamId,
    };

    const SLACK_OPERATOR_TENANT: &str = "tenant:test";
    const SLACK_OPERATOR_USER: &str = "user:operator";

    #[test]
    fn slack_admin_managed_connectable_channel_matches_allowed_channel_copy() {
        let channel = slack_admin_managed_channel_connectable_channel();

        assert_eq!(channel.channel, "slack");
        assert_eq!(
            channel.strategy,
            RebornChannelConnectStrategy::AdminManagedChannels
        );
        assert_eq!(
            channel.action.instructions,
            "Configure the Slack app, then map channels to the team agents that should answer there."
        );
        assert!(channel.command_aliases.is_empty());
    }

    #[test]
    fn slack_requirement_copy_matches_connectable_descriptor() {
        // The in-chat connect requirement (built in extension_lifecycle) and the
        // Settings connectable-channels descriptor must read identically for Slack.
        // Enforce that invariant here so the two copies can never silently drift.
        let descriptor = slack_inbound_proof_code_connectable_channel();
        let requirement =
            crate::extension_lifecycle::channel_connection_requirement("slack", "Slack");

        assert_eq!(requirement.channel, descriptor.channel);
        assert_eq!(requirement.instructions, descriptor.action.instructions);
        assert_eq!(
            requirement.input_placeholder,
            descriptor.action.input_placeholder
        );
        assert_eq!(requirement.submit_label, descriptor.action.submit_label);
        assert_eq!(requirement.error_message, descriptor.action.error_message);
        // The requirement's `strategy` string must be the descriptor strategy's
        // wire value (what the Settings UI branches on).
        let strategy_wire = serde_json::to_value(descriptor.strategy)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .expect("strategy serializes to a string");
        assert_eq!(requirement.strategy, strategy_wire);
    }

    #[test]
    fn slack_inbound_proof_code_connectable_channel_matches_pairing_copy() {
        let channel = slack_inbound_proof_code_connectable_channel();

        assert_eq!(channel.channel, "slack");
        assert_eq!(
            channel.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert_eq!(
            channel.action.input_placeholder,
            "Enter Slack pairing code..."
        );
        assert_eq!(
            channel.action.instructions,
            "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one."
        );
        assert_eq!(
            channel.action.error_message,
            "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one."
        );
        assert_eq!(
            channel.command_aliases,
            vec![
                "slack".to_string(),
                "slack account".to_string(),
                "slack pairing".to_string()
            ]
        );
    }

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
            team_id.as_str().to_string(),
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
    }

    #[tokio::test]
    async fn slack_mounts_inject_channel_admin_action_into_webui_facade() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "slack-webui-tenant".to_string(),
                agent_id: "slack-webui-agent".to_string(),
                source_binding_id: "slack-webui-source".to_string(),
                reply_target_binding_id: "slack-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_slack_connectable_channel(
            &runtime,
            None,
            SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(SLACK_OPERATOR_TENANT).expect("tenant"),
            UserId::new(SLACK_OPERATOR_USER).expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        )
        .with_operator_webui_config(true);

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 2);
        let personal = &response.channels[0];
        assert_eq!(personal.channel, "slack");
        assert_eq!(
            personal.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        let channel_admin = &response.channels[1];
        assert_eq!(channel_admin.channel, "slack");
        assert_eq!(
            channel_admin.strategy,
            RebornChannelConnectStrategy::AdminManagedChannels
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn slack_mounts_hide_channel_admin_action_from_operator_user_without_operator_capability()
    {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "slack-webui-tenant".to_string(),
                agent_id: "slack-webui-agent".to_string(),
                source_binding_id: "slack-webui-source".to_string(),
                reply_target_binding_id: "slack-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_slack_connectable_channel(
            &runtime,
            None,
            SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(SLACK_OPERATOR_TENANT).expect("tenant"),
            UserId::new(SLACK_OPERATOR_USER).expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        );

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 1);
        assert_eq!(
            response.channels[0].strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn slack_mounts_hide_channel_admin_action_from_non_operator_callers() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "slack-webui-tenant".to_string(),
                agent_id: "slack-webui-agent".to_string(),
                source_binding_id: "slack-webui-source".to_string(),
                reply_target_binding_id: "slack-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_slack_connectable_channel(
            &runtime,
            None,
            SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(SLACK_OPERATOR_TENANT).expect("tenant"),
            UserId::new("user:not-operator").expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        )
        .with_operator_webui_config(true);

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 1);
        assert_eq!(
            response.channels[0].strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn slack_mounts_hide_channel_admin_action_from_cross_tenant_operator_user() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "slack-webui-tenant".to_string(),
                agent_id: "slack-webui-agent".to_string(),
                source_binding_id: "slack-webui-source".to_string(),
                reply_target_binding_id: "slack-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_slack_connectable_channel(
            &runtime,
            None,
            SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("tenant:other").expect("tenant"),
            UserId::new(SLACK_OPERATOR_USER).expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        )
        .with_operator_webui_config(true);

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 1);
        assert_eq!(
            response.channels[0].strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn slack_mounts_without_operator_action_advertise_personal_pairing_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "slack-webui-tenant".to_string(),
                agent_id: "slack-webui-agent".to_string(),
                source_binding_id: "slack-webui-source".to_string(),
                reply_target_binding_id: "slack-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_slack_connectable_channel(
            &runtime,
            None,
            SlackConnectableChannelVisibility::PersonalPairing,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(SLACK_OPERATOR_TENANT).expect("tenant"),
            UserId::new(SLACK_OPERATOR_USER).expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        )
        .with_operator_webui_config(true);

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 1);
        assert_eq!(
            response.channels[0].strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[derive(Debug)]
    struct StaticGateway;

    #[async_trait::async_trait]
    impl HostManagedModelGateway for StaticGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Ok(HostManagedModelResponse::assistant_reply("ok"))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            _capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.stream_model(request).await
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
            if provider != SLACK_IDENTITY_PROVIDER {
                return Ok(false);
            }
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .values()
                .any(|bound_user_id| bound_user_id == user_id))
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
}
