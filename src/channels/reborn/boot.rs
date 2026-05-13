//! Boot-time orchestration for the Reborn Telegram v2 channel.
//!
//! Combines [`composition::build_reborn_product_runtime`] with the channel
//! + workflow + runner construction so `src/main.rs` has a single call site.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, EgressCredentialHandle, ProductAdapterId,
};
use ironclaw_product_workflow::DefaultProductWorkflow;
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
};
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth,
    WebhookAuth,
};

use crate::channels::channel::Channel;
use crate::db::DatabaseHandles;
use crate::error::ChannelError;
use crate::secrets::SecretsStore;

use super::composition::{RebornProductRuntimeConfig, build_reborn_product_runtime};
use super::product_channel::{ProductChannel, ProductChannelConfig};
use super::v2_inbound_turn::V2InboundTurnService;
use super::v2_router::TelegramV2RouterState;

/// Channel name reported by the synthetic Telegram v2 product channel.
pub const TELEGRAM_V2_CHANNEL_NAME: &str = "telegram_v2";

/// Secret name shared with v1 — flipping the v2 flag on an install that
/// already has v1 configured does not require new secrets.
const TELEGRAM_BOT_TOKEN_SECRET: &str = "telegram_bot_token";
const TELEGRAM_WEBHOOK_SECRET: &str = "telegram_webhook_secret";

/// Header name Telegram uses to deliver the webhook shared secret.
const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

/// Returned to `src/main.rs` boot wiring. The caller pushes `routes` into the
/// webhook server's accumulator and `channel` into the `ChannelManager`.
pub struct TelegramV2Bootstrap {
    pub routes: axum::Router,
    pub channel: Box<dyn Channel>,
}

