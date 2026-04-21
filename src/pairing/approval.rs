//! Pairing approval orchestration.
//!
//! Propagates an approved pairing to the running WASM channel:
//! persists the numeric owner ID (if applicable), updates runtime
//! config, sets the owner actor ID, and restarts polling.

use std::collections::HashMap;
use std::sync::Arc;

use crate::channels::wasm::{
    RUNTIME_CONFIG_KEY_OWNER_ID, RUNTIME_CONFIG_KEY_TUNNEL_URL, RUNTIME_CONFIG_KEY_WEBHOOK_SECRET,
    WasmChannel,
};
use crate::extensions::ExtensionError;
use crate::pairing::ExternalId;

/// Dependencies needed to propagate a pairing approval to a running channel.
pub struct ApprovalDeps<'a> {
    pub tunnel_url: Option<&'a str>,
    pub store: Option<&'a Arc<dyn crate::db::Database>>,
    pub user_id: &'a str,
    pub config_overrides: HashMap<String, serde_json::Value>,
}

/// Propagate an approved pairing to a running WASM channel.
///
/// This is the core orchestration after `PairingStore::approve()` returns a
/// valid [`ExternalId`]. It:
///
/// 1. Persists the numeric owner_id (if the external_id is numeric, e.g. Telegram)
/// 2. Sets the string-based `owner_actor_id` on the running channel
/// 3. Rebuilds runtime config with tunnel URL and owner ID
/// 4. Calls `on_start()` to restart polling with the new owner binding
pub async fn propagate_approval(
    channel: &Arc<WasmChannel>,
    channel_name: &str,
    external_id: &ExternalId,
    deps: &ApprovalDeps<'_>,
) -> Result<(), ExtensionError> {
    let numeric_id: Option<i64> = external_id.as_str().parse().ok();
    let previous_owner_actor_id = channel.owner_actor_id().await;
    let previous_config_json = channel.config_json_snapshot().await;

    // Update the live channel binding first, but restore it if the restart
    // path fails so the running channel never keeps a claimed owner that the
    // durable pairing store has already rolled back.
    channel
        .set_owner_actor_id(Some(external_id.as_str().to_string()))
        .await;

    let mut config_updates =
        build_runtime_config_updates(deps.tunnel_url, None, Some(external_id.as_str()));
    config_updates.extend(deps.config_overrides.clone());

    if !config_updates.is_empty() {
        channel.update_config(config_updates).await;
    }

    let config = match channel.call_on_start().await {
        Ok(config) => config,
        Err(e) => {
            // Restore the live runtime view before returning so a failed
            // pairing propagation cannot leave the running channel trusting the
            // new owner while the durable approval row is rolled back.
            channel
                .restore_runtime_state(previous_owner_actor_id, previous_config_json)
                .await;
            tracing::warn!(
                channel = %channel_name,
                error = %e,
                "on_start failed after owner binding propagation"
            );
            return Err(ExtensionError::ActivationFailed(e.to_string()));
        }
    };

    channel.ensure_polling(&config).await;

    // Persist numeric owner_id only after runtime propagation succeeds. This
    // keeps settings DB, live runtime, and pairing approval state aligned.
    if let Some(owner_id_numeric) = numeric_id {
        if let Err(e) =
            persist_numeric_owner_id(deps.store, deps.user_id, channel_name, owner_id_numeric).await
        {
            tracing::debug!(
                channel = %channel_name,
                error = %e,
                "Failed to persist numeric owner_id (non-critical)"
            );
        }
    } else {
        tracing::debug!(
            channel = %channel_name,
            external_id = %external_id,
            "Non-numeric external_id, skipping numeric owner_id persistence"
        );
    }

    tracing::debug!(
        channel = %channel_name,
        external_id = %external_id,
        "Propagated owner binding to running channel and restarted polling"
    );

    Ok(())
}

/// Build a map of runtime config updates for a WASM channel.
pub(crate) fn build_runtime_config_updates(
    tunnel_url: Option<&str>,
    webhook_secret: Option<&str>,
    owner_actor_id: Option<&str>,
) -> HashMap<String, serde_json::Value> {
    let mut config_updates = HashMap::new();

    if let Some(tunnel_url) = tunnel_url {
        config_updates.insert(
            RUNTIME_CONFIG_KEY_TUNNEL_URL.to_string(),
            serde_json::Value::String(tunnel_url.to_string()),
        );
    }

    if let Some(secret) = webhook_secret {
        config_updates.insert(
            RUNTIME_CONFIG_KEY_WEBHOOK_SECRET.to_string(),
            serde_json::Value::String(secret.to_string()),
        );
    }

    if let Some(owner_actor_id) = owner_actor_id {
        let owner_id_value = owner_actor_id
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::Value::String(owner_actor_id.to_string()));
        config_updates.insert(RUNTIME_CONFIG_KEY_OWNER_ID.to_string(), owner_id_value);
    }

    config_updates
}

