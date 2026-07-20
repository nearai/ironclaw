//! Resolved cost-based budget defaults for IronClaw Reborn.
//!
//! Three layers merge to produce a [`BudgetDefaults`]:
//!
//! 1. Compiled defaults — production-tuned safe ceilings ($5/day/user,
//!    $2/day/project, $0.05/tick heartbeat, etc.).
//! 2. The `[budget]` section of the TOML config file
//!    ([`crate::BudgetSection`]).
//! 3. Environment variables (highest precedence) — see the `*_ENV`
//!    constants. Setting any USD field to `0` makes that dimension
//!    unlimited.
//!
//! This crate stays free of workspace dependencies on the host_api/types
//! family, so timezone and threshold validation happen here as plain
//! string + f64 checks; the composition root promotes them into the
//! typed `BudgetPeriod::Calendar { tz, … }` / `BudgetThresholds { … }`
//! when seeding the governor.

use std::env;

use crate::BudgetSection;

/// Per-user daily ceiling in USD. `0` = unlimited.
pub const USER_DAILY_USD_ENV: &str = "IRONCLAW_BUDGET_USER_DAILY_USD";
/// Per-project daily ceiling in USD. `0` = unlimited.
pub const PROJECT_DAILY_USD_ENV: &str = "IRONCLAW_BUDGET_PROJECT_DAILY_USD";
/// Per-tick budget for background missions. `0` = unlimited.
pub const MISSION_PER_TICK_USD_ENV: &str = "IRONCLAW_BUDGET_MISSION_PER_TICK_USD";
/// Per-tick budget for heartbeat ticks. `0` = unlimited.
pub const HEARTBEAT_PER_TICK_USD_ENV: &str = "IRONCLAW_BUDGET_HEARTBEAT_PER_TICK_USD";
/// Per-fire budget for lightweight routines. `0` = unlimited.
pub const ROUTINE_LIGHTWEIGHT_USD_ENV: &str = "IRONCLAW_BUDGET_ROUTINE_LIGHTWEIGHT_USD";
/// Per-fire budget for standard routines. `0` = unlimited.
pub const ROUTINE_STANDARD_USD_ENV: &str = "IRONCLAW_BUDGET_ROUTINE_STANDARD_USD";
/// Default per-job budget for one-shot container jobs. `0` = unlimited.
pub const BACKGROUND_JOB_DEFAULT_USD_ENV: &str = "IRONCLAW_BUDGET_BACKGROUND_JOB_DEFAULT_USD";
/// IANA timezone for calendar-period rollover.
pub const BUDGET_DEFAULT_TZ_ENV: &str = "IRONCLAW_BUDGET_DEFAULT_TZ";
/// Warn threshold fraction in `[0.0, 1.0]`.
pub const BUDGET_WARN_AT_ENV: &str = "IRONCLAW_BUDGET_WARN_AT";
/// Pause-with-approval threshold fraction in `[0.0, 1.0]`.
pub const BUDGET_PAUSE_AT_ENV: &str = "IRONCLAW_BUDGET_PAUSE_AT";
/// Pre-call estimate inflation multiplier.
pub const BUDGET_OVERESTIMATE_FACTOR_ENV: &str = "IRONCLAW_BUDGET_OVERESTIMATE_FACTOR";

/// Fully-resolved budget defaults handed to composition.
///
/// USD amounts are stored as `f64` (parsed from TOML/env); the composition
/// root converts to `Decimal` at the governor boundary. Any USD field
/// equal to `0.0` is preserved as-is — the governor interprets that as
/// "unlimited" via its `0 = unlimited` convention.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetDefaults {
    pub user_daily_usd: f64,
    pub project_daily_usd: f64,
    pub mission_per_tick_usd: f64,
    pub heartbeat_per_tick_usd: f64,
    pub routine_lightweight_usd: f64,
    pub routine_standard_usd: f64,
    pub background_job_default_usd: f64,
    pub default_tz: String,
    pub warn_at: f64,
    pub pause_at: f64,
    pub overestimate_factor: f64,
}

