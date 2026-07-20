//! Telegram entries for the WebUI connectable-channels surface + the
//! per-user channel-connection facade.
//!
//! Two audiences: the operator sees the admin bot-setup card
//! (`admin_managed_channels`); every same-tenant member sees the pairing
//! connect action (`web_generated_code`) once the bot is configured. The
//! connection facade reports per-caller pairedness under the `"telegram"`
//! key and services disconnect (unpair).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_workflow::{
    ChannelConnectionFacade, ConnectableChannelsProductFacade, RebornChannelConnectAction,
    RebornChannelConnectStrategy, RebornConnectableChannelInfo,
    RebornConnectableChannelListResponse, RebornServicesError, WebUiAuthenticatedCaller,
};

use crate::pairing::TelegramPairingService;
use crate::setup::TelegramSetupService;

pub struct TelegramConnectableChannelsProductFacade {
    setup: Arc<TelegramSetupService>,
    operator_routes_visible: bool,
}

impl TelegramConnectableChannelsProductFacade {
    pub fn new(setup: Arc<TelegramSetupService>, operator_routes_visible: bool) -> Self {
        Self {
            setup,
            operator_routes_visible,
        }
    }
}

fn telegram_admin_managed_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "telegram".to_string(),
        display_name: "Telegram".to_string(),
        strategy: RebornChannelConnectStrategy::AdminManagedChannels,
        action: RebornChannelConnectAction {
            title: "Telegram bot setup".to_string(),
            instructions: "Provide the bot token from @BotFather. IronClaw validates it and registers the webhook automatically.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Save bot".to_string(),
            success_message: "Telegram bot configured.".to_string(),
            error_message: "Telegram bot setup failed. Check the token and try again.".to_string(),
        },
        command_aliases: Vec::new(),
    }
}

fn telegram_pairing_connectable_channel() -> RebornConnectableChannelInfo {
    RebornConnectableChannelInfo {
        channel: "telegram".to_string(),
        display_name: "Telegram".to_string(),
        strategy: RebornChannelConnectStrategy::WebGeneratedCode,
        action: RebornChannelConnectAction {
            title: "Pair Telegram".to_string(),
            instructions: "Tap the link or scan the QR to open the bot in Telegram, or send the shown code to the bot.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Open pairing".to_string(),
            success_message: "Telegram paired.".to_string(),
            error_message: "Pairing failed — get a fresh code and try again.".to_string(),
        },
        command_aliases: Vec::new(),
    }
}

#[async_trait]
impl ConnectableChannelsProductFacade for TelegramConnectableChannelsProductFacade {
    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        let mut channels = Vec::new();
        let same_tenant = caller.tenant_id == *self.setup.tenant_id();
        if !same_tenant {
            return Ok(RebornConnectableChannelListResponse { channels });
        }
        let operator = self.operator_routes_visible
            && caller.operator_webui_config
            && caller.user_id == *self.setup.operator_user_id();
        if operator {
            channels.push(telegram_admin_managed_channel());
        }
        let configured = self
            .setup
            .status()
            .await
            .map(|status| status.configured)
            .map_err(|error| {
                tracing::debug!(reason = %error, "telegram setup status unavailable");
                RebornServicesError::internal_from(error)
            })?;
        if configured {
            channels.push(telegram_pairing_connectable_channel());
        }
        Ok(RebornConnectableChannelListResponse { channels })
    }
}

pub struct TelegramChannelConnectionFacade {
    pairing: Arc<TelegramPairingService>,
    setup: Arc<TelegramSetupService>,
}

impl TelegramChannelConnectionFacade {
    pub fn new(pairing: Arc<TelegramPairingService>, setup: Arc<TelegramSetupService>) -> Self {
        Self { pairing, setup }
    }
}

#[async_trait]
impl ChannelConnectionFacade for TelegramChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<HashMap<String, bool>, RebornServicesError> {
        if caller.tenant_id != *self.setup.tenant_id() {
            return Ok(HashMap::new());
        }
        let status = self
            .pairing
            .status_for(&caller.user_id)
            .await
            .map_err(|error| {
                tracing::debug!(reason = %error, "telegram pairing status unavailable");
                RebornServicesError::internal_from(error)
            })?;
        Ok(HashMap::from([("telegram".to_string(), status.connected)]))
    }

    async fn disconnect_channel_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        channel: &str,
    ) -> Result<(), RebornServicesError> {
        if channel != "telegram" || caller.tenant_id != *self.setup.tenant_id() {
            return Err(RebornServicesError::internal());
        }
        self.pairing.unpair(&caller.user_id).await.map_err(|error| {
            tracing::debug!(reason = %error, "telegram unpair failed");
            RebornServicesError::internal_from(error)
        })
    }
}
