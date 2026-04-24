//! Cost-based budget defaults and enforcement mode.
//!
//! Loaded once at startup. The per-scope `Budget` rows in the database
//! override these defaults for any specific user/project/mission/thread
//! that has its own row; this struct only answers "what should the default
//! be when a scope has no row yet?"
//!
//! Env-only — budget limits are security-sensitive (raising a user's daily
//! cap is a privilege-changing operation), so per the top-level `config/mod.rs`
//! policy, they do NOT fall through to DB/TOML settings. Per-user overrides
//! live in the `budgets` table and go through the audited `budget_increase`
//! tool.

use std::str::FromStr;

use ironclaw_common::{HARD_CAP_BUDGET_USD_STR, HARD_CAP_ITERATIONS, HARD_CAP_WALL_CLOCK_SECS};
use rust_decimal::Decimal;

use crate::config::helpers::{optional_env, parse_optional_env, parse_string_env};

#[cfg(test)]
use crate::config::helpers::parse_bool_env;
use crate::error::ConfigError;

/// Enforcement mode for the cost-based budget system.
///
/// Lets us roll out the full rework in phases without flipping every
/// production user's bill at once (see issue #2843 "Migration & Rollout").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetEnforcementMode {
    /// Enforcer records nothing, old iteration/time caps are authoritative.
    /// This is the on-release default — existing deployments upgrade
    /// without behavior change.
    Off,
    /// Enforcer records reservations and denials but never blocks. Used to
    /// calibrate default budgets against real traffic in staging.
    Shadow,
    /// Enforcer denies at 100% only; the 90% approval gate is a soft warning.
    Warn,
    /// Full enforcement: warn at 75%, approval gate at 90%, hard stop at 100%.
    Enforce,
}

impl FromStr for BudgetEnforcementMode {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, ConfigError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "shadow" => Ok(Self::Shadow),
            "warn" => Ok(Self::Warn),
            "enforce" => Ok(Self::Enforce),
            other => Err(ConfigError::InvalidValue {
                key: "BUDGET_ENFORCEMENT_MODE".into(),
                message: format!(
                    "expected one of 'off' | 'shadow' | 'warn' | 'enforce', got '{other}'"
                ),
            }),
        }
    }
}

impl BudgetEnforcementMode {
    pub fn is_recording(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn is_denying(self) -> bool {
        matches!(self, Self::Warn | Self::Enforce)
    }

    pub fn is_gating(self) -> bool {
        matches!(self, Self::Enforce)
    }
}

/// Cost-based budgeting configuration.
///
/// Every per-scope default is a USD `Decimal`. Tokens and wall-clock
/// defaults live on the invariant constants — they're backstops, not
/// per-scope knobs, so we don't expose them through env.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    pub mode: BudgetEnforcementMode,

    /// Default daily USD cap applied to a user who has no override row.
    pub user_daily_usd: Decimal,
    /// Default daily USD cap for a project.
    pub project_daily_usd: Decimal,
    /// Default per-tick USD budget for a mission that didn't set its own.
    pub mission_per_tick_usd: Decimal,
    /// Default per-tick USD budget for the heartbeat.
    pub heartbeat_per_tick_usd: Decimal,
    /// Default budget for a lightweight routine fire.
    pub routine_lightweight_usd: Decimal,
    /// Default budget for a standard routine fire.
    pub routine_standard_usd: Decimal,
    /// Default per-job USD budget.
    pub job_default_usd: Decimal,

    /// Utilization fraction at which a `Warn` tier warning is emitted
    /// (0.0–1.0). Default 0.75.
    pub warn_threshold: f64,
    /// Utilization fraction at which an approval gate is inserted
    /// (0.0–1.0). Default 0.90. Must be strictly greater than
    /// `warn_threshold`.
    pub approval_threshold: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        // These strings are guaranteed to parse — covered by the
        // `defaults_parse_cleanly` unit test.
        Self {
            mode: BudgetEnforcementMode::Off,
            user_daily_usd: Decimal::from_str("5.00").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            project_daily_usd: Decimal::from_str("2.00").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            mission_per_tick_usd: Decimal::from_str("0.50").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            heartbeat_per_tick_usd: Decimal::from_str("0.05").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            routine_lightweight_usd: Decimal::from_str("0.02").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            routine_standard_usd: Decimal::from_str("0.10").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            job_default_usd: Decimal::from_str("1.00").expect("hardcoded default parses"), // safety: literal, covered by defaults_parse_cleanly
            warn_threshold: 0.75,
            approval_threshold: 0.90,
        }
    }
}

impl BudgetConfig {
    /// Resolve from env vars, returning `ConfigError::InvalidValue` if any
    /// value violates the hard-cap invariants.
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let defaults = Self::default();

