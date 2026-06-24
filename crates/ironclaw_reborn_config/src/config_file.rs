//! Boot-time TOML config for the standalone Reborn binary.
//!
//! Operator-facing file at `$IRONCLAW_REBORN_HOME/config.toml`. Read once
//! at process start by `ironclaw-reborn run`. Provides the *selection*
//! layer of the three-layer config model:
//!
//! - **Catalog**: `providers.json` (this crate exposes the path; the
//!   composition root loads the file via `ironclaw_llm::ProviderRegistry`).
//! - **Selection**: this file. "Use provider X for the `default` LLM
//!   slot, with model Y."
//! - **Runtime config**: derived in the composition root by resolving
//!   the selection against the catalog.
//!
//! Precedence on each individual field:
//!
//! ```text
//! compiled defaults  <  this file  <  env vars  <  CLI flags
//! ```
//!
//! Secrets are env-only by policy. Pasting raw secret-shaped values
//! into this file is rejected at parse time via [`secrets_guard`].
//!
//! Layering note: this crate must stay free of IronClaw workspace
//! dependencies (the boundary test
//! `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`
//! pins this). So we parse into **plain strings** for fields whose
//! typed counterparts live in `ironclaw_host_api` (TenantId, AgentId,
//! UserId, ProjectId, DeploymentMode, RuntimeProfile, ApprovalPolicy) or
//! `ironclaw_reborn_composition` (RebornDriverChoice, RebornHarnessId).
//! The composition root validates/promotes the strings into the typed
//! shapes — that's where validation belongs anyway. This crate only
//! enforces shape (sections exist, fields are the right TOML type,
//! no inline secrets).

use std::borrow::Cow;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use crate::secrets_guard::{InlineSecretError, reject_inline_secret};

/// API version stamp this crate understands. Mirrors
/// `ironclaw_reborn_composition::RebornRuntimeApiVersion::V1`. A future
/// major bump fails parse closed; minor bumps are accepted.
pub const REBORN_CONFIG_API_VERSION: &str = "ironclaw.runtime/v1";

/// Full parsed config file.
///
/// Every section is optional so an operator can ship a sparse file that
/// overrides only the fields they care about; the rest stays at the
/// CLI-shaped defaults baked into composition.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RebornConfigFile {
    /// API version. When set, must be parseable as `ironclaw.runtime/vN.M`
    /// with matching major. When omitted, parser assumes the file targets
    /// the current major.
    pub api_version: Option<String>,
    pub boot: Option<BootSection>,
    pub identity: Option<IdentitySection>,
    pub policy: Option<PolicySection>,
    pub drivers: Option<DriversSection>,
    pub harness: Option<HarnessSection>,
    pub runner: Option<RunnerSection>,
    /// Skill activation selection settings for local-dev runtime skill context.
    pub skills: Option<SkillsSection>,
    /// Durable storage selection for production Reborn boot.
    ///
    /// Credential-bearing database URLs must stay env-only. This section names
    /// the backend and the environment variable that contains the URL.
    pub storage: Option<StorageSection>,
    /// Per-slot LLM selection. Keyed by Reborn model slot name. Today
    /// composition wires only the `default` slot; the `mission` slot
    /// becomes live when the planned driver lands. Operators are free
    /// to populate `mission` ahead of time.
    pub llm: Option<std::collections::BTreeMap<String, LlmSlotSelection>>,
    /// WebChat v2 HTTP gateway settings. Consumed by
    /// `ironclaw_reborn_webui_ingress` when the standalone CLI's
    /// `serve` subcommand is invoked. Optional — sparse configs
    /// fall back to compiled defaults documented on each field.
    pub webui: Option<WebuiSection>,
    /// Slack Events API host-beta route settings. Consumed by
    /// `ironclaw-reborn serve` only when the binary is built with the
    /// Slack host-beta feature. Secrets are env-only; this section stores
    /// IDs and environment variable names.
    pub slack: Option<SlackSection>,
    /// Cost-based budgets. Composition seeds defaults on first reservation
    /// for each user/project; per-account overrides happen through the
    /// `budget_set` tool or CLI at runtime. Setting any limit to `0` means
    /// "unlimited" for that dimension.
    pub budget: Option<BudgetSection>,
    /// Trigger poller lifecycle settings. All fields optional; absent section
    /// leaves the worker at the compiled defaults in the composition root.
    pub trigger_poller: Option<TriggerPollerConfigSection>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootSection {
    /// Composition profile name. Stringly typed; composition validates
    /// against `RebornCompositionProfile`. Examples: `"local-dev"`,
    /// `"local-dev-yolo"`, `"hosted-single-tenant"`, `"production"`,
    /// `"migration-dry-run"`.
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySection {
    pub tenant: Option<String>,
    pub default_agent: Option<String>,
    pub default_owner: Option<String>,
    pub default_project: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySection {
    /// One of `local_single_user`, `hosted_multi_tenant`,
    /// `enterprise_dedicated`. Composition matches against
    /// `ironclaw_host_api::runtime_policy::DeploymentMode`.
    pub deployment_mode: Option<String>,
    /// `RuntimeProfile` variant in snake_case.
    pub default_profile: Option<String>,
    /// One of `ask_always`, `ask_writes`, `ask_destructive`, `org_policy`,
    /// `minimal`. Composition matches against `ApprovalPolicy`.
    pub default_approval_policy: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriversSection {
    /// Default driver name. Composition matches against
    /// `RebornDriverChoice`: `"text_only"`, `"planned"`.
    pub default: Option<String>,
    /// Additional drivers to register so per-turn
    /// `requested_run_profile` can pick them.
    pub additional: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessSection {
    /// Active harness id. Composition logs the value at boot; takes
    /// effect when the harness substrate from epic #3036 lands.
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerSection {
    pub heartbeat_interval_secs: Option<u64>,
    pub poll_interval_ms: Option<u64>,
    /// Number of concurrent turn-runner worker tasks. `None` or `0` defaults to 4. Clamped to 32.
    pub worker_count: Option<usize>,
    /// Max concurrent runs in `TurnStatus::Running` per (tenant_id, owner user_id). `None` or `0` = unlimited.
    pub max_concurrent_runs_per_user: Option<u32>,
    /// Max concurrent runs in `TurnStatus::Running` for `ScheduledTrigger` origin. `None` or `0` = unlimited.
    pub max_concurrent_trigger_runs: Option<u32>,
    /// Max concurrent runs in `TurnStatus::Running` for `Inbound` or `WebUi` origin. `None` or `0` = unlimited.
    pub max_concurrent_conversation_runs: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsSection {
    /// When false, regex activation criteria no longer auto-load full skill context.
    /// Keyword/tag activation and explicit skill mentions still work.
    pub regex_activation_enabled: Option<bool>,
}

/// Durable storage backend names accepted by the Reborn production boot config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageBackend {
    Postgres,
    #[doc(hidden)]
    Unknown(String),
}

impl<'de> Deserialize<'de> for StorageBackend {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StorageBackendVisitor;

        impl Visitor<'_> for StorageBackendVisitor {
            type Value = StorageBackend;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a storage backend name such as `postgres`")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "postgres" => Ok(StorageBackend::Postgres),
                    candidate => Ok(StorageBackend::Unknown(candidate.to_string())),
                }
            }
        }

        deserializer.deserialize_str(StorageBackendVisitor)
    }
}

/// Durable storage selection for production Reborn boot.
///
/// `url_env` and `secret_master_key_env` are environment variable NAMES, not
/// credential-bearing values. The parser rejects raw URL-shaped values so
/// credentials cannot be pasted into `config.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSection {
    /// Storage backend name. First production slice supports `"postgres"`.
    pub backend: Option<StorageBackend>,
    /// Environment variable name containing the PostgreSQL connection URL.
    pub url_env: Option<String>,
    /// Environment variable name containing the Reborn secret master key.
    pub secret_master_key_env: Option<String>,
    /// PostgreSQL connection pool size for production storage. Defaults to 2.
    pub pool_max_size: Option<usize>,
}

