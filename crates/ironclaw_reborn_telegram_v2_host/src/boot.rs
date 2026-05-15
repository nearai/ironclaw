//! Boot-time orchestration for the Reborn Telegram v2 host.
//!
//! Stitches together: storage layer (`composition`), the stubbed inbound turn
//! service, the Telegram adapter + workflow + native runner, and the axum
//! router. Returns the router for the crate's top-level `serve` helper to
//! hand off to `axum::serve`.

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
use secrecy::ExposeSecret;

use crate::composition::{
    BackendHandles, RebornProductRuntimeConfig, build_reborn_product_runtime,
};
use crate::config::HostConfig;
use crate::error::HostError;
use crate::inbound_turn::StubInboundTurnService;
use crate::router::{TelegramV2RouterState, telegram_v2_routes};

pub const TELEGRAM_V2_CHANNEL_NAME: &str = "telegram_v2";
/// Credential handle name — kept in lockstep with v1's `telegram_bot_token`
/// secret naming so the bot config carries over if operators migrate.
const TELEGRAM_BOT_TOKEN_HANDLE: &str = "telegram_bot_token";
/// Header Telegram uses to deliver the webhook shared secret.
const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

pub struct BootArtifacts {
    pub router: axum::Router,
}

pub async fn boot(
    handles: BackendHandles,
    config: &HostConfig,
) -> Result<BootArtifacts, HostError> {
    let installation_id = AdapterInstallationId::new(&config.installation_id).map_err(|e| {
        HostError::Startup(format!(
            "invalid installation id '{}': {e}",
            config.installation_id
        ))
    })?;
    let adapter_id = ProductAdapterId::new("telegram_v2")
        .map_err(|e| HostError::Startup(format!("invalid adapter id: {e}")))?;
    let credential_handle = EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_HANDLE)
        .map_err(|e| HostError::Startup(format!("invalid credential handle: {e}")))?;
    let default_tenant_id = TenantId::new(&config.tenant_id)
        .map_err(|e| HostError::Startup(format!("invalid tenant id: {e}")))?;
    let default_agent_id = AgentId::new(&config.agent_id)
        .map_err(|e| HostError::Startup(format!("invalid agent id: {e}")))?;

    // Expose at the boundary into the storage crate — the resolver still
    // holds a plain `String`, so the residual exposure documented on
    // `HostConfig::telegram_bot_token` applies here too.
    let runtime = build_reborn_product_runtime(
        handles,
        RebornProductRuntimeConfig {
            default_tenant_id: default_tenant_id.clone(),
            default_agent_id: default_agent_id.clone(),
            telegram_bot_token: config.telegram_bot_token.expose_secret().to_string(),
            telegram_credential_handle: credential_handle.clone(),
            telegram_declared_hosts: telegram_declared_egress_hosts(),
        },
    )
    .await?;

    let (bot_username, bot_user_id) = match fetch_bot_identity(
        config.telegram_bot_token.expose_secret(),
    )
    .await
    {
        Ok(identity) => {
            tracing::info!(
                bot_user_id = identity.id,
                bot_username = %identity.username,
                "Reborn host: resolved bot identity via getMe"
            );
            (identity.username, identity.id)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Reborn host: getMe failed; falling back to placeholder bot identity. Group-chat triggers may misclassify until restart."
            );
            // 0 is reserved as the sentinel — Telegram never assigns id=0 to a
            // real bot, so misclassification stays bounded to the placeholder.
            ("ironclaw_telegram_v2_unknown".to_string(), 0)
        }
    };

    let adapter = TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: installation_id.clone(),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username,
            bot_user_id,
            recognized_commands: vec!["start".into(), "help".into()],
        },
        egress_credential_handle: credential_handle.clone(),
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        progress_push_enabled: false,
    });
    let adapter_arc = Arc::new(adapter);

    let inbound_turn_service = StubInboundTurnService::new(Arc::clone(&runtime.binding));
    let workflow =
        DefaultProductWorkflow::new(Arc::new(inbound_turn_service), Arc::clone(&runtime.ledger));
    let auth = WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
        header_name: TELEGRAM_SECRET_HEADER.into(),
        // `SharedSecretHeaderAuth` keeps the expected secret as a plain
        // `String` inside the runner for the lifetime of this process; see
        // the residual-exposure note on `HostConfig::telegram_webhook_secret`.
        expected_secret: config.telegram_webhook_secret.expose_secret().to_string(),
        subject: format!("telegram_v2:{}", config.installation_id),
    });
    let runner_config = NativeProductAdapterRunnerConfig::new(
        Duration::from_secs(15),
        NonZeroUsize::new(64).expect("64 > 0"), // safety: literal 64 is provably non-zero
    );
    let runner = NativeProductAdapterRunner::with_config(
        adapter_arc,
        Arc::new(workflow),
        auth,
        runner_config,
    );

    let mut runners: HashMap<String, Arc<NativeProductAdapterRunner>> = HashMap::new();
    runners.insert(installation_id.as_str().to_string(), Arc::new(runner));
    let router_state = TelegramV2RouterState {
        runners: Arc::new(runners),
    };
    let router = telegram_v2_routes(router_state);

    tracing::info!(
        installation = %config.installation_id,
        channel = TELEGRAM_V2_CHANNEL_NAME,
        "Reborn Telegram v2 host wired"
    );

    Ok(BootArtifacts { router })
}

struct BotIdentity {
    id: i64,
    username: String,
}

async fn fetch_bot_identity(bot_token: &str) -> Result<BotIdentity, String> {
    let client = reqwest::Client::builder()
        .user_agent("ironclaw-reborn-telegram-v2/0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build reqwest client: {}", scrub(e)))?;
    // Telegram's only auth method is path-embedded bot tokens. reqwest's
    // `Error::Display` includes the request URL by default, so a DNS/TLS/
    // connect failure formatted with `{e}` writes the bot token into logs.
    // `.without_url()` strips the URL from every error returned downstream
    // (zmanian / Henry's review on PR #3590, "Concerning" #2 — token leak
    // through error-stringification). Body-related errors don't carry a
    // URL, but we use the same scrubber uniformly to avoid drift.
    let url = format!("https://api.telegram.org/bot{bot_token}/getMe");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("getMe request failed: {}", scrub(e)))?;
    if !response.status().is_success() {
        return Err(format!("getMe returned HTTP {}", response.status()));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("parse getMe response: {}", scrub(e)))?;
    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!("getMe response not ok: {body}"));
    }
    let result = body
        .get("result")
        .ok_or_else(|| "getMe response missing 'result'".to_string())?;
    let id = result
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "getMe 'result.id' missing or not an integer".to_string())?;
    let username = result
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "getMe 'result.username' missing or not a string".to_string())?
        .to_string();
    Ok(BotIdentity { id, username })
}

/// Format a `reqwest::Error` with its URL stripped — `.without_url()` mutates
/// the error to drop the `url` field that `Display` would otherwise include,
/// which for Telegram's bot API leaks the token path segment.
fn scrub(err: reqwest::Error) -> String {
    err.without_url().to_string()
}
