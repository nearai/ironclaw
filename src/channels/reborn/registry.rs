//! Single entry point invoked by `src/main.rs` to register Reborn channels.
//!
//! All Reborn-related orchestration lives here: secrets reads, storage
//! construction, adapter + workflow + runner assembly, axum route mounting,
//! `Channel` registration. `src/main.rs` only knows this function's name and
//! shape; it doesn't import Reborn crate types directly. This keeps the v1
//! agent binary clean of Reborn-specific concerns at the call site (per
//! @serrrfirat's review on PR #3590).

use std::sync::Arc;

use crate::channels::ChannelManager;
use crate::db::DatabaseHandles;
use crate::secrets::SecretsStore;

use super::boot::{TELEGRAM_V2_CHANNEL_NAME, bootstrap_telegram_v2};

/// Reborn channel wiring inputs — flat so callers don't have to hand over
/// the whole `AppComponents`. Keeps `src/main.rs` independent of which
/// fields the Reborn wiring actually reads.
pub struct RebornChannelWiringInputs<'a> {
    pub enable_non_cli: bool,
    pub reborn_telegram_v2_enabled: bool,
    pub owner_id: &'a str,
    pub database_handles: Option<&'a DatabaseHandles>,
    pub secrets_store: Option<&'a Arc<dyn SecretsStore + Send + Sync>>,
}

/// Optionally wire every Reborn channel implementation gated by its own
/// config flag. Today this is just Telegram v2; future channels add their
/// own arms here. Never crashes — every failure path is a soft warn so the
/// rest of the binary still starts.
pub async fn register_reborn_channels(
    inputs: RebornChannelWiringInputs<'_>,
    channels: &ChannelManager,
    webhook_routes: &mut Vec<axum::Router>,
    channel_names: &mut Vec<String>,
) {
    if !inputs.enable_non_cli {
        return;
    }
    if inputs.reborn_telegram_v2_enabled {
        register_telegram_v2(&inputs, channels, webhook_routes, channel_names).await;
    }
    // Future Reborn channels go here.
}

async fn register_telegram_v2(
    inputs: &RebornChannelWiringInputs<'_>,
    channels: &ChannelManager,
    webhook_routes: &mut Vec<axum::Router>,
    channel_names: &mut Vec<String>,
) {
    let installation_id_str = std::env::var("REBORN_TELEGRAM_V2_INSTALLATION_ID")
        .unwrap_or_else(|_| "default".to_string());
    let (handles, secrets_store) = match (inputs.database_handles, inputs.secrets_store) {
        (Some(handles), Some(secrets_store)) => (handles, secrets_store),
        _ => {
            tracing::warn!(
                "REBORN_TELEGRAM_V2_ENABLED=true requires a database backend and secrets \
                 store; skipping v2 wiring"
            );
            return;
        }
    };

    match bootstrap_telegram_v2(
        handles,
        secrets_store,
        inputs.owner_id,
        &installation_id_str,
    )
    .await
    {
        Ok(Some(bootstrap)) => {
            webhook_routes.push(bootstrap.routes);
            channel_names.push(TELEGRAM_V2_CHANNEL_NAME.to_string());
            channels.add(bootstrap.channel).await;
        }
        Ok(None) => {
            tracing::warn!(
                "REBORN_TELEGRAM_V2_ENABLED=true but required secrets are missing; \
                 skipping v2 wiring"
            );
        }
        Err(err) => {
            tracing::error!(error = %err, "Reborn Telegram v2 bootstrap failed");
        }
    }
}
