//! Pairing approval orchestration.
//!
//! Propagates an approved pairing to the running WASM channel:
//! persists the numeric owner ID (if applicable), updates runtime
//! config, sets the owner actor ID, and restarts polling.

use std::collections::HashMap;
use std::sync::Arc;

use crate::channels::wasm::WasmChannel;
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

    // Persist numeric owner_id if the external_id is numeric (Telegram).
    // Non-numeric external IDs (Discord, Slack) skip this but still
    // proceed with the string-based owner_actor_id binding below.
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

    channel
        .set_owner_actor_id(Some(external_id.as_str().to_string()))
        .await;

    let mut config_updates = build_runtime_config_updates(deps.tunnel_url, None, numeric_id);
    config_updates.extend(deps.config_overrides.clone());

    if !config_updates.is_empty() {
        channel.update_config(config_updates).await;
    }

    match channel.call_on_start().await {
        Ok(config) => {
            channel.ensure_polling(&config).await;
            tracing::debug!(
                channel = %channel_name,
                external_id = %external_id,
                "Propagated owner binding to running channel and restarted polling"
            );
        }
        Err(e) => {
            tracing::warn!(
                channel = %channel_name,
                error = %e,
                "on_start failed after owner binding propagation"
            );
        }
    }

    Ok(())
}

/// Build a map of runtime config updates for a WASM channel.
pub(crate) fn build_runtime_config_updates(
    tunnel_url: Option<&str>,
    webhook_secret: Option<&str>,
    owner_id: Option<i64>,
) -> HashMap<String, serde_json::Value> {
    let mut config_updates = HashMap::new();

    if let Some(tunnel_url) = tunnel_url {
        config_updates.insert(
            "tunnel_url".to_string(),
            serde_json::Value::String(tunnel_url.to_string()),
        );
    }

    if let Some(secret) = webhook_secret {
        config_updates.insert(
            "webhook_secret".to_string(),
            serde_json::Value::String(secret.to_string()),
        );
    }

    if let Some(owner_id) = owner_id {
        config_updates.insert("owner_id".to_string(), serde_json::json!(owner_id));
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
