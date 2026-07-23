//! Boot configuration contracts for the standalone IronClaw binary.
//!
//! This crate is intentionally small and has no IronClaw workspace dependencies.
//! It owns process/environment boot configuration that must be shared by the
//! `ironclaw` binary and later IronClaw runtime composition without pulling
//! in the v1 root application.
//!
//! Four boot-time surfaces live here:
//!
//! - [`IronClawBootConfig`] — home + profile resolved from env vars at
//!   process start. The original API; unchanged.
//! - [`IronClawConfigFile`] — the operator-edited TOML at
//!   `$IRONCLAW_HOME/config.toml`. Read once at process start;
//!   provides the *selection* layer of the three-layer config model
//!   (catalog → selection → runtime config). See `config_file.rs`.
//! - Provider catalog — lives in `$IRONCLAW_HOME/providers.json`
//!   in the v1 `providers.json` shape. This crate exposes the path via
//!   [`IronClawHome::providers_file_path`]; loading the file goes through
//!   `ironclaw_llm::ProviderRegistry` in the composition root (this
//!   crate has no workspace deps, per boundary rules).
//! - [`seed_default_config_file_if_missing`] — first-run seeding for the
//!   sparse runtime `config.toml` written by stateful IronClaw commands.

mod boot;
mod budget;
mod capability_remediation;
mod config_file;
mod config_seed;
mod doctor;
mod home;
mod profile;
mod secrets_guard;

pub use boot::IronClawBootConfig;
pub use budget::{
    BACKGROUND_JOB_DEFAULT_USD_ENV, BUDGET_DEFAULT_TZ_ENV, BUDGET_OVERESTIMATE_FACTOR_ENV,
    BUDGET_PAUSE_AT_ENV, BUDGET_WARN_AT_ENV, BudgetDefaults, BudgetDefaultsError,
    HEARTBEAT_PER_TICK_USD_ENV, MISSION_PER_TICK_USD_ENV, PROJECT_DAILY_USD_ENV,
    ROUTINE_LIGHTWEIGHT_USD_ENV, ROUTINE_STANDARD_USD_ENV, USER_DAILY_USD_ENV,
};
pub use capability_remediation::{
    HostRemediationText, apply_step_text, google_backend_auth_text, google_not_configured_text,
    google_remediation_text, google_setup_steps_text,
};
pub use config_file::{
    BootSection, BudgetSection, DefaultLlmSlotUpdate, DefaultLlmSlotUpdateSession, DriversSection,
    GoogleFieldUpdate, GoogleOauthConfigUpdate, GoogleOauthConfigUpdateSession, GoogleSection,
    HarnessSection, IRONCLAW_CONFIG_API_VERSION, IdentitySection, IronClawConfigFile,
    IronClawConfigFileError, IronClawConfigFileUpdateError, LlmSlotFieldUpdate, LlmSlotSelection,
    PolicySection, RunnerSection, SlackChannelRouteSection, SlackSection, StorageBackend,
    StorageSection, TelegramSection, TriggerPollerConfigSection, begin_default_llm_slot_update,
    begin_google_oauth_config_update, update_default_llm_slot, update_google_oauth_config,
    update_slack_enabled,
};
pub use config_seed::{
    IronClawConfigSeedError, IronClawConfigSeedOutcome, seed_default_config_file_if_missing,
};
pub use doctor::IronClawDoctorReport;
pub use home::{
    IRONCLAW_HOME_ENV, IronClawConfigError, IronClawHome, IronClawHomeSource,
    LEGACY_IRONCLAW_HOME_ENV,
};
pub use profile::{IRONCLAW_PROFILE_ENV, IronClawProfile, LEGACY_IRONCLAW_PROFILE_ENV};
pub use secrets_guard::{InlineSecretError, reject_inline_secret};