impl BudgetDefaults {
    /// Production-baseline defaults.
    pub fn compiled_defaults() -> Self {
        Self {
            user_daily_usd: 5.00,
            project_daily_usd: 2.00,
            mission_per_tick_usd: 0.50,
            heartbeat_per_tick_usd: 0.05,
            routine_lightweight_usd: 0.02,
            routine_standard_usd: 0.10,
            background_job_default_usd: 1.00,
            default_tz: "UTC".to_string(),
            warn_at: 0.75,
            pause_at: 0.90,
            overestimate_factor: 1.20,
        }
    }

    /// Apply the `[budget]` TOML section over compiled defaults.
    pub fn with_section(mut self, section: &BudgetSection) -> Self {
        if let Some(v) = section.user_daily_usd {
            self.user_daily_usd = v;
        }
        if let Some(v) = section.project_daily_usd {
            self.project_daily_usd = v;
        }
        if let Some(v) = section.mission_per_tick_usd {
            self.mission_per_tick_usd = v;
        }
        if let Some(v) = section.heartbeat_per_tick_usd {
            self.heartbeat_per_tick_usd = v;
        }
        if let Some(v) = section.routine_lightweight_usd {
            self.routine_lightweight_usd = v;
        }
        if let Some(v) = section.routine_standard_usd {
            self.routine_standard_usd = v;
        }
        if let Some(v) = section.background_job_default_usd {
            self.background_job_default_usd = v;
        }
        if let Some(tz) = section.default_tz.as_deref() {
            self.default_tz = tz.to_string();
        }
        if let Some(v) = section.warn_at {
            self.warn_at = v;
        }
        if let Some(v) = section.pause_at {
            self.pause_at = v;
        }
        if let Some(v) = section.overestimate_factor {
            self.overestimate_factor = v;
        }
        self
    }

    /// Apply env-var overrides over the current set.
    pub fn with_env(mut self) -> Result<Self, BudgetDefaultsError> {
        if let Some(v) = read_f64_env(USER_DAILY_USD_ENV)? {
            self.user_daily_usd = v;
        }
        if let Some(v) = read_f64_env(PROJECT_DAILY_USD_ENV)? {
            self.project_daily_usd = v;
        }
        if let Some(v) = read_f64_env(MISSION_PER_TICK_USD_ENV)? {
            self.mission_per_tick_usd = v;
        }
        if let Some(v) = read_f64_env(HEARTBEAT_PER_TICK_USD_ENV)? {
            self.heartbeat_per_tick_usd = v;
        }
        if let Some(v) = read_f64_env(ROUTINE_LIGHTWEIGHT_USD_ENV)? {
            self.routine_lightweight_usd = v;
        }
        if let Some(v) = read_f64_env(ROUTINE_STANDARD_USD_ENV)? {
            self.routine_standard_usd = v;
        }
        if let Some(v) = read_f64_env(BACKGROUND_JOB_DEFAULT_USD_ENV)? {
            self.background_job_default_usd = v;
        }
        if let Ok(tz) = env::var(BUDGET_DEFAULT_TZ_ENV)
            && !tz.is_empty()
        {
            self.default_tz = tz;
        }
        if let Some(v) = read_f64_env(BUDGET_WARN_AT_ENV)? {
            self.warn_at = v;
        }
        if let Some(v) = read_f64_env(BUDGET_PAUSE_AT_ENV)? {
            self.pause_at = v;
        }
        if let Some(v) = read_f64_env(BUDGET_OVERESTIMATE_FACTOR_ENV)? {
            self.overestimate_factor = v;
        }
        Ok(self)
    }