/// Persist the numeric owner ID to settings DB.
async fn persist_numeric_owner_id(
    store: Option<&Arc<dyn crate::db::Database>>,
    user_id: &str,
    channel_name: &str,
    owner_id: i64,
) -> Result<(), ExtensionError> {
    if let Some(store) = store {
        store
            .set_setting(
                user_id,
                &format!("channels.wasm_channel_owner_ids.{channel_name}"),
                &serde_json::json!(owner_id),
            )
            .await
            .map_err(|e| ExtensionError::Config(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ApprovalDeps, build_runtime_config_updates, propagate_approval};
    use crate::channels::wasm::{
        ChannelCapabilitiesFile, WasmChannel, WasmChannelRuntime, WasmChannelRuntimeConfig,
        is_reserved_runtime_config_key,
    };
    use crate::pairing::{ExternalId, PairingStore};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn telegram_wasm_path() -> Option<PathBuf> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            manifest_dir
                .join("channels-src/telegram/target/wasm32-wasip2/release/telegram_channel.wasm"),
            manifest_dir.join("channels-src/telegram/telegram.wasm"),
        ];
        candidates.into_iter().find(|path| path.exists())
    }

    fn telegram_capabilities_path() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("channels-src/telegram/telegram.capabilities.json");
        assert!(path.exists(), "telegram capabilities fixture not found");
        path
    }

    #[test]
    fn runtime_config_updates_only_emit_reserved_host_keys() {
        let updates = build_runtime_config_updates(
            Some("https://example.test"),
            Some("webhook-secret"),
            Some("12345"),
        );

        assert_eq!(updates.len(), 3);
        for key in updates.keys() {
            assert!(
                is_reserved_runtime_config_key(key),
                "runtime config key {key} must be blocked from secret_config_mappings"
            );
        }
    }

    // `#[tokio::test]` without a `flavor` argument uses the current-thread
    // runtime, so holding `ENV_MUTEX` across awaits here cannot deadlock
    // other tasks — there are none scheduled concurrently on this
    // runtime. The serialization guarantee against sibling tests
    // reading `IRONCLAW_TEST_TELEGRAM_API_BASE_URL` is worth the lint
    // suppression.
    #[tokio::test(flavor = "current_thread")]
    #[ignore] // requires prebuilt telegram WASM binary (not checked in, *.wasm is gitignored)
    #[allow(clippy::await_holding_lock)]
    async fn propagate_approval_restores_runtime_state_when_on_start_fails() {
        // Hold ENV_MUTEX for the duration so the runtime-env overlay mutation
        // here cannot race with other tests reading IRONCLAW_TEST_TELEGRAM_API_BASE_URL
        // (e.g. extensions::manager::tests::test_telegram_token_colon_preserved_in_validation_url).
        let _env_lock = crate::config::helpers::lock_env();
        let original =
            crate::config::helpers::env_or_override("IRONCLAW_TEST_TELEGRAM_API_BASE_URL");
        crate::config::helpers::set_runtime_env(
            "IRONCLAW_TEST_TELEGRAM_API_BASE_URL",
            "http://127.0.0.1:1",
        );

        let Some(telegram_wasm_path) = telegram_wasm_path() else {
            crate::config::helpers::set_runtime_env(
                "IRONCLAW_TEST_TELEGRAM_API_BASE_URL",
                original.as_deref().unwrap_or(""),
            );
            return;
        };

        let runtime = Arc::new(
            WasmChannelRuntime::new(WasmChannelRuntimeConfig::for_testing()).expect("runtime"),
        );
        let wasm_bytes = std::fs::read(telegram_wasm_path).expect("read telegram wasm");
        let prepared = runtime
            .prepare(
                "telegram",
                &wasm_bytes,
                None,
                Some("Telegram Bot API channel".to_string()),
            )
            .await
            .expect("prepare telegram module");
        let capabilities_bytes =
            std::fs::read(telegram_capabilities_path()).expect("read telegram capabilities");
        let capabilities = ChannelCapabilitiesFile::from_bytes(&capabilities_bytes)
            .expect("parse capabilities")
            .to_capabilities();

        let channel = Arc::new(
            WasmChannel::new(
                runtime,
                prepared,
                capabilities,
                "default",
                r#"{"owner_id":"old-owner","dm_policy":"pairing"}"#.to_string(),
                Arc::new(PairingStore::new_noop()),
                None,
            )
            .with_owner_actor_id(Some("old-owner".to_string())),
        );
        channel
            .set_credential("TELEGRAM_BOT_TOKEN", "123456:ABCDEF".to_string())
            .await;

        let original_owner = channel.owner_actor_id_for_test().await;
        let original_config = channel.config_json_snapshot().await;
        let deps = ApprovalDeps {
            tunnel_url: None,
            store: None,
            user_id: "default",
            config_overrides: HashMap::new(),
        };

        let result = propagate_approval(
            &channel,
            "telegram",
            &ExternalId::from("new-owner".to_string()),
            &deps,
        )
        .await;

        assert!(matches!(
            result,
            Err(crate::extensions::ExtensionError::ActivationFailed(_))
        ));
        assert_eq!(channel.owner_actor_id_for_test().await, original_owner);
        assert_eq!(channel.config_json_snapshot().await, original_config);

        crate::config::helpers::set_runtime_env(
            "IRONCLAW_TEST_TELEGRAM_API_BASE_URL",
            original.as_deref().unwrap_or(""),
        );
    }
}
