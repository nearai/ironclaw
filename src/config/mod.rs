//! Configuration for IronClaw.
//!
//! Settings are loaded with priority: env var > database > default.
//! `DATABASE_URL` lives in `~/.ironclaw/.env` (loaded via dotenvy early
//! in startup). Everything else comes from env vars, the DB settings
//! table, or auto-detection.

mod agent;
mod builder;
mod channels;
mod database;
pub(crate) mod embeddings;
mod heartbeat;
pub(crate) mod helpers;
mod hygiene;
pub(crate) mod llm;
pub mod relay;
mod routines;
mod safety;
mod sandbox;
mod search;
mod secrets;
mod skills;
mod transcription;
mod tunnel;
mod wasm;
mod workspace;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, Once};

use crate::error::ConfigError;
use crate::settings::Settings;

// Re-export all public types so `crate::config::FooConfig` continues to work.
pub use self::agent::AgentConfig;
pub use self::builder::BuilderModeConfig;
pub use self::channels::{
    ChannelsConfig, CliConfig, DEFAULT_GATEWAY_PORT, GatewayConfig, HttpConfig, SignalConfig,
};
pub use self::database::{DatabaseBackend, DatabaseConfig, SslMode, default_libsql_path};
pub use self::embeddings::{DEFAULT_EMBEDDING_CACHE_SIZE, EmbeddingsConfig};
pub use self::heartbeat::HeartbeatConfig;
pub use self::hygiene::HygieneConfig;
pub use self::llm::default_session_path;
pub use self::relay::RelayConfig;
pub use self::routines::RoutineConfig;
pub use self::safety::SafetyConfig;
use self::safety::resolve_safety_config;
pub use self::sandbox::{ClaudeCodeConfig, SandboxModeConfig};
pub use self::search::WorkspaceSearchConfig;
pub use self::secrets::SecretsConfig;
pub use self::skills::SkillsConfig;
pub use self::transcription::TranscriptionConfig;
pub use self::tunnel::TunnelConfig;
pub use self::wasm::WasmConfig;
pub use self::workspace::WorkspaceConfig;
pub use crate::llm::config::{
    BedrockConfig, CacheRetention, LlmConfig, NearAiConfig, OAUTH_PLACEHOLDER, OpenAiCodexConfig,
    RegistryProviderConfig,
};
pub use crate::llm::session::SessionConfig;

// Thread-safe env var override helpers (replaces unsafe `std::env::set_var`
// for mid-process env mutations in multi-threaded contexts).
pub use self::helpers::{env_or_override, set_runtime_env};

/// Thread-safe overlay for injected env vars (secrets loaded from DB).
///
/// Used by `inject_llm_keys_from_secrets()` to make API keys available to
/// `optional_env()` without unsafe `set_var` calls. `optional_env()` checks
/// real env vars first, then falls back to this overlay.
///
/// Uses `Mutex<HashMap>` instead of `OnceLock` so that both
/// `inject_os_credentials()` and `inject_llm_keys_from_secrets()` can merge
/// their data. Whichever runs first initialises the map; the second merges in.
static INJECTED_VARS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static WARNED_EXPLICIT_DEFAULT_OWNER_ID: Once = Once::new();

/// Main configuration for the agent.
#[derive(Debug, Clone)]
pub struct Config {
    pub owner_id: String,
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub embeddings: EmbeddingsConfig,
    pub tunnel: TunnelConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
    pub hygiene: HygieneConfig,
    pub routines: RoutineConfig,
    pub sandbox: SandboxModeConfig,
    pub claude_code: ClaudeCodeConfig,
    pub skills: SkillsConfig,
    pub transcription: TranscriptionConfig,
    pub search: WorkspaceSearchConfig,
    pub workspace: WorkspaceConfig,
    pub observability: crate::observability::ObservabilityConfig,
    /// Channel-relay integration (Slack via external relay service).
    /// Present only when both `CHANNEL_RELAY_URL` and `CHANNEL_RELAY_API_KEY` are set.
    pub relay: Option<RelayConfig>,
}

