//! User settings persistence.
//!
//! Stores user preferences in `~/.ironclaw` (JSON/TOML) and, for some values,
//! in the database. At runtime, precedence between database values,
//! environment variables, on-disk config, and built-in defaults is determined
//! on a per-setting basis by the corresponding resolver.
//! LLM backend and related settings in particular may prefer DB values over
//! environment variables, as documented on their respective types.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;

/// A custom LLM provider defined by the user through the web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomLlmProviderSettings {
    /// Unique identifier (used as `llm_backend` value).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Adapter protocol: "open_ai_completions", "anthropic", "ollama".
    pub adapter: String,
    /// Base URL for the API endpoint.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Default model identifier.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Optional API key stored inline.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whether this is a built-in provider (should always be false for custom).
    #[serde(default)]
    pub builtin: bool,
}

/// Per-provider overrides for built-in LLM providers (API key and/or model).
///
/// Stored as `llm_builtin_overrides` in the settings store, keyed by provider ID
/// (e.g. `"openai"`, `"gemini"`). Resolved at startup during `LlmConfig::resolve()`.
///
/// Note: The global `selected_model` (if set) takes precedence over these
/// per-provider overrides, which in turn take precedence over environment variables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmBuiltinOverride {
    /// API key override. Takes precedence over environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Model override. Takes precedence over environment variables but not `selected_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Base URL override. Takes precedence over environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Canonical secret name for a built-in provider's API key.
pub fn builtin_secret_name(provider_id: &str) -> String {
    format!("llm_builtin_{provider_id}_api_key")
}

/// Canonical secret name for a custom provider's API key.
pub fn custom_secret_name(provider_id: &str) -> String {
    format!("llm_custom_{provider_id}_api_key")
}