/// WebChat v2 HTTP gateway configuration.
///
/// Composition reads this section when wiring the `serve` subcommand.
/// Stringly typed by design — the `ironclaw_reborn_config` crate stays
/// free of workspace deps, so concrete validation (origin parsing,
/// listen-address resolution) lives in the consuming ingress crate.
///
/// Secrets are env-only: `env_token_var` is the **NAME** of an
/// environment variable, never a token value. The `secrets_guard`
/// inline-secret check fires at parse time if an operator pastes a
/// token-shaped string into either field documented as a name.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebuiSection {
    /// IP address the WebChat v2 listener binds. Default `127.0.0.1`
    /// (loopback only — operators MUST opt in to `0.0.0.0` or a
    /// specific interface to expose the gateway).
    pub listen_host: Option<String>,
    /// TCP port the listener binds. Default `3000`. `0` is rejected
    /// at composition time (`ironclaw-reborn serve` accepts `0` only
    /// via an explicit `--port 0` CLI flag, intended for tests).
    pub listen_port: Option<u16>,
    /// Name of the environment variable holding the host-installation
    /// bearer token (used by the env-bearer authenticator). Default
    /// `IRONCLAW_REBORN_WEBUI_TOKEN`. The token VALUE never appears in
    /// this config file — `secrets_guard` rejects inline secrets.
    pub env_token_var: Option<String>,
    /// Name of the environment variable holding the `UserId` that an
    /// env-bearer-authenticated caller maps to. Default
    /// `IRONCLAW_REBORN_WEBUI_USER_ID`. Stringly typed; composition
    /// resolves to a real `UserId` and rejects malformed values.
    pub env_user_id_var: Option<String>,
    /// CORS allow-origin list (e.g.
    /// `["http://localhost:3000", "https://app.example.com"]`).
    /// Default empty — composition then fails-closed on every
    /// cross-origin preflight, never echoing an attacker-supplied
    /// `Origin` header. Operators MUST opt in to whichever origins
    /// the host installation actually serves.
    pub allowed_origins: Option<Vec<String>>,
    /// Override the default Content-Security-Policy header. Default
    /// `None` → composition applies its locked-down default
    /// (`default-src 'self'; object-src 'none'; frame-ancestors 'none';
    /// base-uri 'self'`). Operators serving a real SPA may need to
    /// override.
    pub csp_header_override: Option<String>,
    /// Maximum per-request body bytes for paths that do NOT match a
    /// declared v2 descriptor (i.e. the 404 fallback path). v2 routes
    /// are individually capped from their `BodyLimitPolicy`
    /// descriptor and are strictly tighter than this outer fallback.
    /// Default `14 * 1024 * 1024` (14 MiB). `0` is rejected.
    pub max_body_bytes_fallback: Option<u64>,
    /// Canonical host this listener is reachable on (e.g.
    /// `"app.example.com"` or `"127.0.0.1:3000"`). When set, the WS
    /// same-origin middleware compares the request `Origin` against
    /// this operator-trusted value instead of trusting the
    /// client-supplied `Host` header. Critical when running behind a
    /// reverse proxy that may forward an attacker-controlled Host —
    /// without `canonical_host`, a forged Host + matching Origin
    /// would pass `SameOriginRequired`. Format: `host` or
    /// `host:port`; composition does not parse further. Default
    /// `None` (fall back to Host-header compare + allowlist).
    pub canonical_host: Option<String>,
}

/// Slack Events API host-beta enablement.
///
/// `enabled = true` mounts the Slack route. Installation identifiers, channel
/// routing, and Slack secrets are configured through the WebUI channel setup
/// surface. The deprecated fields below are accepted as a startup migration
/// bridge for existing `config.toml` files; secret values still stay env-only.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlackSection {
    /// Explicit host-beta enablement gate. Omitted/false means the Slack route
    /// is not mounted by `ironclaw-reborn serve`.
    pub enabled: Option<bool>,
    /// Deprecated: adapter installation id for legacy config-backed setup.
    pub installation_id: Option<String>,
    /// Deprecated: Slack team id for legacy config-backed setup.
    pub team_id: Option<String>,
    /// Deprecated: Slack app id for legacy config-backed setup.
    pub api_app_id: Option<String>,
    /// Deprecated: optional Slack user id for legacy static personal binding.
    pub slack_user_id: Option<String>,
    /// Deprecated: Reborn user id for legacy Slack setup.
    pub user_id: Option<String>,
    /// Deprecated: Reborn user id whose scope owns shared Slack channel turns.
    pub shared_subject_user_id: Option<String>,
    /// Deprecated: channel-specific shared subjects for Slack app mentions.
    #[serde(default)]
    pub channel_routes: Vec<SlackChannelRouteSection>,
    /// Deprecated: environment variable name containing the Slack signing secret.
    pub signing_secret_env: Option<String>,
    /// Deprecated: environment variable name containing the Slack bot token.
    pub bot_token_env: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlackChannelRouteSection {
    pub channel_id: Option<String>,
    pub subject_user_id: Option<String>,
}

/// `[budget]` section. All limits in USD. **0 = unlimited.**
///
/// Composition uses these as defaults when first seeding a user/project
/// account. Runtime tools can install per-account overrides at any time.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSection {
    /// Per-user daily ceiling. Default in composition is `5.00`.
    pub user_daily_usd: Option<f64>,
    /// Per-project daily ceiling. Default in composition is `2.00`.
    pub project_daily_usd: Option<f64>,
    /// Per-tick budget for background missions. Default `0.50`.
    pub mission_per_tick_usd: Option<f64>,
    /// Per-tick budget for heartbeat ticks. Default `0.05`.
    pub heartbeat_per_tick_usd: Option<f64>,
    /// Per-fire budget for lightweight routines. Default `0.02`.
    pub routine_lightweight_usd: Option<f64>,
    /// Per-fire budget for standard routines. Default `0.10`.
    pub routine_standard_usd: Option<f64>,
    /// Default per-job budget for one-shot container jobs. Default `1.00`.
    pub background_job_default_usd: Option<f64>,
    /// IANA timezone for calendar-period rollover (e.g. `"UTC"`,
    /// `"America/Los_Angeles"`). Default `"UTC"`.
    pub default_tz: Option<String>,
    /// Warn threshold as a fraction in `[0.0, 1.0]`. Default `0.75`.
    pub warn_at: Option<f64>,
    /// Pause-with-approval threshold as a fraction in `[0.0, 1.0]`.
    /// Must be `>= warn_at`. Default `0.90`.
    pub pause_at: Option<f64>,
    /// Multiplier applied to upfront cost estimates before reserving.
    /// Default `1.20` (20% safety margin); reconcile releases the
    /// overshoot.
    pub overestimate_factor: Option<f64>,
}

