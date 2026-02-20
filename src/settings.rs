//! User settings persistence.
//!
//! Stores user preferences in ~/.ironclaw/settings.json.
//! Settings are loaded with env var > settings.json > default priority.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// User settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Whether onboarding wizard has been completed.
    #[serde(default, alias = "setup_completed")]
    pub onboard_completed: bool,

    // === Step 1: Database ===
    /// Database backend: "postgres" or "libsql".
    #[serde(default)]
    pub database_backend: Option<String>,

    /// Database connection URL (postgres://...).
    #[serde(default)]
    pub database_url: Option<String>,

    /// Database pool size.
    #[serde(default)]
    pub database_pool_size: Option<usize>,

    /// Path to local libSQL database file.
    #[serde(default)]
    pub libsql_path: Option<String>,

    /// Turso cloud URL for remote replica sync.
    #[serde(default)]
    pub libsql_url: Option<String>,

    // === Step 2: Security ===
    /// Source for the secrets master key.
    #[serde(default)]
    pub secrets_master_key_source: KeySource,

    // === Step 3: Inference Provider ===
    /// LLM backend: "nearai", "anthropic", "openai", "ollama", "openai_compatible".
    #[serde(default)]
    pub llm_backend: Option<String>,

    /// Ollama base URL (when llm_backend = "ollama").
    #[serde(default)]
    pub ollama_base_url: Option<String>,

    /// OpenAI-compatible endpoint base URL (when llm_backend = "openai_compatible").
    #[serde(default)]
    pub openai_compatible_base_url: Option<String>,

    // === Step 4: Model Selection ===
    /// Currently selected model.
    #[serde(default)]
    pub selected_model: Option<String>,

    // === Step 5: Embeddings ===
    /// Embeddings configuration.
    #[serde(default)]
    pub embeddings: EmbeddingsSettings,

    // === Step 6: Channels ===
    /// Tunnel configuration for public webhook endpoints.
    #[serde(default)]
    pub tunnel: TunnelSettings,

    /// Channel configuration.
    #[serde(default)]
    pub channels: ChannelSettings,

    // === Step 7: Heartbeat ===
    /// Heartbeat configuration.
    #[serde(default)]
    pub heartbeat: HeartbeatSettings,

    // === Advanced Settings (not asked during setup, editable via CLI) ===
    /// Agent behavior configuration.
    #[serde(default)]
    pub agent: AgentSettings,

    /// WASM sandbox configuration.
    #[serde(default)]
    pub wasm: WasmSettings,

    /// Docker sandbox configuration.
    #[serde(default)]
    pub sandbox: SandboxSettings,

    /// Safety configuration.
    #[serde(default)]
    pub safety: SafetySettings,

    /// Builder configuration.
    #[serde(default)]
    pub builder: BuilderSettings,
}

/// Source for the secrets master key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Auto-generated key stored in OS keychain.
    Keychain,
    /// User provides via SECRETS_MASTER_KEY env var.
    Env,
    /// Not configured (secrets features disabled).
    #[default]
    None,
}

/// Embeddings configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsSettings {
    /// Whether embeddings are enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Provider to use: "openai" or "nearai".
    #[serde(default = "default_embeddings_provider")]
    pub provider: String,

    /// Model to use for embeddings.
    #[serde(default = "default_embeddings_model")]
    pub model: String,
}

fn default_embeddings_provider() -> String {
    "nearai".to_string()
}

fn default_embeddings_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for EmbeddingsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_embeddings_provider(),
            model: default_embeddings_model(),
        }
    }
}