    /// Validate the resolved set. USD ceilings must be `>= 0` (`0` =
    /// unlimited); thresholds in `[0, 1]`; pause >= warn; overestimate
    /// factor `>= 1.0`.
    pub fn validate(&self) -> Result<(), BudgetDefaultsError> {
        for (label, value) in [
            ("user_daily_usd", self.user_daily_usd),
            ("project_daily_usd", self.project_daily_usd),
            ("mission_per_tick_usd", self.mission_per_tick_usd),
            ("heartbeat_per_tick_usd", self.heartbeat_per_tick_usd),
            ("routine_lightweight_usd", self.routine_lightweight_usd),
            ("routine_standard_usd", self.routine_standard_usd),
            (
                "background_job_default_usd",
                self.background_job_default_usd,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(BudgetDefaultsError::Invalid {
                    field: label,
                    reason: "must be a finite, non-negative number (0 = unlimited)".to_string(),
                });
            }
        }
        for (label, value) in [("warn_at", self.warn_at), ("pause_at", self.pause_at)] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(BudgetDefaultsError::Invalid {
                    field: label,
                    reason: "must be in [0.0, 1.0]".to_string(),
                });
            }
        }
        if self.pause_at < self.warn_at {
            return Err(BudgetDefaultsError::Invalid {
                field: "pause_at",
                reason: "must be >= warn_at".to_string(),
            });
        }
        if !self.overestimate_factor.is_finite() || self.overestimate_factor < 1.0 {
            return Err(BudgetDefaultsError::Invalid {
                field: "overestimate_factor",
                reason: "must be a finite number >= 1.0".to_string(),
            });
        }
        Ok(())
    }
}