/// `[trigger_poller]` section. Controls the background trigger-poller worker.
///
/// All fields are optional so a sparse or absent section is valid; the
/// composition root applies its own compiled defaults for any field not set
/// here. Env vars (`IRONCLAW_TRIGGER_POLLER_*`) override this section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerPollerConfigSection {
    /// Enable or disable the trigger poller. Default `false` (off) in
    /// composition; operators MUST set `enabled = true` to activate it.
    pub enabled: Option<bool>,
    /// How often the poller ticks, in seconds. Default in composition is 30.
    /// Range `1..=3600` is enforced at boot by the CLI settings layer;
    /// values outside the range are a fatal startup error.
    pub poll_interval_secs: Option<u64>,
    /// Maximum triggers to fire per tick. Default in composition is 32.
    /// Range `1..=1000` is enforced at boot by the CLI settings layer;
    /// values outside the range are a fatal startup error.
    pub fires_per_tick: Option<u32>,
    /// Maximum concurrent fires allowed for a single trigger. Default in
    /// composition is 1. V1 invariant: must equal 1, enforced at boot by
    /// the CLI settings layer; any other value is a fatal startup error.
    pub max_concurrent_fires_per_trigger: Option<u32>,
    /// Upper bound (seconds) of a random jitter delay before the first tick.
    /// Spreads startup load across instances. Default in composition is 0.
    /// Range `0..=3600` is enforced at boot by the CLI settings layer.
    pub startup_jitter_max_secs: Option<u64>,
    /// Upper bound (seconds) of a random jitter added to each tick interval.
    /// Prevents synchronized thundering-herd across instances. Default 0.
    /// Range `0..=3600` is enforced at boot by the CLI settings layer.
    pub tick_jitter_max_secs: Option<u64>,
}

/// One `[llm.<slot>]` entry. The slot name (typically `"default"` or
/// `"mission"`) is the TOML table key.
///
/// References a provider by `provider_id` (resolved against the merged
/// `ProviderRegistry` in the composition root) and optionally overrides
/// the provider's `default_model` and `api_key_env`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmSlotSelection {
    /// Provider id from `providers.json` (built-in or user catalog).
    pub provider_id: Option<String>,
    /// Override the provider's `default_model`. Optional.
    pub model: Option<String>,
    /// Override the provider's `api_key_env`. Optional. Per the secrets
    /// rule, this MUST be an env-var NAME (e.g. `"OPENAI_API_KEY"`), not
    /// the value itself — `secrets_guard::reject_inline_secret` enforces
    /// that during validation.
    pub api_key_env: Option<String>,
    /// Override the provider's `default_base_url`. Optional.
    pub base_url: Option<String>,
}

/// Field update for an existing LLM slot selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LlmSlotFieldUpdate {
    /// Preserve the field exactly as it appears in the current document.
    #[default]
    Keep,
    /// Set the field to a new string value.
    Set(String),
    /// Remove the field from the slot selection.
    Remove,
}

/// Typed patch for `[llm.default]` in the operator config file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultLlmSlotUpdate {
    pub provider_id: LlmSlotFieldUpdate,
    pub model: LlmSlotFieldUpdate,
    pub api_key_env: LlmSlotFieldUpdate,
    pub base_url: LlmSlotFieldUpdate,
}

/// Held exclusive lock plus editable config document for one config update.
pub struct DefaultLlmSlotUpdateSession {
    path: PathBuf,
    doc: toml_edit::DocumentMut,
    _lock_file: fs::File,
}