/// Tunnel settings for public webhook endpoints.
///
/// The tunnel URL is shared across all channels that need webhooks.
/// Two modes:
/// - **Static URL**: `public_url` set directly (manual tunnel management).
/// - **Managed provider**: `provider` is set and the agent starts/stops the
///   tunnel process automatically at boot/shutdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelSettings {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// When set without a provider, treated as a static (externally managed) URL.
    #[serde(default)]
    pub public_url: Option<String>,

    /// Managed tunnel provider: "ngrok", "cloudflare", "tailscale", "custom".
    #[serde(default)]
    pub provider: Option<String>,

    /// Cloudflare tunnel token.
    #[serde(default)]
    pub cf_token: Option<String>,

    /// ngrok auth token.
    #[serde(default)]
    pub ngrok_token: Option<String>,

    /// ngrok custom domain (paid plans).
    #[serde(default)]
    pub ngrok_domain: Option<String>,

    /// Use Tailscale Funnel (public) instead of Serve (tailnet-only).
    #[serde(default)]
    pub ts_funnel: bool,

    /// Tailscale hostname override.
    #[serde(default)]
    pub ts_hostname: Option<String>,

    /// Shell command for custom tunnel (with `{port}` / `{host}` placeholders).
    #[serde(default)]
    pub custom_command: Option<String>,

    /// Health check URL for custom tunnel.
    #[serde(default)]
    pub custom_health_url: Option<String>,

    /// Substring pattern to extract URL from custom tunnel stdout.
    #[serde(default)]
    pub custom_url_pattern: Option<String>,
}

/// Channel-specific settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelSettings {
    /// Whether HTTP webhook channel is enabled.
    #[serde(default)]
    pub http_enabled: bool,

    /// HTTP webhook port (if enabled).
    #[serde(default)]
    pub http_port: Option<u16>,

    /// HTTP webhook host.
    #[serde(default)]
    pub http_host: Option<String>,

    /// Telegram owner user ID. When set, the bot only responds to this user.
    /// Captured during setup by having the user message the bot.
    #[serde(default)]
    pub telegram_owner_id: Option<i64>,

    /// Enabled WASM channels by name.
    /// Channels not in this list but present in the channels directory will still load.
    /// This is primarily used by the setup wizard to track which channels were configured.
    #[serde(default)]
    pub wasm_channels: Vec<String>,

    /// Whether WASM channels are enabled.
    #[serde(default = "default_true")]
    pub wasm_channels_enabled: bool,

    /// Directory containing WASM channel modules.
    #[serde(default)]
    pub wasm_channels_dir: Option<PathBuf>,
}

/// Heartbeat configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatSettings {
    /// Whether heartbeat is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Interval between heartbeat checks in seconds.
    #[serde(default = "default_heartbeat_interval")]
    pub interval_secs: u64,

    /// Channel to notify on heartbeat findings.
    #[serde(default)]
    pub notify_channel: Option<String>,

    /// User ID to notify on heartbeat findings.
    #[serde(default)]
    pub notify_user: Option<String>,
}

fn default_heartbeat_interval() -> u64 {
    1800 // 30 minutes
}

impl Default for HeartbeatSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_heartbeat_interval(),
            notify_channel: None,
            notify_user: None,
        }
    }
}

/// Agent behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Agent name.
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// Maximum parallel jobs.
    #[serde(default = "default_max_parallel_jobs")]
    pub max_parallel_jobs: u32,

    /// Job timeout in seconds.
    #[serde(default = "default_job_timeout")]
    pub job_timeout_secs: u64,

    /// Stuck job threshold in seconds.
    #[serde(default = "default_stuck_threshold")]
    pub stuck_threshold_secs: u64,

    /// Whether to use planning before tool execution.
    #[serde(default = "default_true")]
    pub use_planning: bool,

    /// Self-repair check interval in seconds.
    #[serde(default = "default_repair_interval")]
    pub repair_check_interval_secs: u64,

    /// Maximum repair attempts.
    #[serde(default = "default_max_repair_attempts")]
    pub max_repair_attempts: u32,

    /// Session idle timeout in seconds (default: 7 days). Sessions inactive
    /// longer than this are pruned from memory.
    #[serde(default = "default_session_idle_timeout")]
    pub session_idle_timeout_secs: u64,
}

fn default_agent_name() -> String {
    "ironclaw".to_string()
}

fn default_max_parallel_jobs() -> u32 {
    5
}

fn default_job_timeout() -> u64 {
    3600 // 1 hour
}

fn default_stuck_threshold() -> u64 {
    300 // 5 minutes
}

fn default_repair_interval() -> u64 {
    60 // 1 minute
}

fn default_session_idle_timeout() -> u64 {
    7 * 24 * 3600 // 7 days
}

fn default_max_repair_attempts() -> u32 {
    3
}