        let mode = match optional_env("BUDGET_ENFORCEMENT_MODE")? {
            Some(s) => BudgetEnforcementMode::from_str(&s)?,
            None => defaults.mode,
        };

        let cfg = Self {
            mode,
            user_daily_usd: read_decimal_env("BUDGET_USER_DAILY_USD", defaults.user_daily_usd)?,
            project_daily_usd: read_decimal_env(
                "BUDGET_PROJECT_DAILY_USD",
                defaults.project_daily_usd,
            )?,
            mission_per_tick_usd: read_decimal_env(
                "BUDGET_MISSION_PER_TICK_USD",
                defaults.mission_per_tick_usd,
            )?,
            heartbeat_per_tick_usd: read_decimal_env(
                "BUDGET_HEARTBEAT_PER_TICK_USD",
                defaults.heartbeat_per_tick_usd,
            )?,
            routine_lightweight_usd: read_decimal_env(
                "BUDGET_ROUTINE_LIGHTWEIGHT_USD",
                defaults.routine_lightweight_usd,
            )?,
            routine_standard_usd: read_decimal_env(
                "BUDGET_ROUTINE_STANDARD_USD",
                defaults.routine_standard_usd,
            )?,
            job_default_usd: read_decimal_env("BUDGET_JOB_DEFAULT_USD", defaults.job_default_usd)?,
            warn_threshold: parse_optional_env("BUDGET_WARN_THRESHOLD", defaults.warn_threshold)?,
            approval_threshold: parse_optional_env(
                "BUDGET_APPROVAL_THRESHOLD",
                defaults.approval_threshold,
            )?,
        };

        cfg.validate()?;
        Ok(cfg)
    }

    /// Hard-cap invariant + threshold sanity checks.
    ///
    /// Fails at startup if any per-scope USD default exceeds the absolute
    /// `HARD_CAP_BUDGET_USD` invariant, or if the warn/approval thresholds
    /// are inverted or out of `(0.0, 1.0]`.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let hard_cap =
            Decimal::from_str(HARD_CAP_BUDGET_USD_STR).map_err(|e| ConfigError::InvalidValue {
                key: "HARD_CAP_BUDGET_USD_STR".into(),
                message: format!(
                    "internal invariant constant did not parse — {e}. This is a code bug."
                ),
            })?;

        let checks: [(&str, Decimal); 7] = [
            ("BUDGET_USER_DAILY_USD", self.user_daily_usd),
            ("BUDGET_PROJECT_DAILY_USD", self.project_daily_usd),
            ("BUDGET_MISSION_PER_TICK_USD", self.mission_per_tick_usd),
            ("BUDGET_HEARTBEAT_PER_TICK_USD", self.heartbeat_per_tick_usd),
            (
                "BUDGET_ROUTINE_LIGHTWEIGHT_USD",
                self.routine_lightweight_usd,
            ),
            ("BUDGET_ROUTINE_STANDARD_USD", self.routine_standard_usd),
            ("BUDGET_JOB_DEFAULT_USD", self.job_default_usd),
        ];
        for (key, val) in checks {
            if val.is_sign_negative() {
                return Err(ConfigError::InvalidValue {
                    key: key.into(),
                    message: format!("must be >= 0, got {val}"),
                });
            }
            if val > hard_cap {
                return Err(ConfigError::InvalidValue {
                    key: key.into(),
                    message: format!(
                        "exceeds HARD_CAP_BUDGET_USD ({HARD_CAP_BUDGET_USD_STR}): got {val}"
                    ),
                });
            }
        }

        // Accept 0.0 so an operator can dial warnings down to "warn on
        // any spend" during calibration; the upper bound stays strict
        // because >=1.0 would fire the warn tier at the same point as
        // the exhausted-USD denial.
        if !(0.0..1.0).contains(&self.warn_threshold) {
            return Err(ConfigError::InvalidValue {
                key: "BUDGET_WARN_THRESHOLD".into(),
                message: format!(
                    "must be in [0.0, 1.0), got {val}",
                    val = self.warn_threshold
                ),
            });
        }
        if !(0.0 < self.approval_threshold && self.approval_threshold <= 1.0) {
            return Err(ConfigError::InvalidValue {
                key: "BUDGET_APPROVAL_THRESHOLD".into(),
                message: format!(
                    "must be in (0.0, 1.0], got {val}",
                    val = self.approval_threshold
                ),
            });
        }
        if self.approval_threshold <= self.warn_threshold {
            return Err(ConfigError::InvalidValue {
                key: "BUDGET_APPROVAL_THRESHOLD".into(),
                message: format!(
                    "must be strictly greater than BUDGET_WARN_THRESHOLD ({}), got {}",
                    self.warn_threshold, self.approval_threshold
                ),
            });
        }

        Ok(())
    }

    /// The absolute iteration backstop. Not environment-tunable — this is
    /// an invariant, documented here so callers have one import site.
    pub fn hard_cap_iterations() -> usize {
        HARD_CAP_ITERATIONS
    }

    /// The absolute wall-clock backstop in seconds.
    pub fn hard_cap_wall_clock_secs() -> u64 {
        HARD_CAP_WALL_CLOCK_SECS
    }
}

