//! WASM channel setup and credential injection.
//!
//! Encapsulates the logic for loading WASM channels, registering their
//! webhook routes, and injecting credentials from the secrets store.
//!
//! # Ownership model
//!
//! Boot-time secret lookups use `config.owner_id` because channels are
//! **instance-level resources** — they run as the instance operator, not as
//! individual users. This is intentional and distinct from tool-level
//! credential resolution, which is scoped to the calling user's `user_id`.
//!
//! See `docs/superpowers/specs/2026-04-01-ownership-model-design.md`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::channels::wasm::{
    LoadedChannel, RegisteredEndpoint, SharedWasmChannel, TELEGRAM_CHANNEL_NAME, WasmChannel,
    WasmChannelLoader, WasmChannelRouter, WasmChannelRuntime, WasmChannelRuntimeConfig,
    bot_username_setting_key, create_wasm_channel_router,
};
use crate::config::Config;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::pairing::PairingStore;
use crate::secrets::SecretsStore;

pub(crate) fn reserved_wasm_channel_names() -> Vec<&'static str> {
    use crate::agent::session::{BOOTSTRAP_SOURCE_CHANNEL, TRUSTED_APPROVAL_CHANNELS};

    let mut reserved: Vec<&str> = vec![
        "cli",
        "repl",
        "http",
        "signal",
        "slack-relay",
        "secret_save",
    ];
    reserved.extend(TRUSTED_APPROVAL_CHANNELS);
    reserved.push(BOOTSTRAP_SOURCE_CHANNEL);
    reserved
}

pub(crate) fn is_reserved_wasm_channel_name(name: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    reserved_wasm_channel_names().contains(&name_lower.as_str())
}

/// Result of WASM channel setup.
pub struct WasmChannelSetup {
    pub channels: Vec<(String, Box<dyn crate::channels::Channel>)>,
    pub channel_names: Vec<String>,
    pub webhook_routes: Option<axum::Router>,
    /// Runtime objects needed for hot-activation via ExtensionManager.
    pub wasm_channel_runtime: Arc<WasmChannelRuntime>,
    pub pairing_store: Arc<PairingStore>,
    pub wasm_channel_router: Arc<WasmChannelRouter>,
}

/// Load WASM channels and register their webhook routes.
pub async fn setup_wasm_channels(
    config: &Config,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    extension_manager: Option<&Arc<ExtensionManager>>,
    database: Option<&Arc<dyn Database>>,
    registered_channel_names: &[String],
    ownership_cache: Arc<crate::ownership::OwnershipCache>,
) -> Option<WasmChannelSetup> {
    let runtime = match WasmChannelRuntime::new(WasmChannelRuntimeConfig::default()) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            tracing::warn!("Failed to initialize WASM channel runtime: {}", e);
            return None;
        }
    };

    let pairing_store = if let Some(db) = database {
        Arc::new(PairingStore::new(Arc::clone(db), ownership_cache))
    } else {
        tracing::warn!("No database available for WASM channels; DM pairing will not persist");
        Arc::new(PairingStore::new_noop())
    };
    let settings_store: Option<Arc<dyn crate::db::SettingsStore>> =
        database.map(|db| Arc::clone(db) as Arc<dyn crate::db::SettingsStore>);
    let mut loader = WasmChannelLoader::new(
        Arc::clone(&runtime),
        Arc::clone(&pairing_store),
        settings_store.clone(),
        config.owner_id.clone(),
    );
    if let Some(secrets) = secrets_store {
        loader = loader.with_secrets_store(Arc::clone(secrets));
    }

    let results = match loader
        .load_from_dir(&config.channels.wasm_channels_dir)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to scan WASM channels directory: {}", e);
            return None;
        }
    };

    let wasm_router = Arc::new(WasmChannelRouter::new());
    let mut channels: Vec<(String, Box<dyn crate::channels::Channel>)> = Vec::new();
    let mut channel_names: Vec<String> = Vec::new();

    // Reserved channel names that WASM modules must not claim.
    // A malicious module could otherwise register as a trusted built-in
    // channel and bypass cross-channel authorization checks.
    //
    // This list includes:
    // - All native/built-in channel names (prevent impersonation)
    // - Trusted approval channels from session::TRUSTED_APPROVAL_CHANNELS
    // - The bootstrap sentinel (universal approval wildcard)
    for loaded in results.loaded {
        let name_lower = loaded.name().to_ascii_lowercase();
        if is_reserved_wasm_channel_name(&name_lower) {
            tracing::warn!(
                channel = %loaded.name(),
                "Rejected WASM channel with reserved name"
            );
            continue;
        }
        // Also reject any name that collides with an already-registered
        // channel to prevent a WASM module from shadowing a channel that
        // was registered earlier in the startup sequence.
        if registered_channel_names
            .iter()
            .any(|n| n.to_ascii_lowercase() == name_lower)
        {
            tracing::warn!(
                channel = %loaded.name(),
                "Rejected WASM channel that collides with already-registered channel"
            );
            continue;
        }

        let (name, channel) = register_channel(
            loaded,
            config,
            secrets_store,
            settings_store.as_ref(),
            &wasm_router,
        )
        .await;
        channel_names.push(name.clone());
        channels.push((name, channel));
    }

    for (path, err) in &results.errors {
        tracing::warn!("Failed to load WASM channel {}: {}", path.display(), err);
    }

    // Always create webhook routes (even with no channels loaded) so that
    // channels hot-added at runtime can receive webhooks without a restart.
    let webhook_routes = {
        Some(create_wasm_channel_router(
            Arc::clone(&wasm_router),
            extension_manager.map(Arc::clone),
        ))
    };

    Some(WasmChannelSetup {
        channels,
        channel_names,
        webhook_routes,
        wasm_channel_runtime: runtime,
        pairing_store,
        wasm_channel_router: wasm_router,
    })
}