fn default_true() -> bool {
    true
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            max_parallel_jobs: default_max_parallel_jobs(),
            job_timeout_secs: default_job_timeout(),
            stuck_threshold_secs: default_stuck_threshold(),
            use_planning: true,
            repair_check_interval_secs: default_repair_interval(),
            max_repair_attempts: default_max_repair_attempts(),
            session_idle_timeout_secs: default_session_idle_timeout(),
        }
    }
}

/// WASM sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSettings {
    /// Whether WASM tool execution is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Directory containing installed WASM tools.
    #[serde(default)]
    pub tools_dir: Option<PathBuf>,

    /// Default memory limit in bytes.
    #[serde(default = "default_wasm_memory_limit")]
    pub default_memory_limit: u64,

    /// Default execution timeout in seconds.
    #[serde(default = "default_wasm_timeout")]
    pub default_timeout_secs: u64,

    /// Default fuel limit for CPU metering.
    #[serde(default = "default_wasm_fuel_limit")]
    pub default_fuel_limit: u64,

    /// Whether to cache compiled modules.
    #[serde(default = "default_true")]
    pub cache_compiled: bool,

    /// Directory for compiled module cache.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

fn default_wasm_memory_limit() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

fn default_wasm_timeout() -> u64 {
    60
}

fn default_wasm_fuel_limit() -> u64 {
    10_000_000
}

impl Default for WasmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: None,
            default_memory_limit: default_wasm_memory_limit(),
            default_timeout_secs: default_wasm_timeout(),
            default_fuel_limit: default_wasm_fuel_limit(),
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

/// Docker sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// Whether the Docker sandbox is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    #[serde(default = "default_sandbox_policy")]
    pub policy: String,

    /// Command timeout in seconds.
    #[serde(default = "default_sandbox_timeout")]
    pub timeout_secs: u64,

    /// Memory limit in megabytes.
    #[serde(default = "default_sandbox_memory")]
    pub memory_limit_mb: u64,

    /// CPU shares (relative weight).
    #[serde(default = "default_sandbox_cpu_shares")]
    pub cpu_shares: u32,

    /// Docker image for the sandbox.
    #[serde(default = "default_sandbox_image")]
    pub image: String,

    /// Whether to auto-pull the image if not found.
    #[serde(default = "default_true")]
    pub auto_pull_image: bool,

    /// Additional domains to allow through the network proxy.
    #[serde(default)]
    pub extra_allowed_domains: Vec<String>,
}

fn default_sandbox_policy() -> String {
    "readonly".to_string()
}

fn default_sandbox_timeout() -> u64 {
    120
}

fn default_sandbox_memory() -> u64 {
    2048
}

fn default_sandbox_cpu_shares() -> u32 {
    1024
}

fn default_sandbox_image() -> String {
    "ironclaw-worker:latest".to_string()
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: default_sandbox_policy(),
            timeout_secs: default_sandbox_timeout(),
            memory_limit_mb: default_sandbox_memory(),
            cpu_shares: default_sandbox_cpu_shares(),
            image: default_sandbox_image(),
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

/// Safety configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetySettings {
    /// Maximum output length in bytes.
    #[serde(default = "default_max_output_length")]
    pub max_output_length: usize,

    /// Whether injection check is enabled.
    #[serde(default = "default_true")]
    pub injection_check_enabled: bool,
}

fn default_max_output_length() -> usize {
    100_000
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            max_output_length: default_max_output_length(),
            injection_check_enabled: true,
        }
    }
}

/// Builder configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderSettings {
    /// Whether the software builder tool is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Directory for build artifacts.
    #[serde(default)]
    pub build_dir: Option<PathBuf>,

    /// Maximum iterations for the build loop.
    #[serde(default = "default_builder_max_iterations")]
    pub max_iterations: u32,

    /// Build timeout in seconds.
    #[serde(default = "default_builder_timeout")]
    pub timeout_secs: u64,

    /// Whether to automatically register built WASM tools.
    #[serde(default = "default_true")]
    pub auto_register: bool,
}

fn default_builder_max_iterations() -> u32 {
    20
}

fn default_builder_timeout() -> u64 {
    600
}

impl Default for BuilderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            build_dir: None,
            max_iterations: default_builder_max_iterations(),
            timeout_secs: default_builder_timeout(),
            auto_register: true,
        }
    }
}

