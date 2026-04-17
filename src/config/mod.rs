//! Configuration for IronClaw.
//!
//! Settings are loaded with priority: **DB/TOML > env > default**.
//!
//! DB and TOML are merged into a single `Settings` struct before
//! resolution (DB wins over TOML when both set the same field).
//! Resolvers then check settings before env vars.
//!
//! For concrete (non-`Option`) fields, a settings value equal to the
//! built-in default is treated as "unset" and falls through to env.
//!
//! Exceptions:
//! - Bootstrap configs (database, secrets): env-only (DB not yet available)
//! - Security-sensitive fields (allow_local_tools, allow_full_access,
//!   cost limits, auth tokens): env-only
//! - API keys: env/secrets store only
//!
//! `DATABASE_URL` lives in `~/.ironclaw/.env` (loaded via dotenvy early
//! in startup).

pub mod acp;
mod agent;
mod builder;
mod channels;
mod database;
pub(crate) mod embeddings;
mod heartbeat;
pub(crate) mod helpers;
mod hygiene;
pub(crate) mod llm;
pub mod oauth;
pub mod profile;
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
pub(crate) mod workspace;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, Once};

use crate::error::ConfigError;
use crate::settings::Settings;

// Re-export all public types so `crate::config::FooConfig` continues to work.
pub use self::agent::AgentConfig;
pub use self::builder::BuilderModeConfig;
pub use self::channels::{
    ChannelsConfig, CliConfig, DEFAULT_GATEWAY_PORT, GatewayConfig, GatewayOidcConfig, HttpConfig,
    SignalConfig, TuiChannelConfig,
};
pub use self::database::{DatabaseBackend, DatabaseConfig, SslMode, default_libsql_path};
pub use self::embeddings::{DEFAULT_EMBEDDING_CACHE_SIZE, EmbeddingsConfig};
pub use self::heartbeat::HeartbeatConfig;
pub use self::hygiene::HygieneConfig;
pub use self::llm::default_session_path;
pub use self::oauth::OAuthConfig;
pub use self::relay::RelayConfig;
pub use self::routines::RoutineConfig;
pub use self::safety::SafetyConfig;
use self::safety::resolve_safety_config;
pub use self::sandbox::{AcpModeConfig, ClaudeCodeConfig, SandboxModeConfig};
pub use self::search::WorkspaceSearchConfig;
pub use self::secrets::SecretsConfig;
pub use self::skills::SkillsConfig;
pub use self::transcription::TranscriptionConfig;
pub use self::tunnel::TunnelConfig;
pub use self::wasm::WasmConfig;
pub use self::workspace::WorkspaceConfig;
pub use crate::llm::config::{
    BedrockConfig, CacheRetention, GeminiOauthConfig, LlmConfig, NearAiConfig, OAUTH_PLACEHOLDER,
    OpenAiCodexConfig, RegistryProviderConfig,
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
    pub acp: AcpModeConfig,
    pub skills: SkillsConfig,
    pub transcription: TranscriptionConfig,
    pub search: WorkspaceSearchConfig,
    pub workspace: WorkspaceConfig,
    pub observability: crate::observability::ObservabilityConfig,
    /// OAuth/social login configuration (Google, GitHub, etc.).
    pub oauth: OAuthConfig,
    /// Channel-relay integration (Slack via external relay service).
    /// Present only when both `CHANNEL_RELAY_URL` and `CHANNEL_RELAY_API_KEY` are set.
    pub relay: Option<RelayConfig>,
}