/// User settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Whether onboarding wizard has been completed.
    #[serde(default, alias = "setup_completed")]
    pub onboard_completed: bool,

    /// Stable owner scope for this IronClaw instance.
    ///
    /// This is bootstrap configuration loaded from env / disk / TOML. We do
    /// not persist it in the per-user DB settings table because the DB lookup
    /// itself already requires the owner scope to be known.
    #[serde(default)]
    pub owner_id: Option<String>,

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

    /// Generated master key hex (env var mode only, written to .env by wizard).
    #[serde(default, skip_serializing)]
    pub secrets_master_key_hex: Option<String>,

    // === Step 3: Inference Provider ===
    /// LLM backend: "nearai", "anthropic", "openai", "github_copilot", "ollama", "openai_compatible", "tinfoil", "bedrock".
    #[serde(default)]
    pub llm_backend: Option<String>,

    /// Custom LLM providers defined by the user through the web UI.
    #[serde(default)]
    pub llm_custom_providers: Vec<CustomLlmProviderSettings>,

    /// Per-provider overrides for built-in providers (API key and/or model).
    #[serde(default)]
    pub llm_builtin_overrides: HashMap<String, LlmBuiltinOverride>,

    /// Ollama base URL (when llm_backend = "ollama").
    #[serde(default)]
    pub ollama_base_url: Option<String>,

    /// OpenAI-compatible endpoint base URL (when llm_backend = "openai_compatible").
    #[serde(default)]
    pub openai_compatible_base_url: Option<String>,

    /// Bedrock region (when llm_backend = "bedrock").
    #[serde(default)]
    pub bedrock_region: Option<String>,

    /// Bedrock cross-region inference prefix (when llm_backend = "bedrock").
    #[serde(default)]
    pub bedrock_cross_region: Option<String>,

    /// AWS profile name for Bedrock (when llm_backend = "bedrock").
    #[serde(default)]
    pub bedrock_profile: Option<String>,

    // === Step 4: Model Selection ===
    /// Currently selected model.
    #[serde(default)]
    pub selected_model: Option<String>,

    /// Default sampling temperature for LLM requests (0.0–2.0).
    /// When set, used as the default for conversational turns.
    /// Per-request temperature (e.g. from the API) takes precedence.
    #[serde(default)]
    pub temperature: Option<f32>,

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

    // === Conversational Profile Onboarding ===
    /// Whether the conversational profile onboarding has been completed.
    ///
    /// Set during the user's first interaction with the running assistant
    /// (not during the setup wizard), after the agent builds a psychographic
    /// profile via `memory_write`. Used by the agent loop (via workspace
    /// system-prompt wiring) to suppress BOOTSTRAP.md injection once
    /// onboarding is complete.
    #[serde(default, alias = "personal_onboarding_completed")]
    pub profile_onboarding_completed: bool,

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

    /// Routine scheduling and execution configuration.
    #[serde(default)]
    pub routines: RoutineSettings,

    /// Skills system configuration.
    #[serde(default)]
    pub skills: SkillsSettings,

    /// Memory hygiene configuration.
    #[serde(default)]
    pub hygiene: HygieneSettings,

    /// Workspace search fusion configuration.
    #[serde(default)]
    pub search: SearchSettings,

    /// Mission configuration.
    #[serde(default)]
    pub missions: MissionSettings,

    /// Transcription configuration.
    #[serde(default)]
    pub transcription: Option<TranscriptionSettings>,

    /// Per-tool permission overrides.
    ///
    /// Keys are tool names; values override the built-in tier defaults from
    /// `TOOL_RISK_DEFAULTS`.  Absent tools fall back to the tier default, or
    /// `AskEachTime` if the tool is unknown.
    #[serde(default)]
    pub tool_permissions:
        std::collections::HashMap<String, crate::tools::permissions::PermissionState>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Whether the web gateway is enabled.
    #[serde(default = "default_true")]
    pub gateway_enabled: bool,

    /// Web gateway listen host.
    #[serde(default)]
    pub gateway_host: Option<String>,

    /// Web gateway listen port.
    #[serde(default)]
    pub gateway_port: Option<u16>,

    /// Web gateway bearer auth token. Auto-generated at gateway startup if unset.
    #[serde(default)]
    pub gateway_auth_token: Option<String>,

    /// Whether the CLI channel is enabled.
    #[serde(default = "default_true")]
    pub cli_enabled: bool,

    /// Whether Signal channel is enabled.
    #[serde(default)]
    pub signal_enabled: bool,

    /// Signal HTTP URL (signal-cli daemon endpoint).
    #[serde(default)]
    pub signal_http_url: Option<String>,

    /// Signal account (E.164 phone number).
    #[serde(default)]
    pub signal_account: Option<String>,

    /// Signal allow from list for DMs (comma-separated E.164 phone numbers).
    /// Comma-separated identifiers: E.164 phone numbers, `*`, bare UUIDs, or `uuid:<id>` entries.
    /// Defaults to the configured account.
    #[serde(default)]
    pub signal_allow_from: Option<String>,

    /// Signal allow from groups (comma-separated group IDs).
    #[serde(default)]
    pub signal_allow_from_groups: Option<String>,

    /// Signal DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    #[serde(default)]
    pub signal_dm_policy: Option<String>,

    /// Signal group policy: "allowlist", "open", or "disabled". Default: "allowlist".
    #[serde(default)]
    pub signal_group_policy: Option<String>,

    /// Signal group allow from (comma-separated group member IDs).
    /// If empty, inherits from signal_allow_from.
    #[serde(default)]
    pub signal_group_allow_from: Option<String>,

    /// Matrix homeserver URL (e.g. `https://matrix.org`).
    #[serde(default)]
    pub matrix_homeserver: Option<String>,

    /// Matrix bot username (localpart only, e.g. `mybot`).
    /// Wizard-only: saved during onboarding so re-onboarding can show the
    /// current value and only require re-entering the password. Not read by
    /// `ChannelsConfig::resolve` and not present in `MatrixConfig`.
    #[serde(default)]
    pub matrix_username: Option<String>,

    /// Matrix bot access token (syt_...).
    /// Prefer storing in the encrypted secrets DB via `ironclaw onboard`.
    #[serde(default, skip_serializing)]
    pub matrix_access_token: Option<String>,

    /// Matrix DM allow list (comma-separated Matrix user IDs).
    #[serde(default)]
    pub matrix_allow_from: Option<String>,

    /// Matrix DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    #[serde(default)]
    pub matrix_dm_policy: Option<String>,

    /// Matrix owner restriction — only this user ID is accepted.
    #[serde(default)]
    pub matrix_owner_id: Option<String>,

    /// Matrix sync poll interval in seconds. Default: 5.
    #[serde(default)]
    pub matrix_poll_interval_secs: Option<u32>,

    /// Whether to auto-join invited rooms. Default: true.
    #[serde(default)]
    pub matrix_auto_join: Option<bool>,

    /// Display name for the Matrix bot. Default: "IronClaw".
    #[serde(default)]
    pub matrix_display_name: Option<String>,

    /// Per-channel owner user IDs. When set, the channel only responds to this user.
    /// Key: channel name (e.g., "telegram"), Value: owner user ID.
    #[serde(default)]
    pub wasm_channel_owner_ids: std::collections::HashMap<String, i64>,

    /// Enabled WASM channels by name.
    /// Primarily used by the setup wizard to track which channels were configured.
    ///
    /// Startup treats this as a fallback restore source only until
    /// `activated_channels` has been persisted by the runtime.
    #[serde(default)]
    pub wasm_channels: Vec<String>,

    /// Whether WASM channels are enabled.
    #[serde(default = "default_true")]
    pub wasm_channels_enabled: bool,

    /// Directory containing WASM channel modules.
    #[serde(default)]
    pub wasm_channels_dir: Option<PathBuf>,

    /// CLI mode: "tui" for rich terminal UI, empty/absent for simple REPL.
    #[serde(default)]
    pub cli_mode: Option<String>,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            http_enabled: false,
            http_port: None,
            http_host: None,
            gateway_enabled: true,
            gateway_host: None,
            gateway_port: None,
            gateway_auth_token: None,
            cli_enabled: true,
            signal_enabled: false,
            signal_http_url: None,
            signal_account: None,
            signal_allow_from: None,
            signal_allow_from_groups: None,
            signal_dm_policy: None,
            signal_group_policy: None,
            signal_group_allow_from: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_access_token: None,
            matrix_allow_from: None,
            matrix_dm_policy: None,
            matrix_owner_id: None,
            matrix_poll_interval_secs: None,
            matrix_auto_join: None,
            matrix_display_name: None,
            wasm_channel_owner_ids: std::collections::HashMap::new(),
            wasm_channels: Vec::new(),
            wasm_channels_enabled: true,
            wasm_channels_dir: None,
            cli_mode: Some("tui".to_string()),
        }
    }
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

    /// Fixed time-of-day to fire (HH:MM, 24h). When set, interval_secs is ignored.
    #[serde(default)]
    pub fire_at: Option<String>,

    /// Hour (0-23) when quiet hours start (heartbeat skipped).
    #[serde(default)]
    pub quiet_hours_start: Option<u32>,

    /// Hour (0-23) when quiet hours end (heartbeat resumes).
    #[serde(default)]
    pub quiet_hours_end: Option<u32>,

    /// Timezone for fire_at and quiet hours (IANA name, e.g. "Pacific/Auckland").
    #[serde(default)]
    pub timezone: Option<String>,
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
            fire_at: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
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

    /// Maximum tool-call iterations per agentic loop invocation (default: 50).
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,

    /// When true, skip tool approval checks entirely. For benchmarks/CI.
    #[serde(default)]
    pub auto_approve_tools: bool,

    /// Default timezone for new sessions (IANA name, e.g. "America/New_York").
    #[serde(default = "default_timezone")]
    pub default_timezone: String,

    /// Maximum tokens per job (0 = unlimited).
    #[serde(default)]
    pub max_tokens_per_job: u64,
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

fn default_max_tool_iterations() -> usize {
    50
}