fn read_decimal_env(key: &str, default: Decimal) -> Result<Decimal, ConfigError> {
    let raw = parse_string_env(key, default.to_string())?;
    Decimal::from_str(raw.trim()).map_err(|e| ConfigError::InvalidValue {
        key: key.into(),
        message: format!("invalid decimal '{raw}': {e}"),
    })
}

/// Only used so `parse_bool_env` is reachable via `cargo check --no-default-features
/// --features libsql` — `BudgetConfig` currently has no bool knobs but the
/// helper is the normal pattern and wiring it in now avoids a later import
/// churn when `BUDGET_DRY_RUN`-style toggles get added. Marked `#[cfg(test)]`
/// because we don't want dead-code warnings in production.
#[cfg(test)]
#[allow(dead_code)]
fn _touch_bool_helper(key: &str) -> Result<bool, ConfigError> {
    parse_bool_env(key, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn defaults_parse_cleanly() {
        let cfg = BudgetConfig::default();
        assert_eq!(cfg.user_daily_usd, dec!(5.00));
        assert_eq!(cfg.project_daily_usd, dec!(2.00));
        assert_eq!(cfg.mission_per_tick_usd, dec!(0.50));
        assert_eq!(cfg.heartbeat_per_tick_usd, dec!(0.05));
        assert_eq!(cfg.routine_lightweight_usd, dec!(0.02));
        assert_eq!(cfg.routine_standard_usd, dec!(0.10));
        assert_eq!(cfg.job_default_usd, dec!(1.00));
        assert_eq!(cfg.warn_threshold, 0.75);
        assert_eq!(cfg.approval_threshold, 0.90);
        assert_eq!(cfg.mode, BudgetEnforcementMode::Off);
        cfg.validate().expect("defaults must validate");
    }

    #[test]
    fn mode_parses_case_insensitive() {
        assert_eq!(
            BudgetEnforcementMode::from_str("Shadow").unwrap(),
            BudgetEnforcementMode::Shadow
        );
        assert_eq!(
            BudgetEnforcementMode::from_str("ENFORCE").unwrap(),
            BudgetEnforcementMode::Enforce
        );
        assert!(BudgetEnforcementMode::from_str("xyz").is_err());
    }

    #[test]
    fn mode_predicates() {
        assert!(!BudgetEnforcementMode::Off.is_recording());
        assert!(BudgetEnforcementMode::Shadow.is_recording());
        assert!(!BudgetEnforcementMode::Shadow.is_denying());
        assert!(BudgetEnforcementMode::Warn.is_denying());
        assert!(!BudgetEnforcementMode::Warn.is_gating());
        assert!(BudgetEnforcementMode::Enforce.is_gating());
    }

    #[test]
    fn validate_rejects_negative_budget() {
        let cfg = BudgetConfig {
            user_daily_usd: dec!(-1),
            ..BudgetConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("BUDGET_USER_DAILY_USD"));
    }

    #[test]
    fn validate_rejects_budget_above_hard_cap() {
        let cfg = BudgetConfig {
            project_daily_usd: dec!(9999),
            ..BudgetConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("HARD_CAP_BUDGET_USD"));
    }

    #[test]
    fn validate_rejects_inverted_thresholds() {
        let cfg = BudgetConfig {
            warn_threshold: 0.95,
            approval_threshold: 0.90,
            ..BudgetConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("BUDGET_APPROVAL_THRESHOLD"));
    }

    #[test]
    fn validate_rejects_out_of_range_thresholds() {
        let cfg = BudgetConfig {
            warn_threshold: 1.5,
            approval_threshold: 2.0,
            ..BudgetConfig::default()
        };
        assert!(cfg.validate().is_err());

        // Negative warn threshold stays rejected; 0.0 itself is now
        // allowed (see below).
        let cfg = BudgetConfig {
            warn_threshold: -0.1,
            ..BudgetConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_allows_zero_warn_threshold() {
        // "Warn on any spend" is a legitimate operator setting during
        // calibration, so warn_threshold = 0.0 must not be rejected.
        let cfg = BudgetConfig {
            warn_threshold: 0.0,
            ..BudgetConfig::default()
        };
        cfg.validate()
            .expect("warn_threshold = 0.0 should be accepted");
    }

    #[test]
    fn read_decimal_env_uses_default_when_unset() {
        let got = read_decimal_env("DEFINITELY_NOT_A_REAL_ENV_VAR_BUDGET_TEST", dec!(3.14))
            .expect("should use default");
        assert_eq!(got, dec!(3.14));
    }
}