impl Default for BudgetDefaults {
    fn default() -> Self {
        Self::compiled_defaults()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetDefaultsError {
    #[error("budget default `{field}` is invalid: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("environment variable `{name}` is not a valid f64: {value}")]
    InvalidEnvF64 { name: &'static str, value: String },
}

fn read_f64_env(name: &'static str) -> Result<Option<f64>, BudgetDefaultsError> {
    match env::var(name) {
        Ok(raw) if !raw.is_empty() => raw
            .parse::<f64>()
            .map(Some)
            .map_err(|_| BudgetDefaultsError::InvalidEnvF64 { name, value: raw }),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_defaults_validate() {
        BudgetDefaults::compiled_defaults().validate().unwrap();
    }

    #[test]
    fn zero_usd_means_unlimited_and_validates() {
        let mut d = BudgetDefaults::compiled_defaults();
        d.user_daily_usd = 0.0;
        d.project_daily_usd = 0.0;
        d.validate().unwrap();
    }

    #[test]
    fn negative_usd_rejected() {
        let mut d = BudgetDefaults::compiled_defaults();
        d.user_daily_usd = -1.0;
        let err = d.validate().unwrap_err();
        assert!(matches!(
            err,
            BudgetDefaultsError::Invalid {
                field: "user_daily_usd",
                ..
            }
        ));
    }

    #[test]
    fn pause_below_warn_rejected() {
        let mut d = BudgetDefaults::compiled_defaults();
        d.warn_at = 0.9;
        d.pause_at = 0.5;
        let err = d.validate().unwrap_err();
        assert!(matches!(
            err,
            BudgetDefaultsError::Invalid {
                field: "pause_at",
                ..
            }
        ));
    }

    #[test]
    fn overestimate_factor_below_one_rejected() {
        let mut d = BudgetDefaults::compiled_defaults();
        d.overestimate_factor = 0.5;
        let err = d.validate().unwrap_err();
        assert!(matches!(
            err,
            BudgetDefaultsError::Invalid {
                field: "overestimate_factor",
                ..
            }
        ));
    }

    #[test]
    fn section_layer_overrides_compiled() {
        let section = BudgetSection::default()
            .set_user_daily_usd(10.0)
            .set_project_daily_usd(4.0)
            .set_mission_per_tick_usd(0.75)
            .set_heartbeat_per_tick_usd(0.07)
            .set_routine_lightweight_usd(0.03)
            .set_routine_standard_usd(0.12)
            .set_background_job_default_usd(1.25)
            .set_default_tz("America/Los_Angeles")
            .set_warn_at(0.6)
            .set_pause_at(0.8)
            .set_overestimate_factor(1.5);
        let d = BudgetDefaults::compiled_defaults().with_section(&section);
        assert_eq!(d.user_daily_usd, 10.0);
        assert_eq!(d.project_daily_usd, 4.0);
        assert_eq!(d.mission_per_tick_usd, 0.75);
        assert_eq!(d.heartbeat_per_tick_usd, 0.07);
        assert_eq!(d.routine_lightweight_usd, 0.03);
        assert_eq!(d.routine_standard_usd, 0.12);
        assert_eq!(d.background_job_default_usd, 1.25);
        assert_eq!(d.default_tz, "America/Los_Angeles");
        assert_eq!(d.warn_at, 0.6);
        assert_eq!(d.pause_at, 0.8);
        assert_eq!(d.overestimate_factor, 1.5);

        let sparse_section = BudgetSection::default().set_user_daily_usd(12.0);
        let sparse = BudgetDefaults::compiled_defaults().with_section(&sparse_section);
        assert_eq!(sparse.user_daily_usd, 12.0);
        assert_eq!(sparse.heartbeat_per_tick_usd, 0.05);
        assert_eq!(sparse.default_tz, "UTC");
        assert_eq!(sparse.overestimate_factor, 1.20);
    }

    // Process env is global state; serialize env-mutating tests behind a
    // single mutex so they cannot race each other. The lock is unpoisoned
    // before each test scope so a panic in one test does not cascade.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// RAII guard that sets an env var on construction and restores the
    /// previous value (or removes the var) on drop, even on test panic.
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = env::var(key).ok();
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                env::set_var(key, value);
            }
            Self { key, prior }
        }

        fn unset(key: &'static str) -> Self {
            let prior = env::var(key).ok();
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                env::remove_var(key);
            }
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                match &self.prior {
                    Some(v) => env::set_var(self.key, v),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn env_layer_overrides_section_and_rejects_invalid_f64() {
        let _lock = env_lock();
        // Establish a section override that env should win against.
        let section = BudgetSection::default()
            .set_user_daily_usd(10.0)
            .set_warn_at(0.60)
            .set_pause_at(0.80);

        // Valid env override path.
        {
            let _g1 = EnvGuard::set(USER_DAILY_USD_ENV, "42.5");
            let _g2 = EnvGuard::set(BUDGET_WARN_AT_ENV, "0.50");
            let _g3 = EnvGuard::set(BUDGET_PAUSE_AT_ENV, "0.75");
            let _g4 = EnvGuard::unset(BUDGET_OVERESTIMATE_FACTOR_ENV);
            let d = BudgetDefaults::compiled_defaults()
                .with_section(&section)
                .with_env()
                .expect("env layer must apply over section");
            assert_eq!(d.user_daily_usd, 42.5);
            assert_eq!(d.warn_at, 0.50);
            assert_eq!(d.pause_at, 0.75);
            // Untouched env preserves prior section value.
            assert!((d.overestimate_factor - 1.20).abs() < f64::EPSILON);
        }

        // Malformed env value is surfaced as InvalidEnvF64, not silently dropped.
        {
            let _g = EnvGuard::set(USER_DAILY_USD_ENV, "not-a-number");
            let err = BudgetDefaults::compiled_defaults()
                .with_env()
                .expect_err("non-numeric env override must error");
            match err {
                BudgetDefaultsError::InvalidEnvF64 { name, value } => {
                    assert_eq!(name, USER_DAILY_USD_ENV);
                    assert_eq!(value, "not-a-number");
                }
                other => panic!("expected InvalidEnvF64, got {other:?}"),
            }
        }
    }
}