fn default_timezone() -> String {
    "UTC".to_string()
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
            max_tool_iterations: default_max_tool_iterations(),
            auto_approve_tools: false,
            default_timezone: default_timezone(),
            max_tokens_per_job: 0,
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
    500_000_000
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

    /// Whether Claude Code sandbox mode is enabled.
    #[serde(default)]
    pub claude_code_enabled: bool,

    /// Whether ACP (Agent Client Protocol) agent mode is enabled.
    #[serde(default)]
    pub acp_enabled: bool,
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
            claude_code_enabled: false,
            acp_enabled: false,
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

/// Routine scheduling and execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineSettings {
    /// Whether the routines system is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// How often (seconds) to poll for cron routines that need firing.
    #[serde(default = "default_routine_cron_interval")]
    pub cron_check_interval_secs: u64,

    /// Max routines executing concurrently.
    #[serde(default = "default_routine_max_concurrent")]
    pub max_concurrent_routines: usize,

    /// Default cooldown between fires (seconds).
    #[serde(default = "default_routine_cooldown")]
    pub default_cooldown_secs: u64,

    /// Max output tokens for lightweight routine LLM calls.
    #[serde(default = "default_routine_max_tokens")]
    pub max_lightweight_tokens: u32,

    /// Enable tool execution in lightweight routines.
    #[serde(default = "default_true")]
    pub lightweight_tools_enabled: bool,

    /// Max tool iterations for lightweight routines.
    #[serde(default = "default_routine_max_iterations")]
    pub lightweight_max_iterations: u32,
}

fn default_routine_cron_interval() -> u64 {
    15
}

fn default_routine_max_concurrent() -> usize {
    10
}

fn default_routine_cooldown() -> u64 {
    300
}

fn default_routine_max_tokens() -> u32 {
    4096
}

fn default_routine_max_iterations() -> u32 {
    3
}

impl Default for RoutineSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            cron_check_interval_secs: default_routine_cron_interval(),
            max_concurrent_routines: default_routine_max_concurrent(),
            default_cooldown_secs: default_routine_cooldown(),
            max_lightweight_tokens: default_routine_max_tokens(),
            lightweight_tools_enabled: true,
            lightweight_max_iterations: default_routine_max_iterations(),
        }
    }
}

/// Skills system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSettings {
    /// Whether the skills system is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Maximum number of skills that can be active simultaneously.
    #[serde(default = "default_skills_max_active")]
    pub max_active_skills: usize,

    /// Maximum total context tokens allocated to skill prompts.
    #[serde(default = "default_skills_max_context_tokens")]
    pub max_context_tokens: usize,
}

fn default_skills_max_active() -> usize {
    3
}

fn default_skills_max_context_tokens() -> usize {
    4000
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_active_skills: default_skills_max_active(),
            max_context_tokens: default_skills_max_context_tokens(),
        }
    }
}

/// Memory hygiene configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HygieneSettings {
    /// Whether hygiene is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Deprecated: retention is now per-folder via `.config` metadata.
    /// Kept for backward compatibility with existing DB settings rows.
    #[serde(default = "default_hygiene_daily_retention")]
    pub daily_retention_days: u32,

    /// Deprecated: retention is now per-folder via `.config` metadata.
    /// Kept for backward compatibility with existing DB settings rows.
    #[serde(default = "default_hygiene_conversation_retention")]
    pub conversation_retention_days: u32,

    /// Maximum versions to keep per document during hygiene passes.
    #[serde(default = "default_hygiene_version_keep_count")]
    pub version_keep_count: u32,

    /// Minimum hours between hygiene passes.
    #[serde(default = "default_hygiene_cadence_hours")]
    pub cadence_hours: u32,
}

fn default_hygiene_daily_retention() -> u32 {
    30
}

fn default_hygiene_conversation_retention() -> u32 {
    7
}

fn default_hygiene_version_keep_count() -> u32 {
    50
}

fn default_hygiene_cadence_hours() -> u32 {
    12
}

impl Default for HygieneSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_retention_days: default_hygiene_daily_retention(),
            conversation_retention_days: default_hygiene_conversation_retention(),
            version_keep_count: default_hygiene_version_keep_count(),
            cadence_hours: default_hygiene_cadence_hours(),
        }
    }
}

/// Workspace search fusion configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSettings {
    /// Fusion strategy: "rrf" or "weighted".
    #[serde(default = "default_search_fusion_strategy")]
    pub fusion_strategy: String,

    /// RRF constant k.
    #[serde(default = "default_search_rrf_k")]
    pub rrf_k: u32,

    /// FTS weight for fusion. `None` = use per-strategy default.
    #[serde(default)]
    pub fts_weight: Option<f32>,

    /// Vector weight for fusion. `None` = use per-strategy default.
    #[serde(default)]
    pub vector_weight: Option<f32>,

    /// Whether reasoning-augmented recall is enabled for memory search.
    /// `None` = use env/default (false).
    #[serde(default)]
    pub reasoning_enabled: Option<bool>,
}

fn default_search_fusion_strategy() -> String {
    "rrf".to_string()
}

fn default_search_rrf_k() -> u32 {
    60
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            fusion_strategy: default_search_fusion_strategy(),
            rrf_k: default_search_rrf_k(),
            fts_weight: None,
            vector_weight: None,
            reasoning_enabled: None,
        }
    }
}

/// Mission-related settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionSettings {
    /// Conversation insights extraction interval (every N completed threads).
    /// `None` = use env/default (5). Minimum: 1.
    #[serde(default)]
    pub insights_interval: Option<u32>,
}

