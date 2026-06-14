use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornChannelConnectionStatus, RebornConnectableChannelInfo,
    RebornConnectableChannelListResponse, RebornServicesError, WebUiAuthenticatedCaller,
};

use crate::{
    RebornBuildError, RebornRuntime, RebornWebuiBundle, SlackHostBetaMounts,
    slack_outbound_targets::SlackDeliveryConnectionProvider,
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
    let slack_delivery_connections =
        slack_mounts.map(|mounts| Arc::clone(&mounts.delivery_connection_provider));
    let outbound_delivery_target_providers = slack_mounts
        .map(|mounts| vec![Arc::clone(&mounts.outbound_delivery_target_provider)])
        .unwrap_or_default();
    build_webui_services_with_connectable_channels(
        runtime,
        event_stream,
        slack_connectable_channels(visibility, slack_delivery_connections),
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
        slack_connectable_channels(visibility, None),
        Vec::new(),
    )
}

fn slack_connectable_channels(
    visibility: SlackConnectableChannelVisibility,
    delivery_connections: Option<Arc<dyn SlackDeliveryConnectionProvider>>,
) -> Option<Arc<dyn ConnectableChannelsProductFacade>> {
    (visibility != SlackConnectableChannelVisibility::Hidden).then(|| {
        let mut channels = vec![slack_inbound_proof_code_connectable_channel()];
        if visibility == SlackConnectableChannelVisibility::PersonalPairingAndAdminChannelManagement
        {
            channels.push(slack_admin_managed_channel_connectable_channel());
        }
        Arc::new(SlackConnectableChannelsProductFacade::new(
            channels,
            delivery_connections,
        )) as Arc<dyn ConnectableChannelsProductFacade>
    })
}

struct SlackConnectableChannelsProductFacade {
    channels: Arc<[RebornConnectableChannelInfo]>,
    delivery_connections: Option<Arc<dyn SlackDeliveryConnectionProvider>>,
}

impl SlackConnectableChannelsProductFacade {
    fn new(
        channels: impl Into<Vec<RebornConnectableChannelInfo>>,
        delivery_connections: Option<Arc<dyn SlackDeliveryConnectionProvider>>,
    ) -> Self {
        Self {
            channels: Arc::from(channels.into()),
            delivery_connections,
        }
    }

    async fn has_delivery_connection(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<bool, RebornServicesError> {
        let Some(delivery_connections) = &self.delivery_connections else {
            return Ok(false);
        };
        delivery_connections.has_delivery_connection(caller).await
    }
}

#[async_trait::async_trait]
impl ConnectableChannelsProductFacade for SlackConnectableChannelsProductFacade {
    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        let connected = self.has_delivery_connection(&caller).await?;
        Ok(RebornConnectableChannelListResponse {
            channels: self
                .channels
                .iter()
                .cloned()
                .map(|mut channel| {
                    if connected
                        && channel.strategy == RebornChannelConnectStrategy::InboundProofCode
                    {
                        channel.connection_status = RebornChannelConnectionStatus::Connected;
                    }
                    channel
                })
                .collect(),
        })
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
        connection_status: RebornChannelConnectionStatus::Disconnected,
    }
}

fn slack_admin_managed_channel_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::AdminManagedChannels,
        action: RebornChannelConnectAction {
            title: "Slack team agents".to_string(),
            instructions: "Map Slack channels to the team agents that should answer there."
                .to_string(),
            input_placeholder: "C0123456789".to_string(),
            submit_label: "Save channels".to_string(),
            success_message: "Slack channels saved.".to_string(),
            error_message: "Slack channel update failed.".to_string(),
        },
        command_aliases: vec![],
        connection_status: RebornChannelConnectionStatus::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_product_workflow::{
        RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
    };
    use ironclaw_turns::run_profile::LoopCapabilityPort;

    use super::*;
    use crate::{
        RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
        local_dev_runtime_policy,
    };

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
            "Map Slack channels to the team agents that should answer there."
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
            TenantId::new("slack-webui-tenant").expect("tenant"),
            UserId::new("slack-webui-owner").expect("user"),
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
    async fn slack_mounts_without_operator_routes_advertise_personal_pairing_only() {
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
            TenantId::new("slack-webui-tenant").expect("tenant"),
            UserId::new("slack-webui-owner").expect("user"),
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
    async fn slack_connectable_channels_propagates_delivery_connection_error() {
        let facade = SlackConnectableChannelsProductFacade::new(
            vec![slack_inbound_proof_code_connectable_channel()],
            Some(Arc::new(FailingDeliveryConnectionProvider)),
        );
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("slack-webui-tenant").expect("tenant"),
            UserId::new("slack-webui-owner").expect("user"),
            Some(AgentId::new("slack-webui-agent").expect("agent")),
            None,
        );

        let error = facade
            .list_connectable_channels(caller)
            .await
            .expect_err("delivery lookup errors should reach the WebUI facade");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    struct FailingDeliveryConnectionProvider;

    #[async_trait::async_trait]
    impl SlackDeliveryConnectionProvider for FailingDeliveryConnectionProvider {
        async fn has_delivery_connection(
            &self,
            _caller: &WebUiAuthenticatedCaller,
        ) -> Result<bool, RebornServicesError> {
            Err(RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                retryable: true,
                status_code: 503,
                field: None,
                validation_code: None,
            })
        }
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
