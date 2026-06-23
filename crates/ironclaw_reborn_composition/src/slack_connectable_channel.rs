use std::sync::Arc;

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, RebornConnectableChannelListResponse, RebornServicesError,
    WebUiAuthenticatedCaller,
};

use crate::{
    RebornBuildError, RebornRuntime, RebornWebuiBundle, SlackHostBetaMounts,
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
    build_webui_services_with_connectable_channels(
        runtime,
        event_stream,
        connectable_channels,
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
            && caller.tenant_id == self.tenant_id
            && caller.user_id == self.operator_user_id
        {
            channels.push(slack_admin_managed_channel_connectable_channel());
        }
        Ok(RebornConnectableChannelListResponse { channels })
    }
}

fn slack_inbound_proof_code_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::InboundProofCode,
        action: RebornChannelConnectAction {
            title: "Slack account connection".to_string(),
            instructions: "Message the Slack app, then enter the code here.".to_string(),
            input_placeholder: "Enter Slack pairing code...".to_string(),
            submit_label: "Connect".to_string(),
            success_message: "Slack account connected.".to_string(),
            error_message: "Invalid or expired Slack pairing code.".to_string(),
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
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;
    use ironclaw_turns::run_profile::LoopCapabilityPort;

    use super::*;
    use crate::{
        RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
        local_dev_runtime_policy,
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
            channel.command_aliases,
            vec![
                "slack".to_string(),
                "slack account".to_string(),
                "slack pairing".to_string()
            ]
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
        );

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
}