impl DefaultLlmSlotUpdateSession {
    pub fn default_llm_slot(
        &self,
    ) -> Result<Option<LlmSlotSelection>, RebornConfigFileUpdateError> {
        let Some(default_slot) = self
            .doc
            .get("llm")
            .and_then(|llm| llm.get("default"))
            .and_then(toml_edit::Item::as_table_like)
        else {
            return Ok(None);
        };

        Ok(Some(LlmSlotSelection {
            provider_id: default_slot
                .get("provider_id")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_string),
            model: default_slot
                .get("model")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_string),
            api_key_env: default_slot
                .get("api_key_env")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_string),
            base_url: default_slot
                .get("base_url")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_string),
        }))
    }

    pub fn apply(
        mut self,
        update: &DefaultLlmSlotUpdate,
    ) -> Result<(), RebornConfigFileUpdateError> {
        apply_llm_slot_field(&mut self.doc, "provider_id", &update.provider_id);
        apply_llm_slot_field(&mut self.doc, "model", &update.model);
        apply_llm_slot_field(&mut self.doc, "api_key_env", &update.api_key_env);
        apply_llm_slot_field(&mut self.doc, "base_url", &update.base_url);
        write_edit_document(&self.path, &self.doc)
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RebornConfigFileError {
    #[error("could not read config file `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse config file `{path}` as TOML: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "config file `{path}` declares api_version `{found}`, but this binary speaks `{expected}`; \
         major mismatch is fail-closed"
    )]
    IncompatibleApiVersion {
        path: String,
        found: String,
        expected: &'static str,
    },
    #[error("config file `{path}` field validation failed: {source}")]
    InlineSecret {
        path: String,
        #[source]
        source: InlineSecretError,
    },
    #[error("config file `{path}` field `{field}` validation failed: {reason}")]
    InvalidField {
        path: String,
        field: String,
        reason: String,
    },
    #[error("config file `{path}` api_version `{found}` could not be parsed: {reason}")]
    InvalidApiVersion {
        path: String,
        found: String,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum RebornConfigFileUpdateError {
    #[error("lock Reborn config `{}`: {source}", path.display())]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("read Reborn config `{}`: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse Reborn config `{}` as TOML: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml_edit::TomlError,
    },
    #[error("validate Reborn config `{}`: {source}", path.display())]
    Validate {
        path: PathBuf,
        source: Box<RebornConfigFileError>,
    },
    #[error("write Reborn config `{}`: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

// ─── Loader ─────────────────────────────────────────────────────────────────

impl RebornConfigFile {
    /// Read a config file from disk. Returns `Ok(None)` if the file
    /// does not exist (sparse configs are legitimate — operator boots
    /// with defaults), `Err` on any other I/O error or on a TOML parse
    /// failure / validation rejection.
    pub fn load(path: &Path) -> Result<Option<Self>, RebornConfigFileError> {
        match fs::read_to_string(path) {
            Ok(text) => {
                let parsed = Self::parse_text(&text, path)?;
                Ok(Some(parsed))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(RebornConfigFileError::Io {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    /// Parse + validate a TOML string. Public so callers can drive the
    /// parser without going through the filesystem (e.g. CLI flag
    /// `--config-string`, tests).
    pub fn parse_text(text: &str, attributed_path: &Path) -> Result<Self, RebornConfigFileError> {
        let parsed: Self = toml::from_str(text).map_err(|source| RebornConfigFileError::Toml {
            path: attributed_path.display().to_string(),
            source,
        })?;
        parsed.validate(attributed_path)?;
        Ok(parsed)
    }

    fn validate(&self, attributed_path: &Path) -> Result<(), RebornConfigFileError> {
        // Inline-secret check on every operator-supplied string before
        // any later validator can echo the value in a more specific error.
        let path_str = || attributed_path.display().to_string();
        let check = |label: Cow<'static, str>, value: &str| -> Result<(), RebornConfigFileError> {
            reject_inline_secret(label, value).map_err(|source| {
                RebornConfigFileError::InlineSecret {
                    path: path_str(),
                    source,
                }
            })
        };
        let check_non_empty_trimmed =
            |label: Cow<'static, str>, value: &str| -> Result<(), RebornConfigFileError> {
                check(label.clone(), value)?;
                if value.trim().is_empty() {
                    return Err(RebornConfigFileError::InvalidField {
                        path: path_str(),
                        field: label.into_owned(),
                        reason: "must not be empty".to_string(),
                    });
                }
                if value.trim() != value {
                    return Err(RebornConfigFileError::InvalidField {
                        path: path_str(),
                        field: label.into_owned(),
                        reason: "must not contain leading or trailing whitespace".to_string(),
                    });
                }
                Ok(())
            };

        if let Some(api_version) = self.api_version.as_deref() {
            check(Cow::Borrowed("api_version"), api_version)?;
            validate_api_version(api_version, attributed_path)?;
        }
        if let Some(boot) = &self.boot
            && let Some(profile) = &boot.profile
        {
            check(Cow::Borrowed("boot.profile"), profile)?;
        }
        if let Some(identity) = &self.identity {
            if let Some(tenant) = &identity.tenant {
                check(Cow::Borrowed("identity.tenant"), tenant)?;
            }
            if let Some(default_agent) = &identity.default_agent {
                check(Cow::Borrowed("identity.default_agent"), default_agent)?;
            }
            if let Some(default_owner) = &identity.default_owner {
                check(Cow::Borrowed("identity.default_owner"), default_owner)?;
            }
            if let Some(default_project) = &identity.default_project {
                check(Cow::Borrowed("identity.default_project"), default_project)?;
            }
        }
        if let Some(policy) = &self.policy {
            if let Some(deployment_mode) = &policy.deployment_mode {
                check(Cow::Borrowed("policy.deployment_mode"), deployment_mode)?;
            }
            if let Some(default_profile) = &policy.default_profile {
                check(Cow::Borrowed("policy.default_profile"), default_profile)?;
            }
            if let Some(default_approval_policy) = &policy.default_approval_policy {
                check(
                    Cow::Borrowed("policy.default_approval_policy"),
                    default_approval_policy,
                )?;
            }
        }
        if let Some(drivers) = &self.drivers {
            if let Some(default) = &drivers.default {
                check(Cow::Borrowed("drivers.default"), default)?;
            }
            if let Some(additional) = &drivers.additional {
                for driver in additional {
                    check(Cow::Borrowed("drivers.additional"), driver)?;
                }
            }
        }
        if let Some(harness) = &self.harness
            && let Some(id) = &harness.id
        {
            check(Cow::Borrowed("harness.id"), id)?;
        }
        if let Some(llm) = &self.llm {
            for (slot, selection) in llm {
                check(Cow::Borrowed("llm.<slot>"), slot)?;
                if let Some(provider_id) = &selection.provider_id {
                    check(llm_slot_field_label(slot, "provider_id"), provider_id)?;
                }
                if let Some(api_key_env) = &selection.api_key_env {
                    check(llm_slot_field_label(slot, "api_key_env"), api_key_env)?;
                }
                if let Some(base_url) = &selection.base_url {
                    check(llm_slot_field_label(slot, "base_url"), base_url)?;
                }
                if let Some(model) = &selection.model {
                    check(llm_slot_field_label(slot, "model"), model)?;
                }
            }
        }
        if let Some(storage) = &self.storage {
            if let Some(StorageBackend::Unknown(backend)) = &storage.backend {
                check_non_empty_trimmed(Cow::Borrowed("storage.backend"), backend)?;
                let reason = if backend.contains("://") {
                    "must be a backend name, not a URL or inline secret value".to_string()
                } else {
                    format!("supports only \"postgres\" in this slice; got `{backend}`")
                };
                return Err(RebornConfigFileError::InvalidField {
                    path: attributed_path.display().to_string(),
                    field: "storage.backend".to_string(),
                    reason,
                });
            }
            if let Some(url_env) = &storage.url_env {
                check_non_empty_trimmed(Cow::Borrowed("storage.url_env"), url_env)?;
                validate_env_var_reference("storage.url_env", url_env, attributed_path)?;
            }
            if let Some(secret_master_key_env) = &storage.secret_master_key_env {
                check_non_empty_trimmed(
                    Cow::Borrowed("storage.secret_master_key_env"),
                    secret_master_key_env,
                )?;
                validate_env_var_reference(
                    "storage.secret_master_key_env",
                    secret_master_key_env,
                    attributed_path,
                )?;
            }
            if let Some(pool_max_size) = storage.pool_max_size
                && pool_max_size == 0
            {
                return Err(RebornConfigFileError::InvalidField {
                    path: attributed_path.display().to_string(),
                    field: "storage.pool_max_size".to_string(),
                    reason: "must be greater than 0".to_string(),
                });
            }
        }
        if let Some(webui) = &self.webui {
            if let Some(host) = &webui.listen_host {
                check(Cow::Borrowed("webui.listen_host"), host)?;
            }
            if let Some(env_token_var) = &webui.env_token_var {
                // Secrets guard: rejects token-shaped values pasted
                // here instead of an env-var name.
                check(Cow::Borrowed("webui.env_token_var"), env_token_var)?;
            }
            if let Some(env_user_id_var) = &webui.env_user_id_var {
                check(Cow::Borrowed("webui.env_user_id_var"), env_user_id_var)?;
            }
            if let Some(allowed_origins) = &webui.allowed_origins {
                for origin in allowed_origins {
                    check(Cow::Borrowed("webui.allowed_origins"), origin)?;
                }
            }
            if let Some(csp) = &webui.csp_header_override {
                check(Cow::Borrowed("webui.csp_header_override"), csp)?;
            }
            if let Some(host) = &webui.canonical_host {
                check(Cow::Borrowed("webui.canonical_host"), host)?;
            }
        }
        if let Some(slack) = &self.slack {
            if let Some(installation_id) = &slack.installation_id {
                check(Cow::Borrowed("slack.installation_id"), installation_id)?;
            }
            if let Some(team_id) = &slack.team_id {
                check(Cow::Borrowed("slack.team_id"), team_id)?;
            }
            if let Some(api_app_id) = &slack.api_app_id {
                check(Cow::Borrowed("slack.api_app_id"), api_app_id)?;
            }
            if let Some(slack_user_id) = &slack.slack_user_id {
                check(Cow::Borrowed("slack.slack_user_id"), slack_user_id)?;
            }
            if let Some(user_id) = &slack.user_id {
                check(Cow::Borrowed("slack.user_id"), user_id)?;
            }
            if let Some(shared_subject_user_id) = &slack.shared_subject_user_id {
                check(
                    Cow::Borrowed("slack.shared_subject_user_id"),
                    shared_subject_user_id,
                )?;
            }
            for (index, route) in slack.channel_routes.iter().enumerate() {
                if let Some(channel_id) = &route.channel_id {
                    check_non_empty_trimmed(
                        Cow::Owned(format!("slack.channel_routes[{index}].channel_id")),
                        channel_id,
                    )?;
                }
                if let Some(subject_user_id) = &route.subject_user_id {
                    check_non_empty_trimmed(
                        Cow::Owned(format!("slack.channel_routes[{index}].subject_user_id")),
                        subject_user_id,
                    )?;
                }
            }
            if let Some(signing_secret_env) = &slack.signing_secret_env {
                check_non_empty_trimmed(
                    Cow::Borrowed("slack.signing_secret_env"),
                    signing_secret_env,
                )?;
                validate_env_var_reference(
                    "slack.signing_secret_env",
                    signing_secret_env,
                    attributed_path,
                )?;
            }
            if let Some(bot_token_env) = &slack.bot_token_env {
                check_non_empty_trimmed(Cow::Borrowed("slack.bot_token_env"), bot_token_env)?;
                validate_env_var_reference("slack.bot_token_env", bot_token_env, attributed_path)?;
            }
        }
        if let Some(budget) = &self.budget {
            if let Some(tz) = &budget.default_tz {
                check(Cow::Borrowed("budget.default_tz"), tz)?;
            }
            // 0 is a legitimate sentinel for "unlimited". Negative values
            // are rejected outright so a bad number doesn't masquerade as a
            // disabled cap.
            for (label, value) in [
                ("budget.user_daily_usd", budget.user_daily_usd),
                ("budget.project_daily_usd", budget.project_daily_usd),
                ("budget.mission_per_tick_usd", budget.mission_per_tick_usd),
                (
                    "budget.heartbeat_per_tick_usd",
                    budget.heartbeat_per_tick_usd,
                ),
                (
                    "budget.routine_lightweight_usd",
                    budget.routine_lightweight_usd,
                ),
                ("budget.routine_standard_usd", budget.routine_standard_usd),
                (
                    "budget.background_job_default_usd",
                    budget.background_job_default_usd,
                ),
                ("budget.overestimate_factor", budget.overestimate_factor),
            ] {
                if let Some(v) = value
                    && v.is_finite()
                    && v < 0.0
                {
                    return Err(RebornConfigFileError::InvalidApiVersion {
                        path: path_str(),
                        found: format!("{label} = {v}"),
                        reason: "must be >= 0 (use 0 for unlimited)".to_string(),
                    });
                }
            }
            for (label, value) in [
                ("budget.warn_at", budget.warn_at),
                ("budget.pause_at", budget.pause_at),
            ] {
                if let Some(v) = value
                    && !(0.0..=1.0).contains(&v)
                {
                    return Err(RebornConfigFileError::InvalidApiVersion {
                        path: path_str(),
                        found: format!("{label} = {v}"),
                        reason: "thresholds must be in [0.0, 1.0]".to_string(),
                    });
                }
            }
            if let (Some(w), Some(p)) = (budget.warn_at, budget.pause_at)
                && p < w
            {
                return Err(RebornConfigFileError::InvalidApiVersion {
                    path: path_str(),
                    found: format!("warn_at={w}, pause_at={p}"),
                    reason: "pause_at must be >= warn_at".to_string(),
                });
            }
        }
        Ok(())
    }

    /// Resolve the `default` LLM slot, if present.
    pub fn default_llm_slot(&self) -> Option<&LlmSlotSelection> {
        self.llm.as_ref().and_then(|map| map.get("default"))
    }
}

/// Apply a typed patch to `[llm.default]` while preserving unrelated TOML.
pub fn update_default_llm_slot(
    path: &Path,
    update: &DefaultLlmSlotUpdate,
) -> Result<(), RebornConfigFileUpdateError> {
    begin_default_llm_slot_update(path)?.apply(update)
}

fn llm_slot_field_label(slot: &str, field: &str) -> Cow<'static, str> {
    Cow::Owned(format!("llm.{slot}.{field}"))
}

pub fn begin_default_llm_slot_update(
    path: &Path,
) -> Result<DefaultLlmSlotUpdateSession, RebornConfigFileUpdateError> {
    let lock_file = acquire_update_lock(path)?;
    let doc = load_edit_document(path)?;
    Ok(DefaultLlmSlotUpdateSession {
        path: path.to_path_buf(),
        doc,
        _lock_file: lock_file,
    })
}

fn acquire_update_lock(path: &Path) -> Result<fs::File, RebornConfigFileUpdateError> {
    use fs4::FileExt as _;

    let lock_path = config_update_lock_path(path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|source| RebornConfigFileUpdateError::Lock {
            path: lock_path.clone(),
            source,
        })?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| RebornConfigFileUpdateError::Lock {
            path: lock_path.clone(),
            source,
        })?;
    file.lock_exclusive()
        .map_err(|source| RebornConfigFileUpdateError::Lock {
            path: lock_path,
            source,
        })?;
    Ok(file)
}

fn config_update_lock_path(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name() else {
        return path.with_extension("lock");
    };
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    path.with_file_name(lock_name)
}

fn load_edit_document(path: &Path) -> Result<toml_edit::DocumentMut, RebornConfigFileUpdateError> {
    match fs::read_to_string(path) {
        Ok(text) => text.parse::<toml_edit::DocumentMut>().map_err(|source| {
            RebornConfigFileUpdateError::Parse {
                path: path.to_path_buf(),
                source,
            }
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml_edit::DocumentMut::new())
        }
        Err(source) => Err(RebornConfigFileUpdateError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn apply_llm_slot_field(
    doc: &mut toml_edit::DocumentMut,
    field: &str,
    update: &LlmSlotFieldUpdate,
) {
    match update {
        LlmSlotFieldUpdate::Keep => {}
        LlmSlotFieldUpdate::Set(value) => {
            ensure_llm_default_table(doc);
            doc["llm"]["default"][field] = toml_edit::value(value);
        }
        LlmSlotFieldUpdate::Remove => {
            ensure_llm_default_table(doc);
            if let Some(table) = doc["llm"]["default"].as_table_like_mut() {
                table.remove(field);
            }
        }
    }
}

fn ensure_llm_default_table(doc: &mut toml_edit::DocumentMut) {
    let root = doc.as_table_mut();
    if root.get("llm").is_none_or(|item| !item.is_table()) {
        root.insert("llm", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    if let Some(llm) = doc["llm"].as_table_mut()
        && llm.get("default").is_none_or(|item| !item.is_table())
    {
        llm.insert("default", toml_edit::Item::Table(toml_edit::Table::new()));
    }
}

fn write_edit_document(
    path: &Path,
    doc: &toml_edit::DocumentMut,
) -> Result<(), RebornConfigFileUpdateError> {
    let text = doc.to_string();
    RebornConfigFile::parse_text(&text, path).map_err(|source| {
        RebornConfigFileUpdateError::Validate {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RebornConfigFileUpdateError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|source| RebornConfigFileUpdateError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    tmp.write_all(text.as_bytes())
        .map_err(|source| RebornConfigFileUpdateError::Write {
            path: tmp.path().to_path_buf(),
            source,
        })?;
    tmp.persist(path)
        .map_err(|error| RebornConfigFileUpdateError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}

fn validate_api_version(found: &str, path: &Path) -> Result<(), RebornConfigFileError> {
    // Expected shape: `ironclaw.runtime/vMAJOR.MINOR` (minor optional).
    let Some(rest) = found.strip_prefix("ironclaw.runtime/v") else {
        return Err(RebornConfigFileError::InvalidApiVersion {
            path: path.display().to_string(),
            found: found.to_string(),
            reason: "expected prefix `ironclaw.runtime/v`".to_string(),
        });
    };
    let mut parts = rest.split('.');
    let major_str = parts.next().unwrap_or("");
    let major: u32 = major_str
        .parse()
        .map_err(
            |error: std::num::ParseIntError| RebornConfigFileError::InvalidApiVersion {
                path: path.display().to_string(),
                found: found.to_string(),
                reason: format!("major version is not a u32: {error}"),
            },
        )?;
    if let Some(minor_str) = parts.next() {
        let _minor: u32 = minor_str
            .parse()
            .map_err(
                |error: std::num::ParseIntError| RebornConfigFileError::InvalidApiVersion {
                    path: path.display().to_string(),
                    found: found.to_string(),
                    reason: format!("minor version is not a u32: {error}"),
                },
            )?;
    }
    if parts.next().is_some() {
        return Err(RebornConfigFileError::InvalidApiVersion {
            path: path.display().to_string(),
            found: found.to_string(),
            reason: "expected at most major.minor components".to_string(),
        });
    }
    // Compatibility is major-fail-closed, minor-accept: all v1.x boot
    // files are valid for this slice, but any other major is refused.
    if major != 1 {
        return Err(RebornConfigFileError::IncompatibleApiVersion {
            path: path.display().to_string(),
            found: found.to_string(),
            expected: REBORN_CONFIG_API_VERSION,
        });
    }
    Ok(())
}

fn validate_env_var_reference(
    field: &str,
    value: &str,
    path: &Path,
) -> Result<(), RebornConfigFileError> {
    let mut chars = value.chars();
    let valid = chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_');
    if valid {
        return Ok(());
    }
    Err(RebornConfigFileError::InvalidField {
        path: path.display().to_string(),
        field: field.to_string(),
        reason: "must be an environment variable name, not an inline secret or URL".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn attributed() -> PathBuf {
        PathBuf::from("/test/config.toml")
    }

    #[test]
    fn missing_file_is_ok_none() {
        let path = PathBuf::from("/does/not/exist/anywhere/config.toml");
        let result = RebornConfigFile::load(&path).expect("missing file must not error");
        assert!(result.is_none());
    }

    #[test]
    fn empty_file_parses_to_all_none() {
        let cfg = RebornConfigFile::parse_text("", &attributed()).expect("empty TOML is valid");
        assert!(cfg.api_version.is_none());
        assert!(cfg.boot.is_none());
        assert!(cfg.identity.is_none());
        assert!(cfg.policy.is_none());
        assert!(cfg.drivers.is_none());
        assert!(cfg.harness.is_none());
        assert!(cfg.runner.is_none());
        assert!(cfg.skills.is_none());
        assert!(cfg.storage.is_none());
        assert!(cfg.llm.is_none());
        assert!(cfg.slack.is_none());
    }

    #[test]
    fn runner_section_new_fields_round_trip() {
        let toml = r#"
[runner]
heartbeat_interval_secs = 10
poll_interval_ms = 100
worker_count = 3
max_concurrent_runs_per_user = 2
max_concurrent_trigger_runs = 5
max_concurrent_conversation_runs = 4
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed()).expect("must parse");
        let runner = cfg.runner.as_ref().expect("runner section present");
        assert_eq!(runner.heartbeat_interval_secs, Some(10));
        assert_eq!(runner.poll_interval_ms, Some(100));
        assert_eq!(runner.worker_count, Some(3));
        assert_eq!(runner.max_concurrent_runs_per_user, Some(2));
        assert_eq!(runner.max_concurrent_trigger_runs, Some(5));
        assert_eq!(runner.max_concurrent_conversation_runs, Some(4));
    }

    #[test]
    fn absent_runner_leaves_new_fields_none() {
        let cfg = RebornConfigFile::parse_text("", &attributed()).expect("empty TOML is valid");
        assert!(cfg.runner.is_none());
    }

    #[test]
    fn runner_section_with_only_new_fields() {
        let toml = r#"
[runner]
worker_count = 8
max_concurrent_runs_per_user = 1
max_concurrent_trigger_runs = 10
max_concurrent_conversation_runs = 5
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed()).expect("must parse");
        let runner = cfg.runner.as_ref().expect("runner section present");
        assert_eq!(runner.heartbeat_interval_secs, None);
        assert_eq!(runner.poll_interval_ms, None);
        assert_eq!(runner.worker_count, Some(8));
        assert_eq!(runner.max_concurrent_runs_per_user, Some(1));
        assert_eq!(runner.max_concurrent_trigger_runs, Some(10));
        assert_eq!(runner.max_concurrent_conversation_runs, Some(5));
    }

    #[test]
    fn full_file_round_trips() {
        let toml = r#"
api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
tenant = "acme"
default_agent = "acme-bot"
default_owner = "acme-operator"

[policy]
deployment_mode = "local_single_user"
default_profile = "local_dev"
default_approval_policy = "ask_destructive"

[drivers]
default = "text_only"
additional = ["planned"]

[harness]
id = "red-team"

[runner]
heartbeat_interval_secs = 5
poll_interval_ms = 200

[skills]
regex_activation_enabled = false

[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
pool_max_size = 32

[llm.default]
provider_id = "openai"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"

[llm.mission]
provider_id = "anthropic"
model = "claude-3-5-sonnet-latest"
api_key_env = "ANTHROPIC_API_KEY"

[slack]
enabled = true
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed()).expect("must parse");
        assert_eq!(cfg.api_version.as_deref(), Some("ironclaw.runtime/v1"));
        assert_eq!(
            cfg.boot.as_ref().unwrap().profile.as_deref(),
            Some("local-dev")
        );
        assert_eq!(
            cfg.identity.as_ref().unwrap().tenant.as_deref(),
            Some("acme")
        );
        assert_eq!(
            cfg.drivers.as_ref().unwrap().additional.as_deref(),
            Some(&["planned".to_string()][..])
        );
        assert_eq!(
            cfg.skills.as_ref().unwrap().regex_activation_enabled,
            Some(false)
        );
        let storage = cfg.storage.as_ref().expect("storage section present");
        assert_eq!(storage.backend, Some(StorageBackend::Postgres));
        assert_eq!(
            storage.url_env.as_deref(),
            Some("IRONCLAW_REBORN_POSTGRES_URL")
        );
        assert_eq!(
            storage.secret_master_key_env.as_deref(),
            Some("IRONCLAW_REBORN_SECRET_MASTER_KEY")
        );
        assert_eq!(storage.pool_max_size, Some(32));
        let default_slot = cfg.default_llm_slot().expect("default slot present");
        assert_eq!(default_slot.provider_id.as_deref(), Some("openai"));
        assert_eq!(default_slot.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(default_slot.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        let llm = cfg.llm.as_ref().unwrap();
        assert!(llm.contains_key("mission"));
        let slack = cfg.slack.as_ref().expect("slack section present");
        assert_eq!(slack.enabled, Some(true));
    }

    #[test]
    fn default_llm_update_preserves_unrelated_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            r#"
[identity]
tenant = "acme"

[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"
base_url = "https://example.test/v1"

[llm.mission]
provider_id = "anthropic"
"#,
        )
        .expect("write config");

        update_default_llm_slot(
            &path,
            &DefaultLlmSlotUpdate {
                provider_id: LlmSlotFieldUpdate::Keep,
                model: LlmSlotFieldUpdate::Set("gpt-5.3-codex".to_string()),
                api_key_env: LlmSlotFieldUpdate::Keep,
                base_url: LlmSlotFieldUpdate::Remove,
            },
        )
        .expect("update config");

        let text = fs::read_to_string(&path).expect("read config");
        assert!(text.contains("[identity]"), "config: {text}");
        assert!(text.contains("tenant = \"acme\""), "config: {text}");
        assert!(text.contains("[llm.mission]"), "config: {text}");
        assert!(text.contains("model = \"gpt-5.3-codex\""), "config: {text}");
        assert!(
            text.contains("api_key_env = \"OPENAI_API_KEY\""),
            "config: {text}"
        );
        assert!(!text.contains("base_url"), "config: {text}");
        RebornConfigFile::load(&path)
            .expect("valid config")
            .expect("config present");
    }

    #[test]
    fn default_llm_update_rejects_malformed_existing_toml() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        fs::write(&path, "[llm.default\nprovider_id = \"openai\"").expect("write config");

        let err = update_default_llm_slot(
            &path,
            &DefaultLlmSlotUpdate {
                model: LlmSlotFieldUpdate::Set("gpt-5-mini".to_string()),
                ..Default::default()
            },
        )
        .expect_err("malformed existing TOML should reject");

        assert!(matches!(err, RebornConfigFileUpdateError::Parse { .. }));
    }

    #[test]
    fn default_llm_update_rejects_inline_secret_value_without_writing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            r#"
[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
"#,
        )
        .expect("write config");
        let before = fs::read_to_string(&path).expect("read config");

        let err = update_default_llm_slot(
            &path,
            &DefaultLlmSlotUpdate {
                api_key_env: LlmSlotFieldUpdate::Set(
                    "sk-proj-1234567890abcdef1234567890".to_string(),
                ),
                ..Default::default()
            },
        )
        .expect_err("inline secret should reject");

        assert!(matches!(err, RebornConfigFileUpdateError::Validate { .. }));
        assert_eq!(fs::read_to_string(&path).expect("read config"), before);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let toml = r#"
something_not_recognized = "foo"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("unknown key must fail parse");
        assert!(matches!(err, RebornConfigFileError::Toml { .. }));
    }

    #[test]
    fn rejects_unknown_section_key() {
        let toml = r#"
[identity]
typo_field = "foo"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("unknown section key must fail parse");
        assert!(matches!(err, RebornConfigFileError::Toml { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_api_key_env() {
        // api_key_env is supposed to be a NAME like "OPENAI_API_KEY";
        // pasting an actual key here is exactly the foot-gun the
        // secrets guard catches.
        let toml = r#"
[llm.default]
provider_id = "openai"
api_key_env = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
        let rendered = err.to_string();
        assert!(
            rendered.contains("llm.default.api_key_env"),
            "slot-specific label should guide operator to the bad field: {rendered}"
        );
    }

    #[test]
    fn parses_legacy_slack_setup_fields() {
        let toml = r#"
[slack]
enabled = true
installation_id = "install-alpha"
team_id = "T123"
api_app_id = "A123"
slack_user_id = "U123"
user_id = "user:operator"
shared_subject_user_id = "user:slack-shared"
signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"

[[slack.channel_routes]]
channel_id = "CENG"
subject_user_id = "user:eng-team-agent"
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed())
            .expect("legacy Slack setup fields should remain parse-compatible");
        let slack = cfg.slack.expect("slack section");
        assert_eq!(slack.enabled, Some(true));
        assert_eq!(slack.installation_id.as_deref(), Some("install-alpha"));
        assert_eq!(slack.team_id.as_deref(), Some("T123"));
        assert_eq!(slack.api_app_id.as_deref(), Some("A123"));
        assert_eq!(slack.slack_user_id.as_deref(), Some("U123"));
        assert_eq!(slack.user_id.as_deref(), Some("user:operator"));
        assert_eq!(
            slack.shared_subject_user_id.as_deref(),
            Some("user:slack-shared")
        );
        assert_eq!(
            slack.signing_secret_env.as_deref(),
            Some("IRONCLAW_REBORN_SLACK_SIGNING_SECRET")
        );
        assert_eq!(slack.channel_routes.len(), 1);
        assert_eq!(slack.channel_routes[0].channel_id.as_deref(), Some("CENG"));
        assert_eq!(
            slack.channel_routes[0].subject_user_id.as_deref(),
            Some("user:eng-team-agent")
        );
    }

    #[test]
    fn parses_storage_postgres_env_reference() {
        let toml = r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
pool_max_size = 24
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed())
            .expect("storage env reference must parse");
        let storage = cfg.storage.expect("storage section");
        assert_eq!(storage.backend, Some(StorageBackend::Postgres));
        assert_eq!(
            storage.url_env.as_deref(),
            Some("IRONCLAW_REBORN_POSTGRES_URL")
        );
        assert_eq!(
            storage.secret_master_key_env.as_deref(),
            Some("IRONCLAW_REBORN_SECRET_MASTER_KEY")
        );
        assert_eq!(storage.pool_max_size, Some(24));
    }

    #[test]
    fn rejects_zero_storage_pool_max_size() {
        let toml = r#"
[storage]
backend = "postgres"
pool_max_size = 0
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("zero pool_max_size must be rejected");
        assert!(
            err.to_string().contains("storage.pool_max_size"),
            "error should identify storage.pool_max_size: {err}"
        );
    }

    #[test]
    fn rejects_whitespace_only_storage_backend() {
        let toml = r#"
[storage]
backend = "   "
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("whitespace-only backend must be rejected");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.backend"),
            "error should identify storage.backend: {err}"
        );
    }

    #[test]
    fn rejects_url_shaped_storage_backend_without_echoing_credentials() {
        let toml = r#"
[storage]
backend = "postgres://user:password@db.example.com/ironclaw"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("backend must not accept raw URLs");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.backend"),
            "error should identify storage.backend: {err}"
        );
        assert!(
            !err.to_string().contains("password"),
            "error must not echo credential-bearing backend value: {err}"
        );
    }

    #[test]
    fn rejects_whitespace_only_storage_secret_master_key_env() {
        let toml = r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "   "
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("whitespace-only secret_master_key_env must be rejected");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.secret_master_key_env"),
            "error should identify storage.secret_master_key_env: {err}"
        );
    }

    #[test]
    fn rejects_whitespace_only_storage_url_env() {
        let toml = r#"
[storage]
url_env = "   "
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("whitespace-only url_env must be rejected");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.url_env"),
            "error should identify storage.url_env: {err}"
        );
    }

    #[test]
    fn rejects_inline_postgres_url_in_storage_url_env() {
        let toml = r#"
[storage]
backend = "postgres"
url_env = "postgres://user:password@db.example.com/ironclaw?sslmode=require"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("storage url_env must not accept raw URLs");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.url_env"),
            "error should identify storage.url_env: {err}"
        );
        assert!(
            !err.to_string().contains("password"),
            "error must not echo credential-bearing URL: {err}"
        );
    }

    #[test]
    fn rejects_inline_secret_in_storage_secret_master_key_env() {
        let toml = r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "postgres://user:password.example.com/ironclaw"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("storage secret_master_key_env must not accept raw secrets");
        assert!(matches!(err, RebornConfigFileError::InvalidField { .. }));
        assert!(
            err.to_string().contains("storage.secret_master_key_env"),
            "error should identify storage.secret_master_key_env: {err}"
        );
        assert!(
            !err.to_string().contains("password"),
            "error must not echo credential-bearing value: {err}"
        );
    }

    #[test]
    fn rejects_inline_secret_in_legacy_slack_secret_env_name() {
        let toml = r#"
[slack]
enabled = true
signing_secret_env = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("legacy Slack env name must not accept raw secrets");
        assert!(
            err.to_string().contains("slack.signing_secret_env"),
            "error should identify legacy Slack field: {err}"
        );
    }

    #[test]
    fn rejects_inline_secret_in_legacy_slack_bot_token_env_name() {
        let toml = r#"
[slack]
enabled = true
bot_token_env = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("legacy Slack bot token env name must not accept raw secrets");
        assert!(
            err.to_string().contains("slack.bot_token_env"),
            "error should identify legacy Slack field: {err}"
        );
    }

    #[test]
    fn rejects_inline_secret_in_provider_id() {
        let toml = r#"
[llm.default]
provider_id = " sk-proj-1234567890abcdef1234567890 "
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_boot_profile_before_profile_parse() {
        let toml = r#"
[boot]
profile = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_identity_default_owner() {
        let toml = r#"
[identity]
default_owner = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_driver_list() {
        let toml = r#"
[drivers]
additional = ["planned", "sk-proj-1234567890abcdef1234567890"]
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_api_version_before_version_parse() {
        let toml = r#"
api_version = "sk-proj-1234567890abcdef1234567890"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_inline_secret_in_llm_slot_key() {
        let toml = r#"
[llm."sk-proj-1234567890abcdef1234567890"]
provider_id = "openai"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("inline secret must be rejected");
        assert!(matches!(err, RebornConfigFileError::InlineSecret { .. }));
    }

    #[test]
    fn rejects_future_major_api_version_fail_closed() {
        let toml = r#"
api_version = "ironclaw.runtime/v9"
"#;
        let err =
            RebornConfigFile::parse_text(toml, &attributed()).expect_err("major bump must fail");
        assert!(matches!(
            err,
            RebornConfigFileError::IncompatibleApiVersion { .. }
        ));
    }

    #[test]
    fn accepts_v1_minor_bumps_forward_compat() {
        for version in ["ironclaw.runtime/v1.0", "ironclaw.runtime/v1.7"] {
            let toml = format!(r#"api_version = "{version}""#);
            let cfg = RebornConfigFile::parse_text(&toml, &attributed())
                .expect("minor bumps must be accepted");
            assert_eq!(cfg.api_version.as_deref(), Some(version));
        }
    }

    #[test]
    fn rejects_malformed_api_version() {
        let toml = r#"
api_version = "ironclaw.runtime/notaversion"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("garbage version must fail");
        assert!(matches!(
            err,
            RebornConfigFileError::InvalidApiVersion { .. }
        ));
    }

    #[test]
    fn rejects_malformed_api_version_minor() {
        let toml = r#"
api_version = "ironclaw.runtime/v1.not-a-number"
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("malformed minor version must fail");
        assert!(matches!(
            err,
            RebornConfigFileError::InvalidApiVersion { .. }
        ));
    }

    #[test]
    fn parses_valid_budget_section() {
        let toml = r#"
[budget]
user_daily_usd = 7.50
project_daily_usd = 0.00
mission_per_tick_usd = 0.25
heartbeat_per_tick_usd = 0.05
routine_lightweight_usd = 0.01
routine_standard_usd = 0.20
background_job_default_usd = 2.00
default_tz = "America/Los_Angeles"
warn_at = 0.60
pause_at = 0.85
overestimate_factor = 1.50
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed())
            .expect("valid budget section must parse");
        let budget = cfg.budget.as_ref().expect("budget section present");
        assert_eq!(budget.user_daily_usd, Some(7.50));
        assert_eq!(budget.project_daily_usd, Some(0.00));
        assert_eq!(budget.default_tz.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(budget.warn_at, Some(0.60));
        assert_eq!(budget.pause_at, Some(0.85));
        assert_eq!(budget.overestimate_factor, Some(1.50));
    }

    #[test]
    fn rejects_negative_budget_usd_field() {
        let toml = r#"
[budget]
user_daily_usd = -1.0
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("negative USD must be rejected");
        assert!(matches!(
            err,
            RebornConfigFileError::InvalidApiVersion { .. }
        ));
        assert!(err.to_string().contains("user_daily_usd"));
    }

    #[test]
    fn rejects_budget_threshold_out_of_range() {
        let toml = r#"
[budget]
warn_at = 1.5
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("out-of-range threshold must be rejected");
        assert!(matches!(
            err,
            RebornConfigFileError::InvalidApiVersion { .. }
        ));
        assert!(err.to_string().contains("warn_at"));
    }

    #[test]
    fn rejects_budget_pause_below_warn() {
        let toml = r#"
[budget]
warn_at = 0.90
pause_at = 0.50
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("pause_at < warn_at must be rejected");
        assert!(matches!(
            err,
            RebornConfigFileError::InvalidApiVersion { .. }
        ));
        assert!(err.to_string().contains("pause_at"));
    }

    #[test]
    fn rejects_unknown_budget_section_key() {
        let toml = r#"
[budget]
not_a_field = 1.0
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("deny_unknown_fields must catch typos in [budget]");
        assert!(matches!(err, RebornConfigFileError::Toml { .. }));
    }

    #[test]
    fn trigger_poller_full_section_parses() {
        let toml = r#"
[trigger_poller]
enabled = true
poll_interval_secs = 30
fires_per_tick = 50
max_concurrent_fires_per_trigger = 3
startup_jitter_max_secs = 10
tick_jitter_max_secs = 5
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed())
            .expect("full trigger_poller section must parse");
        let tp = cfg
            .trigger_poller
            .as_ref()
            .expect("trigger_poller section present");
        assert_eq!(tp.enabled, Some(true));
        assert_eq!(tp.poll_interval_secs, Some(30));
        assert_eq!(tp.fires_per_tick, Some(50));
        // max_concurrent_fires_per_trigger is intentionally not 1 here: this test
        // exercises the parse layer, which deliberately accepts any u32. The CLI
        // settings layer (trigger_poller_settings) enforces the V1 invariant that
        // the value must equal 1 — see runtime/trigger_poller.rs.
        assert_eq!(tp.max_concurrent_fires_per_trigger, Some(3));
        assert_eq!(tp.startup_jitter_max_secs, Some(10));
        assert_eq!(tp.tick_jitter_max_secs, Some(5));
    }

    #[test]
    fn trigger_poller_absent_section_yields_none() {
        let cfg = RebornConfigFile::parse_text("", &attributed()).expect("empty TOML must parse");
        assert!(cfg.trigger_poller.is_none());
    }

    #[test]
    fn trigger_poller_partial_section_other_fields_none() {
        let toml = r#"
[trigger_poller]
enabled = true
"#;
        let cfg = RebornConfigFile::parse_text(toml, &attributed())
            .expect("partial trigger_poller section must parse");
        let tp = cfg
            .trigger_poller
            .as_ref()
            .expect("trigger_poller section present");
        assert_eq!(tp.enabled, Some(true));
        assert_eq!(tp.poll_interval_secs, None);
        assert_eq!(tp.fires_per_tick, None);
        assert_eq!(tp.max_concurrent_fires_per_trigger, None);
        assert_eq!(tp.startup_jitter_max_secs, None);
        assert_eq!(tp.tick_jitter_max_secs, None);
    }

    #[test]
    fn trigger_poller_rejects_unknown_key() {
        let toml = r#"
[trigger_poller]
not_a_field = true
"#;
        let err = RebornConfigFile::parse_text(toml, &attributed())
            .expect_err("deny_unknown_fields must catch typos in [trigger_poller]");
        assert!(matches!(err, RebornConfigFileError::Toml { .. }));
    }
}