/// Materialize the v2 runtime against the running binary's database and
/// secrets stores. Returns `Ok(None)` if no installations are configured
/// (i.e. required secrets are missing), so the caller can skip wiring.
pub async fn bootstrap_telegram_v2(
    handles: &DatabaseHandles,
    secrets_store: &Arc<dyn SecretsStore + Send + Sync>,
    owner_id: &str,
    installation_id_str: &str,
) -> Result<Option<TelegramV2Bootstrap>, ChannelError> {
    // 1. Required secrets. Missing secrets → skip wiring (operator hasn't
    // configured them yet; the binary should still start cleanly).
    let bot_token = match secrets_store
        .get_decrypted(owner_id, TELEGRAM_BOT_TOKEN_SECRET)
        .await
    {
        Ok(secret) => secret.expose().to_string(),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Reborn Telegram v2 enabled but '{}' secret missing; skipping wiring",
                TELEGRAM_BOT_TOKEN_SECRET
            );
            return Ok(None);
        }
    };
    let webhook_secret = match secrets_store
        .get_decrypted(owner_id, TELEGRAM_WEBHOOK_SECRET)
        .await
    {
        Ok(secret) => secret.expose().to_string(),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Reborn Telegram v2 enabled but '{}' secret missing; skipping wiring",
                TELEGRAM_WEBHOOK_SECRET
            );
            return Ok(None);
        }
    };

    // 2. Typed identifiers from configuration.
    let installation_id = AdapterInstallationId::new(installation_id_str).map_err(|e| {
        ChannelError::StartupFailed {
            name: TELEGRAM_V2_CHANNEL_NAME.into(),
            reason: format!("invalid installation id '{installation_id_str}': {e}"),
        }
    })?;
    let adapter_id =
        ProductAdapterId::new("telegram_v2").map_err(|e| ChannelError::StartupFailed {
            name: TELEGRAM_V2_CHANNEL_NAME.into(),
            reason: format!("invalid adapter id: {e}"),
        })?;
    let credential_handle =
        EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_SECRET).map_err(|e| {
            ChannelError::StartupFailed {
                name: TELEGRAM_V2_CHANNEL_NAME.into(),
                reason: format!("invalid credential handle: {e}"),
            }
        })?;
    // Reuse v1's owner_id for the default tenant/agent. v1 has a single
    // global agent, so v2 routes everything to the same one.
    let default_tenant_id =
        TenantId::new(format!("tenant_{owner_id}")).map_err(|e| ChannelError::StartupFailed {
            name: TELEGRAM_V2_CHANNEL_NAME.into(),
            reason: format!("invalid tenant id: {e}"),
        })?;
    let default_agent_id =
        AgentId::new(format!("agent_{owner_id}")).map_err(|e| ChannelError::StartupFailed {
            name: TELEGRAM_V2_CHANNEL_NAME.into(),
            reason: format!("invalid agent id: {e}"),
        })?;

    // 3. Storage + transport layer.
    let runtime = build_reborn_product_runtime(
        handles,
        RebornProductRuntimeConfig {
            default_tenant_id: default_tenant_id.clone(),
            default_agent_id: default_agent_id.clone(),
            telegram_bot_token: bot_token,
            telegram_credential_handle: credential_handle.clone(),
            telegram_declared_hosts: telegram_declared_egress_hosts(),
        },
    )
    .await?;

    // 4. Telegram adapter (stateless given config).
    //
    // The tracer uses placeholder bot identity values — group-chat triggers
    // are not validated end-to-end in this slice (DM-only paths exercise
    // the same parse/render code). Real installs replace these via a
    // follow-up that reads bot identity from `getMe` on first activation.
    let adapter = TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: installation_id.clone(),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: "ironclaw_tracer_bot".into(),
            bot_user_id: 0,
            recognized_commands: vec!["start".into(), "help".into()],
        },
        egress_credential_handle: credential_handle.clone(),
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        progress_push_enabled: false,
    });
    let adapter_arc = Arc::new(adapter);

    // 5. Bus between V2InboundTurnService and ProductChannel.
    let product_channel_config = ProductChannelConfig {
        name: TELEGRAM_V2_CHANNEL_NAME.into(),
        adapter: Arc::clone(&adapter_arc),
        egress: Arc::clone(&runtime.egress)
            as Arc<dyn ironclaw_product_adapters::ProtocolHttpEgress>,
        outbound_store: Arc::clone(&runtime.outbound_store),
        default_tenant_id: default_tenant_id.clone(),
        default_agent_id: default_agent_id.clone(),
    };
    let (product_channel, inbound_tx) = ProductChannel::new(product_channel_config);

    // 6. Custom inbound turn service that emits IncomingMessage to the bus.
    let inbound_turn_service = V2InboundTurnService::new(
        Arc::clone(&runtime.binding),
        inbound_tx,
        TELEGRAM_V2_CHANNEL_NAME,
    );

    // 7. Workflow + native runner.
    let workflow =
        DefaultProductWorkflow::new(Arc::new(inbound_turn_service), Arc::clone(&runtime.ledger));
    let auth = WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
        header_name: TELEGRAM_SECRET_HEADER.into(),
        expected_secret: webhook_secret,
        subject: format!("telegram_v2:{installation_id_str}"),
    });
    let runner_config = NativeProductAdapterRunnerConfig::new(
        Duration::from_secs(15),
        NonZeroUsize::new(64).expect("64 > 0"),
    );
    let runner = NativeProductAdapterRunner::with_config(
        adapter_arc,
        Arc::new(workflow),
        auth,
        runner_config,
    );

    // 8. Router state — one runner per installation_id key.
    let mut runners: HashMap<String, Arc<NativeProductAdapterRunner>> = HashMap::new();
    runners.insert(installation_id_str.to_string(), Arc::new(runner));
    let router_state = TelegramV2RouterState {
        runners: Arc::new(runners),
    };
    let routes = super::v2_router::telegram_v2_routes(router_state);

    tracing::info!(
        installation = %installation_id_str,
        channel = TELEGRAM_V2_CHANNEL_NAME,
        "Reborn Telegram v2 wired"
    );

    Ok(Some(TelegramV2Bootstrap {
        routes,
        channel: Box::new(product_channel),
    }))
}