impl Config {
    /// Create a full Config for integration tests without reading env vars.
    ///
    /// Requires the `libsql` feature. Sets up:
    /// - libSQL database at the given path
    /// - WASM and embeddings disabled
    /// - Skills enabled with the given directories
    /// - Heartbeat, routines, sandbox, builder all disabled
    /// - Safety with injection check off, 100k output limit
    #[cfg(feature = "libsql")]
    pub fn for_testing(
        libsql_path: std::path::PathBuf,
        skills_dir: std::path::PathBuf,
        installed_skills_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            owner_id: "default".to_string(),
            database: DatabaseConfig {
                backend: DatabaseBackend::LibSql,
                url: secrecy::SecretString::from("unused://test".to_string()),
                pool_size: 1,
                ssl_mode: SslMode::Disable,
                libsql_path: Some(libsql_path),
                libsql_url: None,
                libsql_auth_token: None,
            },
            llm: LlmConfig::for_testing(),
            embeddings: EmbeddingsConfig::default(),
            tunnel: TunnelConfig::default(),
            channels: ChannelsConfig {
                cli: CliConfig { enabled: false },
                http: None,
                gateway: None,
                signal: None,
                wasm_channels_dir: std::env::temp_dir().join("ironclaw-test-channels"),
                wasm_channels_enabled: false,
                wasm_channel_owner_ids: HashMap::new(),
            },
            agent: AgentConfig::for_testing(),
            safety: SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            },
            wasm: WasmConfig {
                enabled: false,
                ..WasmConfig::default()
            },
            secrets: SecretsConfig::default(),
            builder: BuilderModeConfig {
                enabled: false,
                ..BuilderModeConfig::default()
            },
            heartbeat: HeartbeatConfig::default(),
            hygiene: HygieneConfig::default(),
            routines: RoutineConfig {
                enabled: false,
                ..RoutineConfig::default()
            },
            sandbox: SandboxModeConfig {
                enabled: false,
                ..SandboxModeConfig::default()
            },
            claude_code: ClaudeCodeConfig::default(),
            skills: SkillsConfig {
                enabled: true,
                local_dir: skills_dir,
                installed_dir: installed_skills_dir,
                ..SkillsConfig::default()
            },
            transcription: TranscriptionConfig::default(),
            search: WorkspaceSearchConfig::default(),
            workspace: WorkspaceConfig {
                memory_layers: vec![],
            },
            observability: crate::observability::ObservabilityConfig::default(),
            relay: None,
        }
    }

    /// Load configuration from environment variables and the database.
    ///
    /// Priority: env var > TOML config file > DB settings > default.
    /// This is the primary way to load config after DB is connected.
    pub async fn from_db(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
    ) -> Result<Self, ConfigError> {
        Self::from_db_with_toml(store, user_id, None).await
    }

    /// Load from DB with an optional TOML config file overlay.
    pub async fn from_db_with_toml(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();

        // Load all settings from DB into a Settings struct
        let mut db_settings = match store.get_all_settings(user_id).await {
            Ok(map) => Settings::from_db_map(&map),
            Err(e) => {
                tracing::warn!("Failed to load settings from DB, using defaults: {}", e);
                Settings::default()
            }
        };

        // Overlay TOML config file (values win over DB settings)
        Self::apply_toml_overlay(&mut db_settings, toml_path)?;

        Self::build(&db_settings).await
    }

    /// Load configuration from environment variables only (no database).
    ///
    /// Used during early startup before the database is connected,
    /// and by CLI commands that don't have DB access.
    /// Falls back to legacy `settings.json` on disk if present.
    ///
    /// Loads both `./.env` (standard, higher priority) and `~/.ironclaw/.env`
    /// (lower priority) via dotenvy, which never overwrites existing vars.
    pub async fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with_toml(None).await
    }

    /// Load from env with an optional TOML config file overlay.
    pub async fn from_env_with_toml(
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, ConfigError> {
        let settings = load_bootstrap_settings(toml_path)?;
        Self::build(&settings).await
    }

    /// Load and merge a TOML config file into settings.
    ///
    /// If `explicit_path` is `Some`, loads from that path (errors are fatal).
    /// If `None`, tries the default path `~/.ironclaw/config.toml` (missing
    /// file is silently ignored).
    fn apply_toml_overlay(
        settings: &mut Settings,
        explicit_path: Option<&std::path::Path>,
    ) -> Result<(), ConfigError> {
        let path = explicit_path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(Settings::default_toml_path);

        match Settings::load_toml(&path) {
            Ok(Some(toml_settings)) => {
                settings.merge_from(&toml_settings);
                tracing::debug!("Loaded TOML config from {}", path.display());
            }
            Ok(None) => {
                if explicit_path.is_some() {
                    return Err(ConfigError::ParseError(format!(
                        "Config file not found: {}",
                        path.display()
                    )));
                }
            }
            Err(e) => {
                if explicit_path.is_some() {
                    return Err(ConfigError::ParseError(format!(
                        "Failed to load config file {}: {}",
                        path.display(),
                        e
                    )));
                }
                tracing::warn!("Failed to load default config file: {}", e);
            }
        }
        Ok(())
    }

    /// Re-resolve only the LLM config after credential injection.
    ///
    /// Called by `AppBuilder::init_secrets()` after injecting API keys into
    /// the env overlay. Only rebuilds `self.llm` — all other config fields
    /// are unaffected, preserving values from the initial config load (or
    /// from `Config::for_testing()` in test mode).
    pub async fn re_resolve_llm(
        &mut self,
        store: Option<&(dyn crate::db::SettingsStore + Sync)>,
        user_id: &str,
        toml_path: Option<&std::path::Path>,
    ) -> Result<(), ConfigError> {
        let settings = if let Some(store) = store {
            let mut s = match store.get_all_settings(user_id).await {
                Ok(map) => Settings::from_db_map(&map),
                Err(_) => Settings::default(),
            };
            Self::apply_toml_overlay(&mut s, toml_path)?;
            s
        } else {
            Settings::default()
        };
        self.llm = LlmConfig::resolve(&settings)?;
        Ok(())
    }

    /// Build config from settings (shared by from_env and from_db).
    async fn build(settings: &Settings) -> Result<Self, ConfigError> {
        let owner_id = resolve_owner_id(settings)?;

        let tunnel = TunnelConfig::resolve(settings)?;
        let channels = ChannelsConfig::resolve(settings, &owner_id)?;
        let workspace_user_id = channels
            .gateway
            .as_ref()
            .map(|gw| gw.user_id.clone())
            .unwrap_or_else(|| "default".to_string());

        Ok(Self {
            owner_id: owner_id.clone(),
            database: DatabaseConfig::resolve()?,
            llm: LlmConfig::resolve(settings)?,
            embeddings: EmbeddingsConfig::resolve(settings)?,
            tunnel,
            channels,
            agent: AgentConfig::resolve(settings)?,
            safety: resolve_safety_config(settings)?,
            wasm: WasmConfig::resolve(settings)?,
            secrets: SecretsConfig::resolve().await?,
            builder: BuilderModeConfig::resolve(settings)?,
            heartbeat: HeartbeatConfig::resolve(settings)?,
            hygiene: HygieneConfig::resolve()?,
            routines: RoutineConfig::resolve()?,
            sandbox: SandboxModeConfig::resolve(settings)?,
            claude_code: ClaudeCodeConfig::resolve(settings)?,
            skills: SkillsConfig::resolve()?,
            transcription: TranscriptionConfig::resolve(settings)?,
            search: WorkspaceSearchConfig::resolve()?,
            workspace: WorkspaceConfig::resolve(&workspace_user_id)?,
            observability: crate::observability::ObservabilityConfig {
                backend: std::env::var("OBSERVABILITY_BACKEND").unwrap_or_else(|_| "none".into()),
            },
            relay: RelayConfig::from_env(),
        })
    }
}

