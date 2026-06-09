//! Boot configuration contracts for the standalone IronClaw Reborn binary.
//!
//! This crate is intentionally small and has no IronClaw workspace dependencies.
//! It owns process/environment boot configuration that must be shared by the
//! `ironclaw-reborn` binary and later Reborn runtime composition without pulling
//! in the v1 root application.
//!
//! Four boot-time surfaces live here:
//!
//! - [`RebornBootConfig`] — home + profile resolved from env vars at
//!   process start. The original API; unchanged.
//! - [`RebornConfigFile`] — the operator-edited TOML at
//!   `$IRONCLAW_REBORN_HOME/config.toml`. Read once at process start;
//!   provides the *selection* layer of the three-layer config model
//!   (catalog → selection → runtime config). See `config_file.rs`.
//! - Provider catalog — lives in `$IRONCLAW_REBORN_HOME/providers.json`
//!   in the v1 `providers.json` shape. This crate exposes the path via
//!   [`RebornHome::providers_file_path`]; loading the file goes through
//!   `ironclaw_llm::ProviderRegistry` in the composition root (this
//!   crate has no workspace deps, per boundary rules).
//! - [`seed_default_config_file_if_missing`] — first-run seeding for the
//!   sparse runtime `config.toml` written by stateful Reborn commands.

mod boot;
mod budget;
mod config_file;
mod config_seed;
mod doctor;
mod home;
mod profile;
mod secrets_guard;

pub use boot::RebornBootConfig;
pub use budget::{
    BACKGROUND_JOB_DEFAULT_USD_ENV, BUDGET_DEFAULT_TZ_ENV, BUDGET_OVERESTIMATE_FACTOR_ENV,
    BUDGET_PAUSE_AT_ENV, BUDGET_WARN_AT_ENV, BudgetDefaults, BudgetDefaultsError,
    HEARTBEAT_PER_TICK_USD_ENV, MISSION_PER_TICK_USD_ENV, PROJECT_DAILY_USD_ENV,
    ROUTINE_LIGHTWEIGHT_USD_ENV, ROUTINE_STANDARD_USD_ENV, USER_DAILY_USD_ENV,
};
pub use config_file::{
    BootSection, BudgetSection, DefaultLlmSlotUpdate, DefaultLlmSlotUpdateSession, DriversSection,
    HarnessSection, IdentitySection, LlmSlotFieldUpdate, LlmSlotSelection, PolicySection,
    REBORN_CONFIG_API_VERSION, RebornConfigFile, RebornConfigFileError,
    RebornConfigFileUpdateError, RunnerSection, SlackChannelRouteSection, SlackSection,
    TriggerPollerConfigSection, begin_default_llm_slot_update, update_default_llm_slot,
};
pub use config_seed::{
    RebornConfigSeedError, RebornConfigSeedOutcome, seed_default_config_file_if_missing,
};
pub use doctor::RebornDoctorReport;
pub use home::{REBORN_HOME_ENV, RebornConfigError, RebornHome, RebornHomeSource};
pub use profile::{REBORN_PROFILE_ENV, RebornProfile};
pub use secrets_guard::{InlineSecretError, reject_inline_secret};