impl Settings {
    /// Reconstruct Settings from a flat key-value map (as stored in the DB).
    ///
    /// Each key is a dotted path (e.g., "agent.name"), value is a JSONB value.
    /// Missing keys get their default value.
    pub fn from_db_map(map: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        // Start with defaults, then overlay each DB setting.
        //
        // The settings table stores both Settings struct fields and app-specific
        // data (e.g. nearai.session_token). Skip keys that don't correspond to
        // a known Settings path.
        let mut settings = Self::default();

        for (key, value) in map {
            // Convert the JSONB value to a string for the existing set() method
            let value_str = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Null => continue, // null means default, skip
                other => other.to_string(),
            };

            match settings.set(key, &value_str) {
                Ok(()) => {}
                // The settings table stores both Settings fields and app-specific
                // data (e.g. nearai.session_token). Silently skip unknown paths.
                Err(e) if e.starts_with("Path not found") => {}
                Err(e) => {
                    tracing::warn!(
                        "Failed to apply DB setting '{}' = '{}': {}",
                        key,
                        value_str,
                        e
                    );
                }
            }
        }

        settings
    }

    /// Flatten Settings into a key-value map suitable for DB storage.
    ///
    /// Each entry is a (dotted_path, JSONB value) pair.
    pub fn to_db_map(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return std::collections::HashMap::new(),
        };

        let mut map = std::collections::HashMap::new();
        collect_settings_json(&json, String::new(), &mut map);
        map
    }

    /// Get the default settings file path (~/.ironclaw/settings.json).
    pub fn default_path() -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".ironclaw")
            .join("settings.json")
    }

    /// Load settings from disk, returning default if not found.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Load settings from a specific path (used by bootstrap legacy migration).
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Default TOML config file path (~/.ironclaw/config.toml).
    pub fn default_toml_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw")
            .join("config.toml")
    }

    /// Load settings from a TOML file.
    ///
    /// Returns `None` if the file doesn't exist. Returns an error only
    /// if the file exists but can't be parsed.
    pub fn load_toml(path: &std::path::Path) -> Result<Option<Self>, String> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("failed to read {}: {}", path.display(), e)),
        };

        let settings: Self = toml::from_str(&data)
            .map_err(|e| format!("invalid TOML in {}: {}", path.display(), e))?;
        Ok(Some(settings))
    }

    /// Write a well-commented TOML config file with current settings.
    pub fn save_toml(&self, path: &std::path::Path) -> Result<(), String> {
        let raw = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize settings: {}", e))?;

        let content = format!(
            "# IronClaw configuration file.\n\
             #\n\
             # Priority: env var > this file > database settings > defaults.\n\
             # Uncomment and edit values to override defaults.\n\
             # Run `ironclaw config init` to regenerate this file.\n\
             #\n\
             # Documentation: https://github.com/nearai/ironclaw\n\
             \n\
             {raw}"
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }

        std::fs::write(path, content)
            .map_err(|e| format!("failed to write {}: {}", path.display(), e))
    }

    /// Merge values from `other` into `self`, preferring `other` for
    /// fields that differ from the default.
    ///
    /// This enables layering: load DB/JSON settings as the base, then
    /// overlay TOML values on top. Only fields that the TOML file
    /// explicitly changed (i.e. differ from Default) are applied.
    pub fn merge_from(&mut self, other: &Self) {
        let default_json = match serde_json::to_value(Self::default()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let other_json = match serde_json::to_value(other) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut self_json = match serde_json::to_value(&*self) {
            Ok(v) => v,
            Err(_) => return,
        };

        merge_non_default(&mut self_json, &other_json, &default_json);

        if let Ok(merged) = serde_json::from_value(self_json) {
            *self = merged;
        }
    }

    /// Get a setting value by dotted path (e.g., "agent.max_parallel_jobs").
    pub fn get(&self, path: &str) -> Option<String> {
        let json = serde_json::to_value(self).ok()?;
        let mut current = &json;

        for part in path.split('.') {
            current = current.get(part)?;
        }

        match current {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            serde_json::Value::Null => Some("null".to_string()),
            serde_json::Value::Array(arr) => Some(serde_json::to_string(arr).unwrap_or_default()),
            serde_json::Value::Object(obj) => Some(serde_json::to_string(obj).unwrap_or_default()),
        }
    }

    /// Set a setting value by dotted path.
    ///
    /// Returns error if path is invalid or value cannot be parsed.
    pub fn set(&mut self, path: &str, value: &str) -> Result<(), String> {
        let mut json = serde_json::to_value(&self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Err("Empty path".to_string());
        }

        // Navigate to parent and set the final key
        let mut current = &mut json;
        for part in &parts[..parts.len() - 1] {
            current = current
                .get_mut(*part)
                .ok_or_else(|| format!("Path not found: {}", path))?;
        }

        let final_key = parts.last().unwrap();
        let obj = current
            .as_object_mut()
            .ok_or_else(|| format!("Parent is not an object: {}", path))?;

        // Try to infer the type from the existing value
        let new_value = if let Some(existing) = obj.get(*final_key) {
            match existing {
                serde_json::Value::Bool(_) => {
                    let b = value
                        .parse::<bool>()
                        .map_err(|_| format!("Expected boolean for {}, got '{}'", path, value))?;
                    serde_json::Value::Bool(b)
                }
                serde_json::Value::Number(n) => {
                    if n.is_u64() {
                        let n = value.parse::<u64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else if n.is_i64() {
                        let n = value.parse::<i64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else {
                        let n = value.parse::<f64>().map_err(|_| {
                            format!("Expected number for {}, got '{}'", path, value)
                        })?;
                        serde_json::Number::from_f64(n)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::String(value.to_string()))
                    }
                }
                serde_json::Value::Null => {
                    // Could be Option<T>, try to parse as JSON or use string
                    serde_json::from_str(value)
                        .unwrap_or(serde_json::Value::String(value.to_string()))
                }
                serde_json::Value::Array(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON array for {}: {}", path, e))?,
                serde_json::Value::Object(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON object for {}: {}", path, e))?,
                serde_json::Value::String(_) => serde_json::Value::String(value.to_string()),
            }
        } else {
            // Key doesn't exist, try to parse as JSON or use string
            serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.to_string()))
        };

        obj.insert((*final_key).to_string(), new_value);

        // Deserialize back to Settings
        *self =
            serde_json::from_value(json).map_err(|e| format!("Failed to apply setting: {}", e))?;

        Ok(())
    }

    /// Reset a setting to its default value.
    pub fn reset(&mut self, path: &str) -> Result<(), String> {
        let default = Self::default();
        let default_value = default
            .get(path)
            .ok_or_else(|| format!("Unknown setting: {}", path))?;

        self.set(path, &default_value)
    }

    /// List all settings as (path, value) pairs.
    pub fn list(&self) -> Vec<(String, String)> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        collect_settings(&json, String::new(), &mut results);
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

/// Recursively collect settings paths with their JSON values (for DB storage).
fn collect_settings_json(
    value: &serde_json::Value,
    prefix: String,
    results: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings_json(val, path, results);
            }
        }
        other => {
            results.insert(prefix, other.clone());
        }
    }
}