pub(crate) fn load_bootstrap_settings(
    toml_path: Option<&std::path::Path>,
) -> Result<Settings, ConfigError> {
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_ironclaw_env();

    let mut settings = Settings::load();
    Config::apply_toml_overlay(&mut settings, toml_path)?;
    Ok(settings)
}

pub(crate) fn resolve_owner_id(settings: &Settings) -> Result<String, ConfigError> {
    let env_owner_id = self::helpers::optional_env("IRONCLAW_OWNER_ID")?;
    let settings_owner_id = settings.owner_id.clone();
    let configured_owner_id = env_owner_id.clone().or(settings_owner_id.clone());

    let owner_id = configured_owner_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());

    if owner_id == "default"
        && (env_owner_id.is_some()
            || settings_owner_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()))
    {
        WARNED_EXPLICIT_DEFAULT_OWNER_ID.call_once(|| {
            tracing::warn!(
                "IRONCLAW_OWNER_ID resolved to the legacy 'default' scope explicitly; durable state will keep legacy owner behavior"
            );
        });
    }

    Ok(owner_id)
}

/// Load API keys from the encrypted secrets store into a thread-safe overlay.
///
/// This bridges the gap between secrets stored during onboarding and the
/// env-var-first resolution in `LlmConfig::resolve()`. Keys in the overlay
/// are read by `optional_env()` before falling back to `std::env::var()`,
/// so explicit env vars always win.
///
/// Also loads tokens from OS credential stores (macOS Keychain / Linux
/// credentials files) which don't require the secrets DB.
pub async fn inject_llm_keys_from_secrets(
    secrets: &dyn crate::secrets::SecretsStore,
    user_id: &str,
) {
    // Static mappings for well-known providers.
    // The registry's setup hints define secret_name -> env_var mappings,
    // so new providers added to providers.json get injection automatically.
    let mut mappings: Vec<(&str, &str)> = vec![
        ("llm_nearai_api_key", "NEARAI_API_KEY"),
        ("llm_anthropic_oauth_token", "ANTHROPIC_OAUTH_TOKEN"),
    ];

    // Dynamically discover secret->env mappings from the provider registry.
    // Uses selectable() which deduplicates user overrides correctly.
    let registry = crate::llm::ProviderRegistry::load();
    let dynamic_mappings: Vec<(String, String)> = registry
        .selectable()
        .iter()
        .filter_map(|def| {
            def.api_key_env.as_ref().and_then(|env_var| {
                def.setup
                    .as_ref()
                    .and_then(|s| s.secret_name())
                    .map(|secret_name| (secret_name.to_string(), env_var.clone()))
            })
        })
        .collect();
    for (secret, env_var) in &dynamic_mappings {
        mappings.push((secret, env_var));
    }

    let mut injected = HashMap::new();

    for (secret_name, env_var) in mappings {
        match std::env::var(env_var) {
            Ok(val) if !val.is_empty() => continue,
            _ => {}
        }
        match secrets.get_decrypted(user_id, secret_name).await {
            Ok(decrypted) => {
                injected.insert(env_var.to_string(), decrypted.expose().to_string());
                tracing::debug!("Loaded secret '{}' for env var '{}'", secret_name, env_var);
            }
            Err(_) => {
                // Secret doesn't exist, that's fine
            }
        }
    }

    inject_os_credential_store_tokens(&mut injected);

    merge_injected_vars(injected);
}