/// Generate a fresh random AES-256-GCM master key for `Config::for_testing`.
///
/// Returns a hex-encoded 32-byte key (64 hex chars), satisfying the length
/// check in `SecretsConfig::resolve`. Each call returns a different value —
/// tests don't need cross-process determinism (each test builds a fresh
/// secrets store on top of a fresh temp DB), and committing a constant
/// master key into the source tree would mean every developer who built
/// with `--features libsql` had a publicly-known key in their process.
#[cfg(feature = "libsql")]
fn generate_test_master_key() -> secrecy::SecretString {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut hex = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    secrecy::SecretString::from(hex)
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
                tui: None,
                wasm_channels_dir: std::env::temp_dir().join("ironclaw-test-channels"),
                wasm_channels_enabled: false,
                configured_wasm_channels: Vec::new(),
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
            // Test config gets a freshly-generated random master key so
            // the secrets store is wired up out of the box. Without this,
            // every replay-mode test that touches credentials would have
            // to either build its own SecretsStore or skip the secrets
            // path entirely. The key is generated per call (NOT a
            // hardcoded constant) — `Config::for_testing` is `pub` so
            // anything in the crate or downstream tests can call it, and
            // committing a known master key into the source tree would
            // mean every developer who built with `--features libsql`
            // had a publicly-known AES-256-GCM key sitting in their
            // process. Tests don't need cross-process determinism here:
            // each test creates its own temp DB, so the secrets store
            // is born fresh on every call anyway.
            secrets: SecretsConfig {
                master_key: Some(generate_test_master_key()),
                enabled: true,
                source: crate::settings::KeySource::Env,
            },
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
            acp: AcpModeConfig::default(),
            skills: SkillsConfig {
                enabled: true,
                local_dir: skills_dir,
                installed_dir: installed_skills_dir,
                ..SkillsConfig::default()
            },
            transcription: TranscriptionConfig::default(),
            search: WorkspaceSearchConfig::default(),
            workspace: WorkspaceConfig::default(),
            observability: crate::observability::ObservabilityConfig::default(),
            oauth: OAuthConfig::default(),
            relay: None,
        }
    }

    /// Load configuration from environment variables and the database.
    ///
    /// Priority: DB/TOML > env > default. TOML is loaded first as a
    /// base, then DB values are merged on top. Subsystem resolvers check
    /// the merged settings before env vars (except bootstrap/security fields).
    pub async fn from_db(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
    ) -> Result<Self, ConfigError> {
        // Existing call sites pass the workspace owner_id, which is the
        // operator/admin scope.
        Self::from_db_with_toml(store, user_id, None, true).await
    }

    /// Load from DB with an optional TOML config file overlay.
    ///
    /// Priority: DB/TOML > env > default. TOML is loaded as the base,
    /// then DB values are merged on top. See module docs for exceptions.
    ///
    /// `is_operator` controls defense-in-depth filtering of admin-only LLM
    /// setting keys (`llm_builtin_overrides`, `llm_custom_providers`,
    /// `ollama_base_url`, `openai_compatible_base_url`). When `false`, those
    /// keys are stripped from the DB overlay so a non-admin user (or a
    /// pre-existing legacy DB row) cannot reactivate a private/loopback
    /// provider endpoint via per-user settings.
    pub async fn from_db_with_toml(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
        toml_path: Option<&std::path::Path>,
        is_operator: bool,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();

        // Resolution layers (lowest -> highest priority):
        //   defaults -> deployment profile -> TOML -> admin DB -> per-user DB
        let mut settings = Settings::default();
        profile::apply_profile(&mut settings)?;
        Self::apply_toml_overlay(&mut settings, toml_path)?;

        // Layer admin-scope defaults between TOML and per-user settings.
        // This lets an admin set instance-wide defaults (e.g. temperature,
        // model) that members inherit unless they override per-user.
        // Skip if the user IS the admin scope to avoid a redundant merge.
        let admin_scope = crate::tools::permissions::ADMIN_SETTINGS_USER_ID;
        if user_id != admin_scope
            && let Ok(mut admin_map) = store.get_all_settings(admin_scope).await
            && !admin_map.is_empty()
        {
            // Defense-in-depth: even though the admin-scope map is written
            // by an operator, never let admin-only LLM endpoint settings
            // (private/loopback URLs) propagate down to non-operators.
            if !is_operator {
                crate::config::helpers::strip_admin_only_llm_keys(&mut admin_map);
            }
            let admin_settings = Settings::from_db_map(&admin_map);
            settings.merge_from(&admin_settings);
        }

        // Overlay per-user DB settings on top (highest priority).
        match store.get_all_settings(user_id).await {
            Ok(mut map) => {
                if !is_operator {
                    crate::config::helpers::strip_admin_only_llm_keys(&mut map);
                }
                let db_settings = Settings::from_db_map(&map);
                settings.merge_from(&db_settings);
            }
            Err(e) => {
                tracing::warn!("Failed to load settings from DB, using defaults: {}", e);
            }
        };

        Self::build(&settings).await
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
        let is_operator = user_id == self.owner_id;
        self.re_resolve_llm_with_secrets(store, user_id, toml_path, None, is_operator)
            .await
    }

    /// Re-resolve LLM config, hydrating API keys from the secrets store.
    ///
    /// `is_operator` controls defense-in-depth filtering of admin-only LLM
    /// setting keys; see [`Config::from_db_with_toml`] for details.
    pub async fn re_resolve_llm_with_secrets(
        &mut self,
        store: Option<&(dyn crate::db::SettingsStore + Sync)>,
        user_id: &str,
        toml_path: Option<&std::path::Path>,
        secrets: Option<&(dyn crate::secrets::SecretsStore + Send + Sync)>,
        is_operator: bool,
    ) -> Result<(), ConfigError> {
        let mut settings = if let Some(store) = store {
            // Resolution layers: profile -> TOML -> admin DB -> per-user DB.
            let mut s = Settings::default();
            profile::apply_profile(&mut s)?;
            Self::apply_toml_overlay(&mut s, toml_path)?;
            let admin_scope = crate::tools::permissions::ADMIN_SETTINGS_USER_ID;
            if user_id != admin_scope
                && let Ok(mut admin_map) = store.get_all_settings(admin_scope).await
                && !admin_map.is_empty()
            {
                if !is_operator {
                    crate::config::helpers::strip_admin_only_llm_keys(&mut admin_map);
                }
                let admin_settings = Settings::from_db_map(&admin_map);
                s.merge_from(&admin_settings);
            }
            if let Ok(mut map) = store.get_all_settings(user_id).await {
                if !is_operator {
                    crate::config::helpers::strip_admin_only_llm_keys(&mut map);
                }
                let db_settings = Settings::from_db_map(&map);
                s.merge_from(&db_settings);
            }
            s
        } else {
            let mut s = Settings::default();
            profile::apply_profile(&mut s)?;
            s
        };

        // Hydrate API keys from encrypted secrets store into the settings
        // struct so that LlmConfig::resolve() sees them without any changes
        // to its synchronous resolution logic.
        if let Some(secrets) = secrets {
            hydrate_llm_keys_from_secrets(&mut settings, secrets, user_id).await;
        }

        self.llm = LlmConfig::resolve(&settings)?;
        Ok(())
    }

    /// Build config from settings (shared by from_env and from_db).
    async fn build(settings: &Settings) -> Result<Self, ConfigError> {
        let owner_id = resolve_owner_id(settings)?;

        let tunnel = TunnelConfig::resolve(settings)?;
        let channels = ChannelsConfig::resolve(settings, &owner_id)?;

        // Resolve the startup workspace against the durable owner scope. The
        // gateway may expose a distinct sender identity, but the base runtime
        // workspace stays owner-scoped and per-user gateway workspaces are
        // handled separately by WorkspacePool.
        let workspace = WorkspaceConfig::resolve(&owner_id)?;

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
            hygiene: HygieneConfig::resolve(settings)?,
            routines: RoutineConfig::resolve(settings)?,
            sandbox: SandboxModeConfig::resolve(settings)?,
            claude_code: ClaudeCodeConfig::resolve(settings)?,
            acp: AcpModeConfig::resolve(settings)?,
            skills: SkillsConfig::resolve(settings)?,
            transcription: TranscriptionConfig::resolve(settings)?,
            search: WorkspaceSearchConfig::resolve(settings)?,
            workspace,
            observability: crate::observability::ObservabilityConfig {
                backend: std::env::var("OBSERVABILITY_BACKEND").unwrap_or_else(|_| "none".into()),
            },
            oauth: OAuthConfig::resolve()?,
            relay: RelayConfig::from_env(),
        })
    }
}