/// Transcription pipeline settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSettings {
    /// Whether audio transcription is enabled.
    #[serde(default)]
    pub enabled: bool,
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
            if key == "owner_id" {
                continue;
            }

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
        map.remove("owner_id");
        map
    }

    /// Get the default settings file path (~/.ironclaw/settings.json).
    pub fn default_path() -> std::path::PathBuf {
        ironclaw_base_dir().join("settings.json")
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
        ironclaw_base_dir().join("config.toml")
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
             # Priority: DB settings > env vars > this file > defaults.\n\
             # A DB value equal to the built-in default is treated as unset.\n\
             # Exceptions: bootstrap and security-sensitive fields are env-only.\n\
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
        let (final_key, parent_parts) =
            parts.split_last().ok_or_else(|| "Empty path".to_string())?;

        // Navigate to parent and set the final key
        let mut current = &mut json;
        for part in parent_parts {
            current = current
                .get_mut(*part)
                .ok_or_else(|| format!("Path not found: {}", path))?;
        }
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
    fn test_from_db_map_tool_permissions() {
        use crate::tools::permissions::PermissionState;
        let mut map = std::collections::HashMap::new();
        map.insert(
            "tool_permissions.http".to_string(),
            serde_json::Value::String("always_allow".to_string()),
        );
        map.insert(
            "tool_permissions.shell".to_string(),
            serde_json::Value::String("ask_each_time".to_string()),
        );
        let settings = Settings::from_db_map(&map);
        assert_eq!(
            settings.tool_permissions.get("http"),
            Some(&PermissionState::AlwaysAllow),
            "tool_permissions.http should be AlwaysAllow, got {:?}",
            settings.tool_permissions
        );
        assert_eq!(
            settings.tool_permissions.get("shell"),
            Some(&PermissionState::AskEachTime),
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
    fn test_wasm_channel_owner_ids_db_round_trip() {
        let mut settings = Settings::default();
        settings
            .channels
            .wasm_channel_owner_ids
            .insert("telegram".to_string(), 123456789);

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(
            restored.channels.wasm_channel_owner_ids.get("telegram"),
            Some(&123456789)
        );
    }

    #[test]
    fn test_wasm_channel_owner_ids_default_empty() {
        let settings = Settings::default();
        assert!(settings.channels.wasm_channel_owner_ids.is_empty());
    }

    #[test]
    fn test_wasm_channel_owner_ids_via_set() {
        let mut settings = Settings::default();
        settings
            .set("channels.wasm_channel_owner_ids.telegram", "987654321")
            .unwrap();
        assert_eq!(
            settings.channels.wasm_channel_owner_ids.get("telegram"),
            Some(&987654321)
        );
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

    /// Regression: /model writes a single key ("selected_model") to the DB via
    /// set_setting(). On restart, get_all_settings() returns ALL keys including
    /// wizard-written defaults. The single-key update must survive the full
    /// from_db_map() round trip.
    #[test]
    fn db_single_key_model_update_survives_roundtrip() {
        // Step 1: Wizard writes full settings to DB (including selected_model
        // from initial setup).
        let wizard_settings = Settings {
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("old-wizard-model".to_string()),
            ..Default::default()
        };
        let mut db: std::collections::HashMap<String, serde_json::Value> =
            wizard_settings.to_db_map();

        // Step 2: User runs /model new-model — persist_selected_model writes
        // a single key, overwriting the wizard value.
        db.insert(
            "selected_model".to_string(),
            serde_json::Value::String("new-model".to_string()),
        );

        // Step 3: On restart, from_db_map() rebuilds Settings from the full
        // DB map.
        let restored = Settings::from_db_map(&db);
        assert_eq!(
            restored.selected_model,
            Some("new-model".to_string()),
            "/model change must survive DB round trip"
        );
    }

    /// TOML is loaded as a base, then DB is merged on top (DB wins).
    /// When both agree, the result matches.
    #[test]
    fn toml_and_db_matching_model_preserved() {
        // from_db_with_toml: TOML base, then DB merged on top.
        let mut toml_base = Settings {
            selected_model: Some("new-model".to_string()),
            ..Default::default()
        };

        let db_overlay = Settings {
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("new-model".to_string()),
            ..Default::default()
        };

        toml_base.merge_from(&db_overlay);
        assert_eq!(
            toml_base.selected_model,
            Some("new-model".to_string()),
            "matching values: result should be the shared value"
        );
    }

    /// Regression: when TOML has a stale model but DB has been updated via
    /// /model command, DB must win. This matches from_db_with_toml where
    /// TOML is loaded first as base, then DB is merged on top.
    #[test]
    fn db_model_wins_over_stale_toml() {
        // TOML base with old model.
        let mut toml_base = Settings {
            selected_model: Some("old-model".to_string()),
            ..Default::default()
        };

        // DB has the new model from /model command.
        let db_overlay = Settings {
            selected_model: Some("new-model".to_string()),
            ..Default::default()
        };

        // from_db_with_toml: TOML first, then DB merged on top.
        toml_base.merge_from(&db_overlay);
        assert_eq!(
            toml_base.selected_model,
            Some("new-model".to_string()),
            "DB selected_model must win over stale TOML value"
        );
    }

    /// Regression test: /model command must persist selected_model to TOML config.
    /// Prior to the fix, `set_model()` only changed the in-memory provider and the
    /// choice was lost on restart.
    #[test]
    fn toml_selected_model_update_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        // Start with a config that has a different model.
        let settings = Settings {
            selected_model: Some("old-model".to_string()),
            ..Default::default()
        };
        settings.save_toml(&path).unwrap();

        // Simulate what persist_selected_model does: load, update, save.
        let mut loaded = Settings::load_toml(&path).unwrap().unwrap();
        loaded.selected_model = Some("new-model".to_string());
        loaded.save_toml(&path).unwrap();

        // Verify the change survived a reload.
        let reloaded = Settings::load_toml(&path).unwrap().unwrap();
        assert_eq!(reloaded.selected_model, Some("new-model".to_string()));
    }

    /// save_toml / load_toml round-trip for selected_model.
    #[test]
    fn toml_save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        assert!(Settings::load_toml(&path).unwrap().is_none());

        let settings = Settings {
            selected_model: Some("new-model".to_string()),
            ..Default::default()
        };
        settings.save_toml(&path).unwrap();

        let loaded = Settings::load_toml(&path).unwrap().unwrap();
        assert_eq!(loaded.selected_model, Some("new-model".to_string()));
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

    /// Simulates the wizard recovery scenario:
    ///
    /// 1. A prior partial run saved steps 1-4 to the DB
    /// 2. User re-runs the wizard, Step 1 sets a new database_url
    /// 3. Prior settings are loaded from the DB
    /// 4. Step 1's fresh choices must win over stale DB values
    ///
    /// This tests the ordering: load DB → merge_from(step1_overrides).
    #[test]
    fn wizard_recovery_step1_overrides_stale_db() {
        // Simulate prior partial run (steps 1-4 completed):
        let prior_run = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://old-host/ironclaw".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        // Save to DB and reload (simulates persistence round-trip)
        let db_map = prior_run.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Step 1 of the new wizard run: user enters a NEW database_url
        let step1_settings = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://new-host/ironclaw".to_string()),
            ..Settings::default()
        };

        // Wizard flow: load DB → merge_from(step1_overrides)
        let mut current = step1_settings.clone();
        // try_load_existing_settings: merge DB into current
        current.merge_from(&from_db);
        // Re-apply Step 1 choices on top
        current.merge_from(&step1_settings);

        // Step 1's fresh database_url wins over stale DB value
        assert_eq!(
            current.database_url,
            Some("postgres://new-host/ironclaw".to_string()),
            "Step 1 fresh choice must override stale DB value"
        );

        // Prior run's steps 2-4 settings are preserved
        assert_eq!(
            current.llm_backend,
            Some("anthropic".to_string()),
            "Prior run's LLM backend must be recovered"
        );
        assert_eq!(
            current.selected_model,
            Some("claude-sonnet-4-5".to_string()),
            "Prior run's model must be recovered"
        );
        assert!(
            current.embeddings.enabled,
            "Prior run's embeddings setting must be recovered"
        );
    }

    /// Verifies that persisting defaults doesn't clobber prior settings
    /// when the merge ordering is correct.
    #[test]
    fn wizard_recovery_defaults_dont_clobber_prior() {
        // Prior run saved non-default settings
        let prior_run = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 900,
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior_run.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // New wizard run: Step 1 only sets DB fields (rest is default)
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            ..Default::default()
        };

        // Correct merge ordering
        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // Prior settings preserved (Step 1 doesn't touch these)
        assert_eq!(current.llm_backend, Some("openai".to_string()));
        assert_eq!(current.selected_model, Some("gpt-4o".to_string()));
        assert!(current.heartbeat.enabled);
        assert_eq!(current.heartbeat.interval_secs, 900);

        // Step 1's choice applied
        assert_eq!(current.database_backend, Some("libsql".to_string()));
    }

    // === QA Plan P1 - 1.2: Config round-trip tests ===

    #[test]
    fn comprehensive_db_map_round_trip() {
        // Set a representative value in EVERY section and verify survival
        let settings = Settings {
            onboard_completed: true,
            database_backend: Some("libsql".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            openai_compatible_base_url: Some("http://vllm:8000/v1".to_string()),
            secrets_master_key_source: KeySource::Keychain,
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "nearai".to_string(),
                model: "text-embedding-3-large".to_string(),
            },
            tunnel: TunnelSettings {
                provider: Some("ngrok".to_string()),
                ngrok_token: Some("tok_xxx".to_string()),
                ..Default::default()
            },
            channels: ChannelSettings {
                http_enabled: true,
                http_port: Some(9090),
                wasm_channel_owner_ids: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("telegram".to_string(), 12345);
                    m
                },
                ..Default::default()
            },
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 900,
                ..Default::default()
            },
            agent: AgentSettings {
                name: "my-bot".to_string(),
                max_parallel_jobs: 10,
                ..Default::default()
            },
            search: SearchSettings {
                reasoning_enabled: Some(true),
                fts_weight: Some(0.4),
                vector_weight: Some(0.6),
                ..Default::default()
            },
            missions: MissionSettings {
                insights_interval: Some(7),
            },
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);

        assert!(restored.onboard_completed, "onboard_completed lost");
        assert_eq!(
            restored.database_backend,
            Some("libsql".to_string()),
            "database_backend lost"
        );
        assert_eq!(
            restored.database_url,
            Some("postgres://host/db".to_string()),
            "database_url lost"
        );
        assert_eq!(
            restored.llm_backend,
            Some("anthropic".to_string()),
            "llm_backend lost"
        );
        assert_eq!(
            restored.selected_model,
            Some("claude-sonnet-4-5".to_string()),
            "selected_model lost"
        );
        assert_eq!(
            restored.openai_compatible_base_url,
            Some("http://vllm:8000/v1".to_string()),
            "openai_compatible_base_url lost"
        );
        assert_eq!(
            restored.secrets_master_key_source,
            KeySource::Keychain,
            "key_source lost"
        );
        assert!(restored.embeddings.enabled, "embeddings.enabled lost");
        assert_eq!(
            restored.embeddings.provider, "nearai",
            "embeddings.provider lost"
        );
        assert_eq!(
            restored.embeddings.model, "text-embedding-3-large",
            "embeddings.model lost"
        );
        assert_eq!(
            restored.tunnel.provider,
            Some("ngrok".to_string()),
            "tunnel.provider lost"
        );
        assert!(restored.channels.http_enabled, "http_enabled lost");
        assert_eq!(restored.channels.http_port, Some(9090), "http_port lost");
        assert_eq!(
            restored.channels.wasm_channel_owner_ids.get("telegram"),
            Some(&12345),
            "wasm_channel_owner_ids lost"
        );
        assert!(restored.heartbeat.enabled, "heartbeat.enabled lost");
        assert_eq!(
            restored.heartbeat.interval_secs, 900,
            "heartbeat.interval_secs lost"
        );
        assert_eq!(restored.agent.name, "my-bot", "agent.name lost");
        assert_eq!(
            restored.agent.max_parallel_jobs, 10,
            "agent.max_parallel_jobs lost"
        );
        assert_eq!(
            restored.search.reasoning_enabled,
            Some(true),
            "search.reasoning_enabled lost"
        );
        assert_eq!(
            restored.search.fts_weight,
            Some(0.4),
            "search.fts_weight lost"
        );
        assert_eq!(
            restored.search.vector_weight,
            Some(0.6),
            "search.vector_weight lost"
        );
        assert_eq!(
            restored.missions.insights_interval,
            Some(7),
            "missions.insights_interval lost"
        );
    }

    #[test]
    fn toml_json_db_all_agree() {
        // A config that goes through all three formats should produce the same values
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("config.toml");
        let json_path = dir.path().join("settings.json");

        let original = Settings {
            llm_backend: Some("ollama".to_string()),
            selected_model: Some("llama3".to_string()),
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 600,
                ..Default::default()
            },
            agent: AgentSettings {
                name: "round-trip-bot".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        // TOML round-trip
        original.save_toml(&toml_path).unwrap();
        let from_toml = Settings::load_toml(&toml_path).unwrap().unwrap();

        // JSON round-trip
        let json = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(&json_path, &json).unwrap();
        let from_json = Settings::load_from(&json_path);

        // DB map round-trip
        let db_map = original.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // All three should agree on key values
        for (label, loaded) in [("TOML", &from_toml), ("JSON", &from_json), ("DB", &from_db)] {
            assert_eq!(
                loaded.llm_backend,
                Some("ollama".to_string()),
                "{label}: llm_backend"
            );
            assert_eq!(
                loaded.selected_model,
                Some("llama3".to_string()),
                "{label}: selected_model"
            );
            assert!(loaded.heartbeat.enabled, "{label}: heartbeat.enabled");
            assert_eq!(
                loaded.heartbeat.interval_secs, 600,
                "{label}: heartbeat.interval_secs"
            );
            assert_eq!(loaded.agent.name, "round-trip-bot", "{label}: agent.name");
        }
    }

    #[test]
    fn set_get_round_trip_all_documented_paths() {
        let mut settings = Settings::default();

        // Test set + get for each documented settings path
        let test_cases: Vec<(&str, &str)> = vec![
            ("agent.name", "test-agent"),
            ("agent.max_parallel_jobs", "8"),
            ("heartbeat.enabled", "true"),
            ("heartbeat.interval_secs", "300"),
            ("channels.http_enabled", "true"),
            ("channels.http_port", "8081"),
        ];

        for (path, value) in &test_cases {
            settings
                .set(path, value)
                .unwrap_or_else(|e| panic!("set({path}, {value}) failed: {e}"));
            let got = settings
                .get(path)
                .unwrap_or_else(|| panic!("get({path}) returned None after set"));
            assert_eq!(&got, value, "set/get round-trip failed for path '{path}'");
        }
    }

    #[test]
    fn option_string_fields_survive_db_round_trip_as_null() {
        // When an Option<String> field is None, it should be stored as null
        // and come back as None, not silently become Some("")
        let settings = Settings {
            database_url: None,
            llm_backend: None,
            selected_model: None,
            openai_compatible_base_url: None,
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);

        assert_eq!(
            restored.database_url, None,
            "None database_url should stay None"
        );
        assert_eq!(
            restored.llm_backend, None,
            "None llm_backend should stay None"
        );
        assert_eq!(
            restored.selected_model, None,
            "None selected_model should stay None"
        );
    }

    // === Wizard re-run regression tests ===
    //
    // These tests simulate the merge ordering used by the wizard's `run()` method
    // to verify that re-running the wizard (or a subset of steps) doesn't
    // accidentally reset settings from prior runs.

    /// Simulates `ironclaw onboard --provider-only` re-running on a fully
    /// configured installation. Only provider + model should change; all
    /// other settings (channels, embeddings, heartbeat) must survive.
    #[test]
    fn provider_only_rerun_preserves_unrelated_settings() {
        // Prior completed run with everything configured
        let prior = Settings {
            onboard_completed: true,
            database_backend: Some("libsql".to_string()),
            libsql_path: Some("/home/user/.ironclaw/ironclaw.db".to_string()),
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
            },
            channels: ChannelSettings {
                http_enabled: true,
                http_port: Some(8080),
                signal_enabled: true,
                signal_account: Some("+1234567890".to_string()),
                wasm_channels: vec!["telegram".to_string()],
                ..Default::default()
            },
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 900,
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior.to_db_map();

        // provider_only mode: reconnect_existing_db loads from DB,
        // then user picks a new provider + model via step_inference_provider
        let mut current = Settings::from_db_map(&db_map);

        // Simulate step_inference_provider: user switches to anthropic
        current.llm_backend = Some("anthropic".to_string());
        current.selected_model = None; // cleared because backend changed

        // Simulate step_model_selection: user picks a model
        current.selected_model = Some("claude-sonnet-4-5".to_string());

        // Verify: provider/model changed
        assert_eq!(current.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(current.selected_model.as_deref(), Some("claude-sonnet-4-5"));

        // Verify: everything else preserved
        assert!(current.channels.http_enabled, "HTTP channel must survive");
        assert_eq!(current.channels.http_port, Some(8080));
        assert!(current.channels.signal_enabled, "Signal must survive");
        assert_eq!(
            current.channels.wasm_channels,
            vec!["telegram".to_string()],
            "WASM channels must survive"
        );
        assert!(current.embeddings.enabled, "Embeddings must survive");
        assert_eq!(current.embeddings.provider, "openai");
        assert!(current.heartbeat.enabled, "Heartbeat must survive");
        assert_eq!(current.heartbeat.interval_secs, 900);
        assert_eq!(
            current.database_backend.as_deref(),
            Some("libsql"),
            "DB backend must survive"
        );
    }

    /// Simulates `ironclaw onboard --channels-only` re-running on a fully
    /// configured installation. Only channel settings should change;
    /// provider, model, embeddings, heartbeat must survive.
    #[test]
    fn channels_only_rerun_preserves_unrelated_settings() {
        let prior = Settings {
            onboard_completed: true,
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "nearai".to_string(),
                model: "text-embedding-3-small".to_string(),
            },
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 1800,
                ..Default::default()
            },
            channels: ChannelSettings {
                http_enabled: false,
                wasm_channels: vec!["telegram".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior.to_db_map();

        // channels_only mode: reconnect_existing_db loads from DB
        let mut current = Settings::from_db_map(&db_map);

        // Simulate step_channels: user enables HTTP and adds discord
        current.channels.http_enabled = true;
        current.channels.http_port = Some(9090);
        current.channels.wasm_channels = vec!["telegram".to_string(), "discord".to_string()];

        // Verify: channels changed
        assert!(current.channels.http_enabled);
        assert_eq!(current.channels.http_port, Some(9090));
        assert_eq!(current.channels.wasm_channels.len(), 2);

        // Verify: everything else preserved
        assert_eq!(current.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(current.selected_model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(current.embeddings.enabled);
        assert_eq!(current.embeddings.provider, "nearai");
        assert!(current.heartbeat.enabled);
        assert_eq!(current.heartbeat.interval_secs, 1800);
    }

    /// Simulates quick mode re-run on an installation that previously
    /// completed a full setup. Quick mode only touches DB + security +
    /// provider + model; channels, embeddings, heartbeat, extensions
    /// should survive via the merge_from ordering.
    #[test]
    fn quick_mode_rerun_preserves_prior_channels_and_heartbeat() {
        let prior = Settings {
            onboard_completed: true,
            database_backend: Some("libsql".to_string()),
            libsql_path: Some("/home/user/.ironclaw/ironclaw.db".to_string()),
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            channels: ChannelSettings {
                http_enabled: true,
                http_port: Some(8080),
                signal_enabled: true,
                wasm_channels: vec!["telegram".to_string()],
                ..Default::default()
            },
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
            },
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 600,
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Quick mode flow:
        // 1. auto_setup_database sets DB fields
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            libsql_path: Some("/home/user/.ironclaw/ironclaw.db".to_string()),
            ..Default::default()
        };

        // 2. try_load_existing_settings → merge DB → merge step1 on top
        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // 3. step_inference_provider: user picks anthropic this time
        current.llm_backend = Some("anthropic".to_string());
        current.selected_model = None; // cleared because backend changed

        // 4. step_model_selection: user picks model
        current.selected_model = Some("claude-opus-4-6".to_string());

        // Verify: provider/model updated
        assert_eq!(current.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(current.selected_model.as_deref(), Some("claude-opus-4-6"));

        // Verify: channels, embeddings, heartbeat survived quick mode
        assert!(
            current.channels.http_enabled,
            "HTTP channel must survive quick mode re-run"
        );
        assert_eq!(current.channels.http_port, Some(8080));
        assert!(
            current.channels.signal_enabled,
            "Signal must survive quick mode re-run"
        );
        assert_eq!(
            current.channels.wasm_channels,
            vec!["telegram".to_string()],
            "WASM channels must survive quick mode re-run"
        );
        assert!(
            current.embeddings.enabled,
            "Embeddings must survive quick mode re-run"
        );
        assert!(
            current.heartbeat.enabled,
            "Heartbeat must survive quick mode re-run"
        );
        assert_eq!(current.heartbeat.interval_secs, 600);
    }

    /// Full wizard re-run where user keeps the same provider. The model
    /// selection from the prior run should be pre-populated (not reset).
    ///
    /// Regression: re-running with the same provider should preserve model.
    #[test]
    fn full_rerun_same_provider_preserves_model_through_merge() {
        let prior = Settings {
            onboard_completed: true,
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Step 1: user keeps same DB
        let step1 = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            ..Default::default()
        };

        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // After merge, prior settings recovered
        assert_eq!(
            current.llm_backend.as_deref(),
            Some("anthropic"),
            "Prior provider must be recovered from DB"
        );
        assert_eq!(
            current.selected_model.as_deref(),
            Some("claude-sonnet-4-5"),
            "Prior model must be recovered from DB"
        );

        // Step 3: user picks same provider (anthropic)
        // set_llm_backend_preserving_model checks if backend changed
        let backend_changed = current.llm_backend.as_deref() != Some("anthropic");
        current.llm_backend = Some("anthropic".to_string());
        if backend_changed {
            current.selected_model = None;
        }

        // Model should NOT be cleared since backend didn't change
        assert_eq!(
            current.selected_model.as_deref(),
            Some("claude-sonnet-4-5"),
            "Model must survive when re-selecting same provider"
        );
    }

    /// Full wizard re-run where user switches provider. Model should be
    /// cleared since the old model is invalid for the new backend.
    #[test]
    fn full_rerun_different_provider_clears_model_through_merge() {
        let prior = Settings {
            onboard_completed: true,
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Step 1 merge
        let step1 = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            ..Default::default()
        };
        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // Step 3: user switches to openai
        let backend_changed = current.llm_backend.as_deref() != Some("openai");
        assert!(backend_changed, "switching providers should be detected");
        current.llm_backend = Some("openai".to_string());
        if backend_changed {
            current.selected_model = None;
        }

        assert_eq!(current.llm_backend.as_deref(), Some("openai"));
        assert!(
            current.selected_model.is_none(),
            "Model must be cleared when switching providers"
        );
    }

    /// Simulates incremental save correctness: persist_after_step after
    /// Step 3 (provider) should not clobber settings set in Step 2 (security).
    ///
    /// The wizard persists the full settings object after each step. This
    /// test verifies that incremental saves are idempotent for prior steps.
    #[test]
    fn incremental_persist_does_not_clobber_prior_steps() {
        // After steps 1-2, settings has DB + security
        let after_step2 = Settings {
            database_backend: Some("libsql".to_string()),
            secrets_master_key_source: KeySource::Keychain,
            ..Default::default()
        };

        // persist_after_step saves to DB
        let db_map_after_step2 = after_step2.to_db_map();

        // Step 3 adds provider
        let mut after_step3 = after_step2.clone();
        after_step3.llm_backend = Some("openai".to_string());

        // persist_after_step saves again — the full settings object
        let db_map_after_step3 = after_step3.to_db_map();

        // Reload from DB after step 3
        let restored = Settings::from_db_map(&db_map_after_step3);

        // Step 2's settings must survive step 3's persist
        assert_eq!(
            restored.secrets_master_key_source,
            KeySource::Keychain,
            "Step 2 security setting must survive step 3 persist"
        );
        assert_eq!(
            restored.database_backend.as_deref(),
            Some("libsql"),
            "Step 1 DB setting must survive step 3 persist"
        );
        assert_eq!(
            restored.llm_backend.as_deref(),
            Some("openai"),
            "Step 3 provider setting must be saved"
        );

        // Also verify that a partial step 2 reload doesn't regress
        // (loading the step 2 snapshot and merging with step 3 state)
        let from_step2_db = Settings::from_db_map(&db_map_after_step2);
        let mut merged = after_step3.clone();
        merged.merge_from(&from_step2_db);

        assert_eq!(
            merged.llm_backend.as_deref(),
            Some("openai"),
            "Step 3 provider must not be clobbered by step 2 snapshot merge"
        );
        assert_eq!(
            merged.secrets_master_key_source,
            KeySource::Keychain,
            "Step 2 security must survive merge"
        );
    }

    /// Switching database backend should allow fresh connection settings.
    /// When user switches from postgres to libsql, the old database_url
    /// should not prevent the new libsql_path from being used.
    #[test]
    fn switching_db_backend_allows_fresh_connection_settings() {
        let prior = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://host/db".to_string()),
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // User picks libsql this time, wizard clears stale postgres settings
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            libsql_path: Some("/home/user/.ironclaw/ironclaw.db".to_string()),
            database_url: None, // explicitly not set for libsql
            ..Default::default()
        };

        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // libsql chosen
        assert_eq!(current.database_backend.as_deref(), Some("libsql"));
        assert_eq!(
            current.libsql_path.as_deref(),
            Some("/home/user/.ironclaw/ironclaw.db")
        );

        // Prior provider/model should survive (unrelated to DB switch)
        assert_eq!(current.llm_backend.as_deref(), Some("openai"));
        assert_eq!(current.selected_model.as_deref(), Some("gpt-4o"));

        // Note: database_url from prior run persists in merge because
        // step1.database_url is None (== default), so merge_from doesn't
        // override it. This is expected — the .env writer decides which
        // vars to emit based on database_backend. The stale URL is
        // harmless because the libsql backend ignores it.
        assert_eq!(
            current.database_url.as_deref(),
            Some("postgres://host/db"),
            "stale database_url persists (harmless, ignored by libsql backend)"
        );
    }

    /// Regression: merge_from must handle boolean fields correctly.
    /// A prior run with heartbeat.enabled=true must not be reset to false
    /// when merging with a Settings that has heartbeat.enabled=false (default).
    #[test]
    fn merge_preserves_true_booleans_when_overlay_has_default_false() {
        let prior = Settings {
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 600,
                ..Default::default()
            },
            channels: ChannelSettings {
                http_enabled: true,
                signal_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // New wizard run only sets DB (everything else is default/false)
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            ..Default::default()
        };

        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // true booleans from prior run must survive
        assert!(
            current.heartbeat.enabled,
            "heartbeat.enabled=true must not be reset to false by default overlay"
        );
        assert!(
            current.channels.http_enabled,
            "http_enabled=true must not be reset to false by default overlay"
        );
        assert!(
            current.channels.signal_enabled,
            "signal_enabled=true must not be reset to false by default overlay"
        );
        assert_eq!(current.heartbeat.interval_secs, 600);
    }

    /// Regression: embeddings settings (provider, model, enabled) must
    /// survive a wizard re-run that doesn't touch step 5.
    #[test]
    fn embeddings_survive_rerun_that_skips_step5() {
        let prior = Settings {
            onboard_completed: true,
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("qwen".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "nearai".to_string(),
                model: "text-embedding-3-large".to_string(),
            },
            ..Default::default()
        };
        let db_map = prior.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Full re-run: step 1 only sets DB
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            ..Default::default()
        };
        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // Before step 5 (embeddings) runs, check that prior values are present
        assert!(current.embeddings.enabled);
        assert_eq!(current.embeddings.provider, "nearai");
        assert_eq!(current.embeddings.model, "text-embedding-3-large");
    }

    /// DB values must win over TOML values when both set the same field.
    ///
    /// This mirrors the merge order in `Config::from_db_with_toml`:
    /// TOML is loaded as the base, then DB is merged on top.
    #[test]
    fn db_settings_win_over_toml_settings() {
        // Simulate TOML base: has llm_backend and selected_model
        let mut base = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("toml-model".to_string()),
            ..Default::default()
        };

        // Simulate DB overlay: has different llm_backend and selected_model
        let db = Settings {
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("db-model".to_string()),
            ..Default::default()
        };

        // Merge DB on top of TOML (same order as from_db_with_toml)
        base.merge_from(&db);

        assert_eq!(
            base.llm_backend.as_deref(),
            Some("anthropic"),
            "DB llm_backend must win over TOML"
        );
        assert_eq!(
            base.selected_model.as_deref(),
            Some("db-model"),
            "DB selected_model must win over TOML"
        );
    }

    /// When DB has no value (default), TOML value should be preserved.
    #[test]
    fn toml_settings_used_when_db_has_no_value() {
        let mut base = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("toml-model".to_string()),
            ..Default::default()
        };

        // DB has no llm_backend or selected_model (both default/None)
        let db = Settings::default();

        base.merge_from(&db);

        assert_eq!(
            base.llm_backend.as_deref(),
            Some("openai"),
            "TOML llm_backend should be preserved when DB has no value"
        );
        assert_eq!(
            base.selected_model.as_deref(),
            Some("toml-model"),
            "TOML selected_model should be preserved when DB has no value"
        );
    }
}