/// Load tokens from OS credential stores (no DB required).
///
/// Called unconditionally during startup — even when the encrypted secrets DB
/// is unavailable (no master key, no DB connection). This ensures OAuth tokens
/// from `claude login` (macOS Keychain / Linux credentials.json)
/// are available for config resolution.
pub fn inject_os_credentials() {
    let mut injected = HashMap::new();
    inject_os_credential_store_tokens(&mut injected);
    merge_injected_vars(injected);
}

/// Merge new entries into the global injected-vars overlay.
///
/// New keys are inserted; existing keys are overwritten (later callers win,
/// e.g. fresh OS credential store tokens override stale DB copies).
fn merge_injected_vars(new_entries: HashMap<String, String>) {
    if new_entries.is_empty() {
        return;
    }
    match INJECTED_VARS.lock() {
        Ok(mut map) => map.extend(new_entries),
        Err(poisoned) => poisoned.into_inner().extend(new_entries),
    }
}

/// Inject a single key-value pair into the overlay.
///
/// Used by the setup wizard to make credentials available to `optional_env()`
/// without calling `unsafe { std::env::set_var }`.
pub fn inject_single_var(key: &str, value: &str) {
    match INJECTED_VARS.lock() {
        Ok(mut map) => {
            map.insert(key.to_string(), value.to_string());
        }
        Err(poisoned) => {
            poisoned
                .into_inner()
                .insert(key.to_string(), value.to_string());
        }
    }
}

/// Shared helper: extract tokens from OS credential stores into the overlay map.
fn inject_os_credential_store_tokens(injected: &mut HashMap<String, String>) {
    // Try the OS credential store for a fresh Anthropic OAuth token.
    // Tokens from `claude login` expire in 8-12h, so the DB copy may be stale.
    // A fresh extraction from macOS Keychain / Linux credentials.json wins
    // over the (possibly expired) copy stored in the encrypted secrets DB.
    if let Some(fresh) = crate::config::ClaudeCodeConfig::extract_oauth_token() {
        injected.insert("ANTHROPIC_OAUTH_TOKEN".to_string(), fresh);
        tracing::debug!("Refreshed ANTHROPIC_OAUTH_TOKEN from OS credential store");
    }
}