/// Process a single loaded WASM channel: retrieve secrets, inject config,
/// register with the router, and set up signing keys and credentials.
async fn register_channel(
    loaded: LoadedChannel,
    config: &Config,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    settings_store: Option<&Arc<dyn crate::db::SettingsStore>>,
    wasm_router: &Arc<WasmChannelRouter>,
) -> (String, Box<dyn crate::channels::Channel>) {
    let channel_name = loaded.name().to_string();
    tracing::debug!("Loaded WASM channel: {}", channel_name);
    let owner_actor_id = owner_actor_id_for_channel(&loaded, config, &channel_name);

    let secret_name = loaded.webhook_secret_name();
    let sig_key_secret_name = loaded.signature_key_secret_name();
    let hmac_secret_name = loaded.hmac_secret_name();

    // Channel-level secrets: owner_id is correct — channels are instance resources.
    let webhook_secret = if let Some(secrets) = secrets_store {
        secrets
            .get_decrypted(&config.owner_id, &secret_name)
            .await
            .ok()
            .map(|s| s.expose().to_string())
    } else {
        None
    };

    let secret_header = loaded.webhook_secret_header().map(|s| s.to_string());
    let host_webhook_secret = if loaded.webhook_secret_managed_by_host() {
        webhook_secret.clone()
    } else {
        None
    };

    let webhook_path = format!("/webhook/{}", channel_name);
    let endpoints = vec![RegisteredEndpoint {
        channel_name: channel_name.clone(),
        path: webhook_path,
        methods: vec!["POST".to_string()],
        require_secret: host_webhook_secret.is_some(),
    }];

    let channel_arc = Arc::new(loaded.channel.with_owner_actor_id(owner_actor_id.clone()));

    // Inject runtime config (tunnel URL, webhook secret, owner_id).
    {
        let mut config_updates = std::collections::HashMap::new();

        if let Some(ref tunnel_url) = config.tunnel.public_url {
            config_updates.insert(
                "tunnel_url".to_string(),
                serde_json::Value::String(tunnel_url.clone()),
            );
        }

        if let Some(ref secret) = webhook_secret {
            config_updates.insert(
                "webhook_secret".to_string(),
                serde_json::Value::String(secret.clone()),
            );
        }

        if let Some(ref resolved_owner_id) = owner_actor_id {
            config_updates.insert(
                "owner_id".to_string(),
                serde_json::Value::String(resolved_owner_id.clone()),
            );
        }

        if channel_name == TELEGRAM_CHANNEL_NAME
            && let Some(store) = settings_store
            && let Ok(Some(serde_json::Value::String(username))) = store
                .get_setting(&config.owner_id, &bot_username_setting_key(&channel_name))
                .await
            && !username.trim().is_empty()
        {
            config_updates.insert("bot_username".to_string(), serde_json::json!(username));
        }
        // Inject channel-specific secrets into config for channels that need
        // credentials in API request bodies (e.g., Feishu token exchange).
        // The credential injection system only replaces placeholders in URLs
        // and headers, so channels like Feishu that exchange app_id + app_secret
        // for a tenant token need the raw values in their config.
        inject_channel_secrets_into_config(
            &channel_name,
            &config.owner_id,
            secrets_store,
            &mut config_updates,
        )
        .await;

        if !config_updates.is_empty() {
            channel_arc.update_config(config_updates).await;
            tracing::info!(
                channel = %channel_name,
                has_tunnel = config.tunnel.public_url.is_some(),
                has_webhook_secret = webhook_secret.is_some(),
                "Injected runtime config into channel"
            );
        }
    }

    tracing::info!(
        channel = %channel_name,
        has_webhook_secret = host_webhook_secret.is_some(),
        secret_header = ?secret_header,
        "Registering channel with router"
    );

    wasm_router
        .register(
            Arc::clone(&channel_arc),
            endpoints,
            host_webhook_secret.clone(),
            secret_header,
        )
        .await;

    // Register Ed25519 signature key if declared in capabilities.
    if let Some(ref sig_key_name) = sig_key_secret_name
        && let Some(secrets) = secrets_store
        && let Ok(key_secret) = secrets.get_decrypted(&config.owner_id, sig_key_name).await
    {
        match wasm_router
            .register_signature_key(&channel_name, key_secret.expose())
            .await
        {
            Ok(()) => {
                tracing::info!(channel = %channel_name, "Registered Ed25519 signature key")
            }
            Err(e) => {
                tracing::error!(channel = %channel_name, error = %e, "Invalid signature key in secrets store")
            }
        }
    }

    // Register HMAC signing secret if declared in capabilities.
    if let Some(ref hmac_secret_name) = hmac_secret_name
        && let Some(secrets) = secrets_store
        && let Ok(secret) = secrets
            .get_decrypted(&config.owner_id, hmac_secret_name)
            .await
    {
        wasm_router
            .register_hmac_secret(&channel_name, secret.expose())
            .await;
        tracing::info!(channel = %channel_name, "Registered HMAC signing secret");
    }

    // Inject credentials from secrets store / environment.
    match inject_channel_credentials(
        &channel_arc,
        secrets_store
            .as_ref()
            .map(|s| s.as_ref() as &dyn SecretsStore),
        &channel_name,
        &config.owner_id,
    )
    .await
    {
        Ok(count) => {
            if count > 0 {
                tracing::info!(
                    channel = %channel_name,
                    credentials_injected = count,
                    "Channel credentials injected"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                channel = %channel_name,
                error = %e,
                "Failed to inject channel credentials"
            );
        }
    }

    (channel_name, Box::new(SharedWasmChannel::new(channel_arc)))
}

fn owner_actor_id_for_channel(
    loaded: &LoadedChannel,
    config: &Config,
    channel_name: &str,
) -> Option<String> {
    config
        .channels
        .wasm_channel_owner_ids
        .get(channel_name)
        .map(ToString::to_string)
        .or_else(|| {
            let value = loaded
                .capabilities_file
                .as_ref()
                .and_then(|file| file.config.get("owner_id"))?;
            match value {
                serde_json::Value::Number(n) => {
                    let id = n.as_i64();
                    if id.is_none() {
                        tracing::debug!(
                            channel = %channel_name,
                            value = %n,
                            "Non-integer numeric owner_id in capabilities config"
                        );
                    }
                    id.map(|id| id.to_string())
                }
                serde_json::Value::String(s) => {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                serde_json::Value::Null => None,
                other => {
                    tracing::debug!(
                        channel = %channel_name,
                        value = ?other,
                        "Ignoring non-scalar owner_id in capabilities config"
                    );
                    None
                }
            }
        })
}

/// Inject credentials for a channel based on naming convention.
///
/// Looks for secrets matching the pattern `{channel_name}_*` and injects them
/// as credential placeholders (e.g., `telegram_bot_token` -> `{TELEGRAM_BOT_TOKEN}`).
///
/// Falls back to environment variables starting with the uppercase channel name
/// prefix (e.g., `TELEGRAM_` for channel `telegram`) for missing credentials.
///
/// Returns the number of credentials injected.
pub async fn inject_channel_credentials(
    channel: &Arc<WasmChannel>,
    secrets: Option<&dyn SecretsStore>,
    channel_name: &str,
    owner_id: &str,
) -> anyhow::Result<usize> {
    if channel_name.trim().is_empty() {
        return Ok(0);
    }

    let mut count = 0;
    let mut injected_placeholders = HashSet::new();

    // 1. Try injecting from persistent secrets store if available
    if let Some(secrets) = secrets {
        let all_secrets = secrets
            .list(owner_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list secrets: {}", e))?;

        let prefix = format!("{}_", channel_name.to_ascii_lowercase());

        for secret_meta in all_secrets {
            if !secret_meta.name.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }

            let decrypted = match secrets.get_decrypted(owner_id, &secret_meta.name).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        secret = %secret_meta.name,
                        error = %e,
                        "Failed to decrypt secret for channel credential injection"
                    );
                    continue;
                }
            };

            let placeholder = secret_meta.name.to_uppercase();

            tracing::debug!(
                channel = %channel_name,
                secret = %secret_meta.name,
                placeholder = %placeholder,
                "Injecting credential"
            );

            channel
                .set_credential(&placeholder, decrypted.expose().to_string())
                .await;
            injected_placeholders.insert(placeholder);
            count += 1;
        }
    }

    // 2. Fall back to environment variables for credentials not in the secrets store.
    // Only env vars starting with the channel's uppercase prefix are allowed
    // (e.g., TELEGRAM_ for channel "telegram") to prevent reading unrelated host
    // credentials like AWS_SECRET_ACCESS_KEY.
    let prefix = format!("{}_", channel_name.to_ascii_uppercase());
    let caps = channel.capabilities();
    if let Some(ref http_cap) = caps.tool_capabilities.http {
        for cred_mapping in http_cap.credentials.values() {
            let placeholder = cred_mapping.secret_name.to_uppercase();
            if injected_placeholders.contains(&placeholder) {
                continue;
            }
            if !placeholder.starts_with(&prefix) {
                tracing::warn!(
                    channel = %channel_name,
                    placeholder = %placeholder,
                    "Ignoring non-prefixed credential placeholder in environment fallback"
                );
                continue;
            }
            if let Ok(env_value) = std::env::var(&placeholder)
                && !env_value.is_empty()
            {
                tracing::debug!(
                    channel = %channel_name,
                    placeholder = %placeholder,
                    "Injecting credential from environment variable"
                );
                channel.set_credential(&placeholder, env_value).await;
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Inject channel-specific secrets into the config JSON.
///
/// Some channels (e.g., Feishu) need raw credential values in their config
/// because they perform token exchanges that require secrets in the HTTP
/// request body. The standard credential injection system only replaces
/// placeholders in URLs and headers, so this function fills config fields
/// that map to secret names.
///
/// Mapping: for a channel named "feishu", secrets `feishu_app_id`,
/// `feishu_app_secret`, and `feishu_verification_token` are injected as config
/// keys `app_id`, `app_secret`, and `verification_token`.
async fn inject_channel_secrets_into_config(
    channel_name: &str,
    owner_id: &str,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    config_updates: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    // Map of (config_key, secret_name) pairs per channel.
    let secret_config_mappings: &[(&str, &str)] = match channel_name {
        "feishu" => &[
            ("app_id", "feishu_app_id"),
            ("app_secret", "feishu_app_secret"),
            ("verification_token", "feishu_verification_token"),
        ],
        _ => return,
    };

    let Some(secrets) = secrets_store else {
        return;
    };

    for &(config_key, secret_name) in secret_config_mappings {
        match secrets.get_decrypted(owner_id, secret_name).await {
            Ok(decrypted) => {
                config_updates.insert(
                    config_key.to_string(),
                    serde_json::Value::String(decrypted.expose().to_string()),
                );
                tracing::debug!(
                    channel = %channel_name,
                    config_key = %config_key,
                    "Injected secret into channel config"
                );
            }
            Err(_) => {
                // Also try environment variable fallback.
                let env_name = secret_name.to_uppercase();
                if let Ok(val) = std::env::var(&env_name)
                    && !val.is_empty()
                {
                    config_updates.insert(config_key.to_string(), serde_json::Value::String(val));
                    tracing::debug!(
                        channel = %channel_name,
                        config_key = %config_key,
                        "Injected secret from env into channel config"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::reserved_wasm_channel_names;
    use crate::agent::session::{BOOTSTRAP_SOURCE_CHANNEL, TRUSTED_APPROVAL_CHANNELS};
    use crate::channels::wasm::capabilities::ChannelCapabilities;
    use crate::channels::wasm::{
        ChannelCapabilitiesFile, LoadedChannel, PreparedChannelModule, WasmChannel,
        WasmChannelRouter, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::config::Config;
    use crate::pairing::PairingStore;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};
    use crate::testing::credentials::TEST_CRYPTO_KEY;
    use crate::tools::wasm::ResourceLimits;
    use secrecy::SecretString;

    /// Build the same reserved-name list that `setup_wasm_channels` uses.
    fn reserved_names() -> Vec<&'static str> {
        reserved_wasm_channel_names()
    }

    fn test_config() -> (Config, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = Config::for_testing(
            temp_dir.path().join("test.db"),
            temp_dir.path().join("skills"),
            temp_dir.path().join("installed-skills"),
        );
        (config, temp_dir)
    }

    fn test_loaded_channel(name: &str, capabilities_config: serde_json::Value) -> LoadedChannel {
        let cap_file = ChannelCapabilitiesFile::from_json(
            &serde_json::json!({
                "type": "channel",
                "name": name,
                "capabilities": {
                    "channel": {
                        "allowed_paths": [format!("/webhook/{name}")]
                    }
                },
                "config": capabilities_config
            })
            .to_string(),
        )
        .unwrap();

        let runtime =
            Arc::new(WasmChannelRuntime::new(WasmChannelRuntimeConfig::for_testing()).unwrap());
        let prepared = Arc::new(PreparedChannelModule {
            name: name.to_string(),
            description: format!("Test channel: {name}"),
            component: None,
            limits: ResourceLimits::default(),
        });
        let channel = WasmChannel::new(
            runtime,
            prepared,
            ChannelCapabilities::for_channel(name),
            "owner-scope",
            cap_file.config_json(),
            Arc::new(PairingStore::new_noop()),
            None,
        );

        LoadedChannel {
            channel,
            capabilities_file: Some(cap_file),
        }
    }

    #[test]
    fn reserved_names_include_trusted_approval_channels() {
        let reserved = reserved_names();
        for &trusted in TRUSTED_APPROVAL_CHANNELS {
            assert!(
                reserved.contains(&trusted),
                "trusted approval channel '{}' must be in WASM reserved names",
                trusted
            );
        }
    }

    #[test]
    fn reserved_names_include_bootstrap_sentinel() {
        let reserved = reserved_names();
        assert!(
            reserved.contains(&BOOTSTRAP_SOURCE_CHANNEL),
            "__bootstrap__ sentinel must be in WASM reserved names"
        );
    }

    #[test]
    fn reserved_names_reject_case_insensitive() {
        // The setup logic lowercases the WASM channel name before checking.
        // Verify that "Web" or "GATEWAY" would be caught.
        let reserved = reserved_names();
        let test_cases = ["Web", "GATEWAY", "CLI", "Repl", "__BOOTSTRAP__"];
        for name in test_cases {
            let lowered = name.to_ascii_lowercase();
            assert!(
                reserved.contains(&lowered.as_str()),
                "'{}' (lowercased to '{}') should match a reserved name",
                name,
                lowered
            );
        }
    }

    #[test]
    fn non_reserved_names_allowed() {
        let reserved = reserved_names();
        let allowed = ["discord", "telegram", "my-custom-channel", "slack-bot"];
        for name in allowed {
            assert!(
                !reserved.contains(&name),
                "'{}' should NOT be reserved",
                name
            );
        }
    }

    #[test]
    fn owner_actor_id_falls_back_to_capabilities_config() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 12345 }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            Some("12345".to_string())
        );
    }

    #[test]
    fn owner_actor_id_prefers_runtime_config_over_capabilities_config() {
        let (mut config, _temp_dir) = test_config();
        config
            .channels
            .wasm_channel_owner_ids
            .insert("telegram".to_string(), 11111);
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 22222 }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            Some("11111".to_string())
        );
    }

    #[test]
    fn owner_actor_id_handles_string_capabilities_config() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": "12345" }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            Some("12345".to_string())
        );
    }

    #[test]
    fn owner_actor_id_returns_none_for_null_capabilities_config() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": null }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            None
        );
    }

    #[test]
    fn owner_actor_id_returns_none_when_no_capabilities_file() {
        let (config, _temp_dir) = test_config();
        let mut loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 12345 }));
        loaded.capabilities_file = None;

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            None
        );
    }

    #[test]
    fn owner_actor_id_returns_none_for_empty_string() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": "" }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            None
        );
    }

    #[test]
    fn owner_actor_id_returns_none_for_non_scalar_value() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": [1, 2, 3] }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            None
        );
    }

    #[test]
    fn owner_actor_id_returns_none_for_float_owner_id() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 1.5 }));

        assert_eq!(
            super::owner_actor_id_for_channel(&loaded, &config, "telegram"),
            None
        );
    }

    #[tokio::test]
    async fn register_channel_routes_capabilities_owner_id_to_wasm_channel() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 12345 }));
        let wasm_router = Arc::new(WasmChannelRouter::new());

        let (name, _channel) =
            super::register_channel(loaded, &config, &None, None, &wasm_router).await;

        assert_eq!(name, "telegram");
        let registered = wasm_router
            .get_channel_for_path("/webhook/telegram")
            .await
            .expect("telegram channel should be registered");
        assert_eq!(
            registered.owner_actor_id_for_test(),
            Some("12345".to_string())
        );
    }

    #[tokio::test]
    async fn register_channel_propagates_capabilities_owner_id_to_config() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": 12345 }));
        let wasm_router = Arc::new(WasmChannelRouter::new());

        let (_name, _channel) =
            super::register_channel(loaded, &config, &None, None, &wasm_router).await;

        let registered = wasm_router
            .get_channel_for_path("/webhook/telegram")
            .await
            .expect("telegram channel should be registered");
        let runtime_config = registered.get_config().await;
        let owner_id = runtime_config
            .get("owner_id")
            .expect("owner_id should be in config");
        assert_eq!(owner_id, &serde_json::json!("12345"));
    }

    #[tokio::test]
    async fn register_channel_does_not_inject_null_owner_id_to_config() {
        let (config, _temp_dir) = test_config();
        let loaded = test_loaded_channel("telegram", serde_json::json!({ "owner_id": null }));
        let wasm_router = Arc::new(WasmChannelRouter::new());

        let (_name, _channel) =
            super::register_channel(loaded, &config, &None, None, &wasm_router).await;

        let registered = wasm_router
            .get_channel_for_path("/webhook/telegram")
            .await
            .expect("telegram channel should be registered");
        assert_eq!(
            registered.owner_actor_id_for_test(),
            None,
            "null owner_id from capabilities should not resolve"
        );
    }

    #[tokio::test]
    async fn inject_channel_secrets_uses_owner_scope() {
        let crypto =
            Arc::new(SecretsCrypto::new(SecretString::from(TEST_CRYPTO_KEY.to_string())).unwrap());
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));
        secrets
            .create(
                "owner-123",
                CreateSecretParams {
                    name: "feishu_app_id".to_string(),
                    value: SecretString::from("owner-app-id".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();
        secrets
            .create(
                "owner-123",
                CreateSecretParams {
                    name: "feishu_app_secret".to_string(),
                    value: SecretString::from("owner-app-secret".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();
        secrets
            .create(
                "default",
                CreateSecretParams {
                    name: "feishu_app_id".to_string(),
                    value: SecretString::from("default-app-id".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let mut config_updates = HashMap::new();
        super::inject_channel_secrets_into_config(
            "feishu",
            "owner-123",
            &Some(Arc::clone(&secrets)),
            &mut config_updates,
        )
        .await;

        assert_eq!(
            config_updates.get("app_id"),
            Some(&serde_json::json!("owner-app-id"))
        );
        assert_eq!(
            config_updates.get("app_secret"),
            Some(&serde_json::json!("owner-app-secret"))
        );
    }
}
