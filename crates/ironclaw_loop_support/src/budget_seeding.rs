//! First-touch seeding policy consumed by
//! [`crate::GovernorBackedAccountant`].
//!
//! When composition supplies a [`BudgetSeedingPolicy`], the accountant
//! installs the bundled limits the first time it sees a particular
//! `ResourceAccount` in the cascade — but only if no limit is already
//! in place. This lets composition declare defaults once at boot
//! without forcing a "seed every user" migration; the cost of the
//! first model call by a fresh user covers the seeding write.

use ironclaw_resources::{BudgetPeriod, BudgetThresholds, ResourceLimits};
use rust_decimal::Decimal;

/// Composition-supplied first-touch seeding policy. Holds the limits
/// that get installed on first contact for the user and project
/// cascade levels.
#[derive(Debug, Clone)]
pub struct BudgetSeedingPolicy {
    pub user_daily: ResourceLimits,
    pub project_daily: ResourceLimits,
}

impl BudgetSeedingPolicy {
    /// Construct from typed defaults, expressed as `(usd, period,
    /// thresholds)`. Use `Decimal::ZERO` for unlimited per the
    /// governor's `0 = unlimited` convention.
    pub fn new(
        user_daily_usd: Decimal,
        project_daily_usd: Decimal,
        period: BudgetPeriod,
        thresholds: BudgetThresholds,
    ) -> Self {
        let user_daily = ResourceLimits {
            max_usd: Some(user_daily_usd),
            period: period.clone(),
            thresholds,
            ..ResourceLimits::default()
        };
        let project_daily = ResourceLimits {
            max_usd: Some(project_daily_usd),
            period,
            thresholds,
            ..ResourceLimits::default()
        };
        Self {
            user_daily,
            project_daily,
        }
    }
}