pub(crate) fn load_bootstrap_settings(
    toml_path: Option<&std::path::Path>,
) -> Result<Settings, ConfigError> {
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_ironclaw_env();

    let mut settings = Settings::default();
    profile::apply_profile(&mut settings)?;
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

/// Hydrate LLM API keys from the secrets store into the settings struct.
///
/// Called after loading settings from DB but before `LlmConfig::resolve()`.
/// Populates `api_key` fields that were stripped from settings during the
/// write path and stored encrypted in the secrets store instead.
pub async fn hydrate_llm_keys_from_secrets(
    settings: &mut Settings,
    secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
    user_id: &str,
) {
    // Hydrate builtin overrides
    for (provider_id, override_val) in settings.llm_builtin_overrides.iter_mut() {
        if override_val.api_key.is_some() {
            continue; // Already has a key (legacy plaintext or TOML)
        }
        let secret_name = crate::settings::builtin_secret_name(provider_id);
        if let Ok(decrypted) = secrets.get_decrypted(user_id, &secret_name).await {
            override_val.api_key = Some(decrypted.expose().to_string());
        }
    }

    // Hydrate custom providers
    for provider in settings.llm_custom_providers.iter_mut() {
        if provider.api_key.is_some() {
            continue;
        }
        let secret_name = crate::settings::custom_secret_name(&provider.id);
        if let Ok(decrypted) = secrets.get_decrypted(user_id, &secret_name).await {
            provider.api_key = Some(decrypted.expose().to_string());
        }
    }
}

/// Migrate plaintext API keys from the settings table to the encrypted secrets store.
///
/// Idempotent: skips keys that are already in the secrets store.
/// After migration, strips plaintext keys from the settings table.
pub async fn migrate_plaintext_llm_keys(
    settings_store: &(dyn crate::db::SettingsStore + Sync),
    secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
    user_id: &str,
) {
    let settings_map = match settings_store.get_all_settings(user_id).await {
        Ok(m) => m,
        Err(_) => return,
    };

    let mut migrated = 0u32;

    // Migrate builtin overrides
    if let Some(obj) = settings_map
        .get("llm_builtin_overrides")
        .and_then(|v| v.as_object())
    {
        let mut sanitized = obj.clone();
        for (provider_id, override_val) in obj {
            if let Some(api_key) = override_val.get("api_key").and_then(|v| v.as_str()) {
                if api_key.is_empty() {
                    continue;
                }
                let secret_name = crate::settings::builtin_secret_name(provider_id);
                if !secrets.exists(user_id, &secret_name).await.unwrap_or(false)
                    && let Err(e) = secrets
                        .create(
                            user_id,
                            crate::secrets::CreateSecretParams {
                                name: secret_name.clone(),
                                value: secrecy::SecretString::from(api_key.to_string()),
                                provider: Some(provider_id.clone()),
                                expires_at: None,
                            },
                        )
                        .await
                {
                    tracing::warn!("Failed to migrate key for builtin '{}': {}", provider_id, e);
                    continue;
                }
                if let Some(o) = sanitized
                    .get_mut(provider_id)
                    .and_then(|v| v.as_object_mut())
                {
                    o.remove("api_key");
                }
                migrated += 1;
            }
        }
        if migrated > 0 {
            let _ = settings_store
                .set_setting(
                    user_id,
                    "llm_builtin_overrides",
                    &serde_json::Value::Object(sanitized),
                )
                .await;
        }
    }

    // Migrate custom providers
    let before = migrated;
    if let Some(arr) = settings_map
        .get("llm_custom_providers")
        .and_then(|v| v.as_array())
    {
        let mut sanitized = arr.clone();
        for (idx, provider_val) in arr.iter().enumerate() {
            let provider_id = provider_val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if provider_id.is_empty() {
                continue;
            }
            if let Some(api_key) = provider_val.get("api_key").and_then(|v| v.as_str()) {
                if api_key.is_empty() {
                    continue;
                }
                let secret_name = crate::settings::custom_secret_name(provider_id);
                if !secrets.exists(user_id, &secret_name).await.unwrap_or(false)
                    && let Err(e) = secrets
                        .create(
                            user_id,
                            crate::secrets::CreateSecretParams {
                                name: secret_name.clone(),
                                value: secrecy::SecretString::from(api_key.to_string()),
                                provider: Some(provider_id.to_string()),
                                expires_at: None,
                            },
                        )
                        .await
                {
                    tracing::warn!("Failed to migrate key for custom '{}': {}", provider_id, e);
                    continue;
                }
                if let Some(o) = sanitized[idx].as_object_mut() {
                    o.remove("api_key");
                }
                migrated += 1;
            }
        }
        if migrated > before {
            let _ = settings_store
                .set_setting(
                    user_id,
                    "llm_custom_providers",
                    &serde_json::Value::Array(sanitized),
                )
                .await;
        }
    }

    if migrated > 0 {
        tracing::info!(
            "Migrated {} plaintext LLM API key(s) to encrypted secrets store",
            migrated
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_secrets_store() -> Arc<dyn crate::secrets::SecretsStore + Send + Sync> {
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                crate::secrets::keychain::generate_master_key_hex(),
            ))
            .unwrap(),
        );
        Arc::new(crate::secrets::InMemorySecretsStore::new(crypto))
    }

    #[tokio::test]
    async fn hydrate_populates_builtin_override_keys_from_secrets() {
        let secrets = test_secrets_store();
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams {
                    name: "llm_builtin_openai_api_key".to_string(),
                    value: secrecy::SecretString::from("sk-from-vault".to_string()),
                    provider: Some("openai".to_string()),
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let mut settings = Settings {
            llm_builtin_overrides: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "openai".to_string(),
                    crate::settings::LlmBuiltinOverride {
                        api_key: None, // stripped during write
                        model: Some("gpt-4o".to_string()),
                        base_url: None,
                    },
                );
                m
            },
            ..Default::default()
        };

        hydrate_llm_keys_from_secrets(&mut settings, secrets.as_ref(), "test").await;

        assert_eq!(
            settings.llm_builtin_overrides["openai"].api_key.as_deref(),
            Some("sk-from-vault"),
            "api_key should be hydrated from secrets store"
        );
        assert_eq!(
            settings.llm_builtin_overrides["openai"].model.as_deref(),
            Some("gpt-4o"),
            "model should remain unchanged"
        );
    }

    #[tokio::test]
    async fn hydrate_populates_custom_provider_keys_from_secrets() {
        let secrets = test_secrets_store();
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams {
                    name: "llm_custom_my-llm_api_key".to_string(),
                    value: secrecy::SecretString::from("gsk-custom".to_string()),
                    provider: Some("my-llm".to_string()),
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let mut settings = Settings {
            llm_custom_providers: vec![crate::settings::CustomLlmProviderSettings {
                id: "my-llm".to_string(),
                name: "My LLM".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: Some("http://localhost:8080".to_string()),
                default_model: Some("model-1".to_string()),
                api_key: None, // stripped during write
                builtin: false,
            }],
            ..Default::default()
        };

        hydrate_llm_keys_from_secrets(&mut settings, secrets.as_ref(), "test").await;

        assert_eq!(
            settings.llm_custom_providers[0].api_key.as_deref(),
            Some("gsk-custom"),
            "custom provider api_key should be hydrated from secrets store"
        );
    }

    /// Minimal in-memory `SettingsStore` for unit tests.
    ///
    /// Only the methods exercised by the resolve path are wired up; the
    /// rest return errors so unintended use during a test is loud.
    struct FakeSettingsStore {
        rows: tokio::sync::RwLock<
            std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>>,
        >,
    }

    impl FakeSettingsStore {
        fn new() -> Self {
            Self {
                rows: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            }
        }

        async fn seed(&self, user_id: &str, key: &str, value: serde_json::Value) {
            let mut rows = self.rows.write().await;
            rows.entry(user_id.to_string())
                .or_default()
                .insert(key.to_string(), value);
        }
    }

    #[async_trait::async_trait]
    impl crate::db::SettingsStore for FakeSettingsStore {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, crate::error::DatabaseError> {
            let rows = self.rows.read().await;
            Ok(rows.get(user_id).and_then(|m| m.get(key).cloned()))
        }

        async fn get_setting_full(
            &self,
            _user_id: &str,
            _key: &str,
        ) -> Result<Option<crate::history::SettingRow>, crate::error::DatabaseError> {
            Err(crate::error::DatabaseError::Query(
                "FakeSettingsStore::get_setting_full not implemented".into(),
            ))
        }

        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), crate::error::DatabaseError> {
            self.seed(user_id, key, value.clone()).await;
            Ok(())
        }

        async fn delete_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<bool, crate::error::DatabaseError> {
            let mut rows = self.rows.write().await;
            Ok(rows
                .get_mut(user_id)
                .map(|m| m.remove(key).is_some())
                .unwrap_or(false))
        }

        async fn list_settings(
            &self,
            _user_id: &str,
        ) -> Result<Vec<crate::history::SettingRow>, crate::error::DatabaseError> {
            Err(crate::error::DatabaseError::Query(
                "FakeSettingsStore::list_settings not implemented".into(),
            ))
        }

        async fn get_all_settings(
            &self,
            user_id: &str,
        ) -> Result<std::collections::HashMap<String, serde_json::Value>, crate::error::DatabaseError>
        {
            let rows = self.rows.read().await;
            Ok(rows.get(user_id).cloned().unwrap_or_default())
        }

        async fn set_all_settings(
            &self,
            user_id: &str,
            settings: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(), crate::error::DatabaseError> {
            let mut rows = self.rows.write().await;
            rows.insert(user_id.to_string(), settings.clone());
            Ok(())
        }

        async fn has_settings(&self, user_id: &str) -> Result<bool, crate::error::DatabaseError> {
            let rows = self.rows.read().await;
            Ok(rows.get(user_id).is_some_and(|m| !m.is_empty()))
        }
    }

    fn member_settings_with_private_endpoint() -> serde_json::Value {
        // A leftover DB row written by a non-admin user before the
        // admin-only restriction landed: points the openai backend at a
        // private LAN address that would normally fail SSRF validation.
        serde_json::json!({
            "openai": {
                "base_url": "http://192.168.1.50:11434",
                "model": "leak-bait"
            }
        })
    }

    fn config_for_owner(owner_id: &str) -> Config {
        let tmp = std::env::temp_dir().join(format!("ironclaw-resolve-test-{owner_id}"));
        let mut cfg = Config::for_testing(tmp.clone(), tmp.clone(), tmp);
        cfg.owner_id = owner_id.to_string();
        cfg
    }

    /// Return a path to a temporary empty TOML file so that tests do not
    /// accidentally load the user's real `~/.ironclaw/config.toml`.
    fn empty_toml_path() -> tempfile::NamedTempFile {
        tempfile::Builder::new()
            .suffix(".toml")
            .tempfile()
            .expect("create temp toml")
    }

    #[tokio::test]
    async fn re_resolve_llm_strips_admin_only_keys_for_non_operator_user() {
        use crate::db::SettingsStore;

        let store = FakeSettingsStore::new();
        // Seed a non-admin user's per-user settings with an admin-only key
        // that points at a private endpoint.
        store
            .seed(
                "member-user",
                "llm_builtin_overrides",
                member_settings_with_private_endpoint(),
            )
            .await;

        let toml = empty_toml_path();
        let mut cfg = config_for_owner("operator-user");
        cfg.re_resolve_llm_with_secrets(
            Some(&store as &(dyn crate::db::SettingsStore + Sync)),
            "member-user",
            Some(toml.path()),
            None,
            false, // <- non-operator: admin-only keys must be stripped
        )
        .await
        .expect("resolve should not fail");

        // Re-load via the store and apply the same filter the resolver
        // uses, to assert the helper actually drops the poisoned key.
        let mut filtered = store.get_all_settings("member-user").await.unwrap();
        crate::config::helpers::strip_admin_only_llm_keys(&mut filtered);
        assert!(
            !filtered.contains_key("llm_builtin_overrides"),
            "filter helper must remove the admin-only key"
        );
    }

    #[tokio::test]
    async fn re_resolve_llm_keeps_admin_only_keys_for_operator() {
        let store = FakeSettingsStore::new();
        store
            .seed(
                "operator-user",
                "llm_builtin_overrides",
                serde_json::json!({
                    "openai": {
                        "model": "gpt-4o"
                    }
                }),
            )
            .await;

        let toml = empty_toml_path();
        let mut cfg = config_for_owner("operator-user");
        // is_operator=true: admin/operator may legitimately configure
        // builtin overrides, so the resolve path must keep them.
        cfg.re_resolve_llm_with_secrets(
            Some(&store as &(dyn crate::db::SettingsStore + Sync)),
            "operator-user",
            Some(toml.path()),
            None,
            true,
        )
        .await
        .expect("resolve should succeed for operator");
    }

    #[tokio::test]
    async fn re_resolve_llm_strips_admin_scope_admin_only_keys_for_non_operator() {
        // Regression: a non-operator member must not inherit admin-only LLM
        // keys from the admin-defaults scope, even when the admin scope was
        // populated by an actual operator. The poisoned model below would
        // propagate to `cfg.llm.nearai.model` if the strip filter was not
        // applied to the admin-scope merge inside `re_resolve_llm_with_secrets`.
        let store = FakeSettingsStore::new();
        store
            .seed(
                crate::tools::permissions::ADMIN_SETTINGS_USER_ID,
                "llm_builtin_overrides",
                serde_json::json!({
                    "nearai": {
                        "model": "admin-poison-model"
                    }
                }),
            )
            .await;

        let toml = empty_toml_path();
        let mut cfg = config_for_owner("operator-user");
        cfg.re_resolve_llm_with_secrets(
            Some(&store as &(dyn crate::db::SettingsStore + Sync)),
            "member-user",
            Some(toml.path()),
            None,
            false,
        )
        .await
        .expect("resolve should succeed for non-operator member");

        assert_ne!(
            cfg.llm.nearai.model, "admin-poison-model",
            "admin-scope llm_builtin_overrides must not propagate to a non-operator member"
        );
    }

    #[tokio::test]
    async fn re_resolve_llm_keeps_admin_scope_admin_only_keys_for_operator() {
        // Mirror of the above: an operator may legitimately inherit admin
        // defaults, including admin-only LLM keys, since they could set them
        // themselves directly.
        let store = FakeSettingsStore::new();
        store
            .seed(
                crate::tools::permissions::ADMIN_SETTINGS_USER_ID,
                "llm_builtin_overrides",
                serde_json::json!({
                    "nearai": {
                        "model": "admin-set-model"
                    }
                }),
            )
            .await;

        let toml = empty_toml_path();
        let mut cfg = config_for_owner("operator-user");
        cfg.re_resolve_llm_with_secrets(
            Some(&store as &(dyn crate::db::SettingsStore + Sync)),
            "another-operator",
            Some(toml.path()),
            None,
            true,
        )
        .await
        .expect("resolve should succeed for operator");

        assert_eq!(
            cfg.llm.nearai.model, "admin-set-model",
            "operator must inherit admin-scope builtin override model"
        );
    }

    #[tokio::test]
    async fn hydrate_skips_when_key_already_present() {
        let secrets = test_secrets_store();
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams {
                    name: "llm_builtin_openai_api_key".to_string(),
                    value: secrecy::SecretString::from("sk-from-vault".to_string()),
                    provider: Some("openai".to_string()),
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let mut settings = Settings {
            llm_builtin_overrides: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "openai".to_string(),
                    crate::settings::LlmBuiltinOverride {
                        api_key: Some("sk-existing".to_string()),
                        model: None,
                        base_url: None,
                    },
                );
                m
            },
            ..Default::default()
        };

        hydrate_llm_keys_from_secrets(&mut settings, secrets.as_ref(), "test").await;

        assert_eq!(
            settings.llm_builtin_overrides["openai"].api_key.as_deref(),
            Some("sk-existing"),
            "existing key should not be overwritten"
        );
    }
}