/// Recursively collect settings paths and values.
fn collect_settings(
    value: &serde_json::Value,
    prefix: String,
    results: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings(val, path, results);
            }
        }
        serde_json::Value::Array(arr) => {
            let display = serde_json::to_string(arr).unwrap_or_default();
            results.push((prefix, display));
        }
        serde_json::Value::String(s) => {
            results.push((prefix, s.clone()));
        }
        serde_json::Value::Number(n) => {
            results.push((prefix, n.to_string()));
        }
        serde_json::Value::Bool(b) => {
            results.push((prefix, b.to_string()));
        }
        serde_json::Value::Null => {
            results.push((prefix, "null".to_string()));
        }
    }
}

/// Recursively merge `other` into `target`, but only for fields where
/// `other` differs from `defaults`. This means only explicitly-set values
/// in the TOML file override the base settings.
fn merge_non_default(
    target: &mut serde_json::Value,
    other: &serde_json::Value,
    defaults: &serde_json::Value,
) {
    match (target, other, defaults) {
        (
            serde_json::Value::Object(t),
            serde_json::Value::Object(o),
            serde_json::Value::Object(d),
        ) => {
            for (key, other_val) in o {
                let default_val = d.get(key).cloned().unwrap_or(serde_json::Value::Null);
                if let Some(target_val) = t.get_mut(key) {
                    merge_non_default(target_val, other_val, &default_val);
                } else if other_val != &default_val {
                    t.insert(key.clone(), other_val.clone());
                }
            }
        }
        (target, other, defaults) => {
            if other != defaults {
                *target = other.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::*;

    #[test]
    fn test_db_map_round_trip() {
        let settings = Settings {
            selected_model: Some("claude-3-5-sonnet-20241022".to_string()),
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(
            restored.selected_model,
            Some("claude-3-5-sonnet-20241022".to_string())
        );
    }

    #[test]
    fn test_get_setting() {
        let settings = Settings::default();

        assert_eq!(settings.get("agent.name"), Some("ironclaw".to_string()));
        assert_eq!(
            settings.get("agent.max_parallel_jobs"),
            Some("5".to_string())
        );
        assert_eq!(settings.get("heartbeat.enabled"), Some("false".to_string()));
        assert_eq!(settings.get("nonexistent"), None);
    }

    #[test]
    fn test_set_setting() {
        let mut settings = Settings::default();

        settings.set("agent.name", "mybot").unwrap();
        assert_eq!(settings.agent.name, "mybot");

        settings.set("agent.max_parallel_jobs", "10").unwrap();
        assert_eq!(settings.agent.max_parallel_jobs, 10);

        settings.set("heartbeat.enabled", "true").unwrap();
        assert!(settings.heartbeat.enabled);
    }

    #[test]
    fn test_reset_setting() {
        let mut settings = Settings::default();

        settings.agent.name = "custom".to_string();
        settings.reset("agent.name").unwrap();
        assert_eq!(settings.agent.name, "ironclaw");
    }

    #[test]
    fn test_list_settings() {
        let settings = Settings::default();
        let list = settings.list();

        // Check some expected entries
        assert!(list.iter().any(|(k, _)| k == "agent.name"));
        assert!(list.iter().any(|(k, _)| k == "heartbeat.enabled"));
        assert!(list.iter().any(|(k, _)| k == "onboard_completed"));
    }

    #[test]
    fn test_key_source_serialization() {
        let settings = Settings {
            secrets_master_key_source: KeySource::Keychain,
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"keychain\""));

        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.secrets_master_key_source, KeySource::Keychain);
    }

    #[test]
    fn test_embeddings_defaults() {
        let settings = Settings::default();
        assert!(!settings.embeddings.enabled);
        assert_eq!(settings.embeddings.provider, "nearai");
        assert_eq!(settings.embeddings.model, "text-embedding-3-small");
    }

    #[test]
    fn test_telegram_owner_id_db_round_trip() {
        let mut settings = Settings::default();
        settings.channels.telegram_owner_id = Some(123456789);

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(restored.channels.telegram_owner_id, Some(123456789));
    }

    #[test]
    fn test_telegram_owner_id_default_none() {
        let settings = Settings::default();
        assert_eq!(settings.channels.telegram_owner_id, None);
    }

    #[test]
    fn test_telegram_owner_id_via_set() {
        let mut settings = Settings::default();
        settings
            .set("channels.telegram_owner_id", "987654321")
            .unwrap();
        assert_eq!(settings.channels.telegram_owner_id, Some(987654321));
    }

    #[test]
    fn test_llm_backend_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            ollama_base_url: Some("http://localhost:11434".to_string()),
            openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.llm_backend, Some("anthropic".to_string()));
        assert_eq!(
            loaded.ollama_base_url,
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(
            loaded.openai_compatible_base_url,
            Some("http://my-vllm:8000/v1".to_string())
        );
    }

    #[test]
    fn test_openai_compatible_db_map_round_trip() {
        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);

        assert_eq!(
            restored.llm_backend,
            Some("openai_compatible".to_string()),
            "llm_backend must survive DB round-trip"
        );
        assert_eq!(
            restored.openai_compatible_base_url,
            Some("http://my-vllm:8000/v1".to_string()),
            "openai_compatible_base_url must survive DB round-trip"
        );
        assert!(
            !restored.embeddings.enabled,
            "embeddings.enabled=false must survive DB round-trip"
        );
    }

    #[test]
    fn toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut settings = Settings::default();
        settings.agent.name = "toml-bot".to_string();
        settings.heartbeat.enabled = true;
        settings.heartbeat.interval_secs = 900;

        settings.save_toml(&path).unwrap();
        let loaded = Settings::load_toml(&path).unwrap().unwrap();

        assert_eq!(loaded.agent.name, "toml-bot");
        assert!(loaded.heartbeat.enabled);
        assert_eq!(loaded.heartbeat.interval_secs, 900);
    }

    #[test]
    fn toml_missing_file_returns_none() {
        let result = Settings::load_toml(std::path::Path::new("/tmp/nonexistent_config.toml"));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn toml_invalid_content_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is not valid toml [[[").unwrap();

        let result = Settings::load_toml(&path);
        assert!(result.is_err());
    }

    #[test]
    fn toml_partial_config_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.toml");

        // Only set agent name, everything else should be default
        std::fs::write(&path, "[agent]\nname = \"partial-bot\"\n").unwrap();

        let loaded = Settings::load_toml(&path).unwrap().unwrap();
        assert_eq!(loaded.agent.name, "partial-bot");
        // Defaults preserved
        assert_eq!(loaded.agent.max_parallel_jobs, 5);
        assert!(!loaded.heartbeat.enabled);
    }

    #[test]
    fn toml_header_comment_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        Settings::default().save_toml(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(content.starts_with("# IronClaw configuration file."));
        assert!(content.contains("[agent]"));
        assert!(content.contains("[heartbeat]"));
    }

    #[test]
    fn merge_only_overrides_non_default_values() {
        let mut base = Settings::default();
        base.agent.name = "from-db".to_string();
        base.heartbeat.interval_secs = 600;

        let mut toml_overlay = Settings::default();
        toml_overlay.agent.name = "from-toml".to_string();

        base.merge_from(&toml_overlay);

        assert_eq!(base.agent.name, "from-toml");
        assert_eq!(base.heartbeat.interval_secs, 600);
    }

    #[test]
    fn merge_preserves_base_when_overlay_is_default() {
        let mut base = Settings::default();
        base.agent.name = "custom-name".to_string();
        base.heartbeat.enabled = true;

        let overlay = Settings::default();
        base.merge_from(&overlay);

        assert_eq!(base.agent.name, "custom-name");
        assert!(base.heartbeat.enabled);
    }

    #[test]
    fn toml_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("config.toml");

        Settings::default().save_toml(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn default_toml_path_under_ironclaw() {
        let path = Settings::default_toml_path();
        assert!(path.to_string_lossy().contains(".ironclaw"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn tunnel_settings_round_trip() {
        let settings = Settings {
            tunnel: TunnelSettings {
                provider: Some("ngrok".to_string()),
                ngrok_token: Some("tok_abc123".to_string()),
                ngrok_domain: Some("my.ngrok.dev".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        // JSON round-trip
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.tunnel.provider, Some("ngrok".to_string()));
        assert_eq!(restored.tunnel.ngrok_token, Some("tok_abc123".to_string()));
        assert_eq!(
            restored.tunnel.ngrok_domain,
            Some("my.ngrok.dev".to_string())
        );
        assert!(restored.tunnel.public_url.is_none());

        // DB map round-trip
        let map = settings.to_db_map();
        let from_db = Settings::from_db_map(&map);
        assert_eq!(from_db.tunnel.provider, Some("ngrok".to_string()));
        assert_eq!(from_db.tunnel.ngrok_token, Some("tok_abc123".to_string()));

        // get/set round-trip
        let mut s = Settings::default();
        s.set("tunnel.provider", "cloudflare").unwrap();
        s.set("tunnel.cf_token", "cf_tok_xyz").unwrap();
        s.set("tunnel.ts_funnel", "true").unwrap();
        assert_eq!(s.tunnel.provider, Some("cloudflare".to_string()));
        assert_eq!(s.tunnel.cf_token, Some("cf_tok_xyz".to_string()));
        assert!(s.tunnel.ts_funnel);
    }
}
