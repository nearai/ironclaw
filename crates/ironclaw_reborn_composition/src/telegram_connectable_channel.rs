//! Telegram connectable-channel advertisement for the WebChat v2 sidecar.
//!
//! Surfaces Telegram in `GET /api/webchat/v2/channels/connectable` so the WebUI
//! can drive the pairing flow. Telegram bots cannot message a user first, so the
//! strategy is inbound-proof-code: the user opens the bot, sends `/start` (or any
//! message), receives a pairing code, and pastes it into IronClaw.

use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, StaticConnectableChannelsProductFacade,
};

use crate::webui::build_webui_services_with_connectable_channels;
use crate::{RebornBuildError, RebornRuntime, RebornWebuiBundle};

/// The Telegram connectable channel descriptor (inbound proof-code pairing).
pub fn telegram_inbound_proof_code_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "telegram".to_string(),
        display_name: "Telegram".to_string(),
        strategy: RebornChannelConnectStrategy::InboundProofCode,
        action: RebornChannelConnectAction {
            title: "Telegram account connection".to_string(),
            instructions: "Open your bot in Telegram, send /start (or any message), \
                then enter the pairing code it replies with here."
                .to_string(),
            input_placeholder: "Enter Telegram pairing code...".to_string(),
            submit_label: "Connect".to_string(),
            success_message: "Telegram account connected.".to_string(),
            error_message: "Invalid or expired Telegram pairing code.".to_string(),
        },
        command_aliases: vec![
            "telegram".to_string(),
            "telegram account".to_string(),
            "telegram pairing".to_string(),
        ],
    }
}

/// Build the WebUI bundle advertising the Telegram connectable channel when the
/// Telegram host route is mounted. Used on the Slack-compiled-out path; when
/// Slack is present, Telegram channels are merged into the single Slack facade
/// instead (the connectable builder accepts exactly one facade).
pub fn build_webui_services_with_telegram_connectable_channel(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    telegram_enabled: bool,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let connectable = telegram_enabled.then(|| {
        Arc::new(StaticConnectableChannelsProductFacade::new(vec![
            telegram_inbound_proof_code_connectable_channel(),
        ])) as Arc<dyn ConnectableChannelsProductFacade>
    });
    build_webui_services_with_connectable_channels(runtime, event_stream, connectable, Vec::new())
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

    #[tokio::test]
    async fn telegram_enabled_advertises_connectable_channel_through_webui_facade() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("tg-webui-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "tg-webui-tenant".to_string(),
                agent_id: "tg-webui-agent".to_string(),
                source_binding_id: "tg-webui-source".to_string(),
                reply_target_binding_id: "tg-webui-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_telegram_connectable_channel(&runtime, None, true)
            .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("tg-webui-tenant").expect("tenant"),
            UserId::new("tg-webui-owner").expect("user"),
            Some(AgentId::new("tg-webui-agent").expect("agent")),
            None,
        );

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert_eq!(response.channels.len(), 1);
        assert_eq!(response.channels[0].channel, "telegram");
        assert_eq!(
            response.channels[0].strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn telegram_disabled_advertises_no_connectable_channel() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("tg-off-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "tg-off-tenant".to_string(),
                agent_id: "tg-off-agent".to_string(),
                source_binding_id: "tg-off-source".to_string(),
                reply_target_binding_id: "tg-off-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let bundle = build_webui_services_with_telegram_connectable_channel(&runtime, None, false)
            .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("tg-off-tenant").expect("tenant"),
            UserId::new("tg-off-owner").expect("user"),
            Some(AgentId::new("tg-off-agent").expect("agent")),
            None,
        );

        let response = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");

        assert!(response.channels.is_empty());

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

    #[test]
    fn telegram_connectable_channel_matches_inbound_proof_code_copy() {
        let channel = telegram_inbound_proof_code_connectable_channel();

        assert_eq!(channel.channel, "telegram");
        assert_eq!(channel.display_name, "Telegram");
        assert_eq!(
            channel.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert!(channel.action.instructions.contains("/start"));
        assert_eq!(
            channel.command_aliases,
            vec![
                "telegram".to_string(),
                "telegram account".to_string(),
                "telegram pairing".to_string(),
            ]
        );
    }
}
