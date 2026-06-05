use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, StaticConnectableChannelsProductFacade,
};

use crate::{
    RebornBuildError, RebornRuntime, RebornWebuiBundle, SlackHostBetaMounts,
    webui::build_webui_services_with_connectable_channels,
};

pub fn build_webui_services_with_slack_host_beta_mounts(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    slack_mounts: Option<&SlackHostBetaMounts>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    build_webui_services_with_slack_connectable_channel(
        runtime,
        event_stream,
        slack_mounts.is_some(),
    )
}

fn build_webui_services_with_slack_connectable_channel(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    slack_pairing_mounted: bool,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let connectable_channels = slack_pairing_mounted.then(|| {
        Arc::new(StaticConnectableChannelsProductFacade::new(vec![
            slack_personal_binding_pairing_connectable_channel(),
        ])) as Arc<dyn ConnectableChannelsProductFacade>
    });
    build_webui_services_with_connectable_channels(runtime, event_stream, connectable_channels)
}

fn slack_personal_binding_pairing_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::InboundProofCode,
        action: RebornChannelConnectAction {
            title: "Slack account connection".to_string(),
            instructions: "Message the Slack app, then enter the code here.".to_string(),
            code_placeholder: "Enter Slack pairing code...".to_string(),
            submit_label: "Connect".to_string(),
            success_message: "Slack account connected.".to_string(),
            error_message: "Invalid or expired Slack pairing code.".to_string(),
        },
        command_aliases: vec!["slack".to_string(), "slack account".to_string()],
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

    #[test]
    fn slack_pairing_connectable_channel_matches_pairing_flow_copy() {
        let channel = slack_personal_binding_pairing_connectable_channel();

        assert_eq!(channel.channel, "slack");
        assert_eq!(
            channel.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert_eq!(
            channel.action.instructions,
            "Message the Slack app, then enter the code here."
        );
        assert_eq!(
            channel.command_aliases,
            vec!["slack".to_string(), "slack account".to_string()]
        );
    }

    #[tokio::test]
    async fn slack_mounts_inject_pairing_channel_into_webui_facade() {
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
        let bundle = build_webui_services_with_slack_connectable_channel(&runtime, None, true)
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

        let channel = response.channels.first().expect("slack channel");
        assert_eq!(channel.channel, "slack");
        assert_eq!(
            channel.strategy,
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
