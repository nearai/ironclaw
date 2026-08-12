//! Budget periodization vocabulary.
//!
//! A [`BudgetPeriod`] decides when an account's ledger resets. `PerInvocation`
//! is the v1 behavior (no accumulating ledger, holds only); `Rolling24h`
//! evicts reconciled spend older than 24h on every read; `Calendar { tz, unit }`
//! resets at the next calendar boundary in the user's timezone.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// When an account's ledger resets.
///
/// `PerInvocation` is the v1 default: no accumulating ledger, reservation holds
/// only. `Rolling24h` evicts reconciled spend older than 24h on every read;
/// `Calendar` resets at the next calendar boundary in `tz`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BudgetPeriod {
    #[default]
    PerInvocation,
    Rolling24h,
    Calendar {
        #[serde(with = "chrono_tz_serde")]
        tz: Tz,
        unit: PeriodUnit,
    },
}

/// Calendar unit for [`BudgetPeriod::Calendar`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodUnit {
    Day,
    Week,
    Month,
}

/// Graduated-intervention thresholds for a budget account.
///
/// `warn_at` and `pause_at` are utilization fractions (0.0..=1.0). Setting
/// either at or above `1.0` disables the corresponding intervention; a 100%
/// limit overrun is always a hard deny regardless.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetThresholds {
    pub warn_at: f64,
    pub pause_at: f64,
}

impl BudgetThresholds {
    /// Production defaults wired by [`crate::DEFAULT_GRADUATED_THRESHOLDS`]
    /// when configuration enables graduated intervention. Disabled here so
    /// callers must opt-in.
    pub const DISABLED: Self = Self {
        warn_at: 1.0,
        pause_at: 1.0,
    };

    /// Recommended graduated defaults (warn at 75%, pause-with-approval at
    /// 90%). Wired by [`crate::DEFAULT_GRADUATED_THRESHOLDS`] for production
    /// limits; existing tests with no thresholds set continue to use
    /// [`Self::DISABLED`].
    pub const RECOMMENDED: Self = Self {
        warn_at: 0.75,
        pause_at: 0.90,
    };

    pub fn validate(self) -> Result<(), BudgetThresholdsError> {
        if !self.warn_at.is_finite() || !self.pause_at.is_finite() {
            return Err(BudgetThresholdsError::NotFinite);
        }
        if self.warn_at < 0.0 || self.pause_at < 0.0 {
            return Err(BudgetThresholdsError::Negative);
        }
        if self.pause_at < self.warn_at {
            return Err(BudgetThresholdsError::PauseBelowWarn);
        }
        Ok(())
    }
}

impl Default for BudgetThresholds {
    fn default() -> Self {
        Self::DISABLED
    }
}

/// Validation failure for [`BudgetThresholds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetThresholdsError {
    #[error("budget thresholds must be finite")]
    NotFinite,
    #[error("budget thresholds must be non-negative")]
    Negative,
    #[error("pause_at must be greater than or equal to warn_at")]
    PauseBelowWarn,
}

/// Compute the `(period_start, period_end)` window covering `now`, where
/// `period_end` is the next instant at which the period should roll over.
///
/// Semantics by variant:
/// - [`BudgetPeriod::PerInvocation`]: `(now, DateTime::<Utc>::MAX_UTC)` —
///   never rolls over; behaves like v1 (no period concept).
/// - [`BudgetPeriod::Rolling24h`]: `(now, now + 24h)` — anchored
///   24h-from-creation window; true sliding eviction is a future
///   enhancement (this is "rolling" relative to the anchor instant).
/// - [`BudgetPeriod::Calendar { tz, unit }`]: `(local_midnight_at_or_before,
///   next_local_midnight)` in `tz`.
pub fn period_bounds(period: &BudgetPeriod, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    match period {
        BudgetPeriod::PerInvocation => (now, DateTime::<Utc>::MAX_UTC),
        BudgetPeriod::Rolling24h => (now, now + Duration::hours(24)),
        BudgetPeriod::Calendar { tz, unit } => calendar_bounds(*tz, *unit, now),
    }
}

fn calendar_bounds(tz: Tz, unit: PeriodUnit, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let local = now.with_timezone(&tz);
    let local_date = local.date_naive();

    let (start_date, end_date) = match unit {
        PeriodUnit::Day => (local_date, local_date + Duration::days(1)),
        PeriodUnit::Week => {
            let weekday = local_date.weekday();
            let days_since_monday = i64::from(weekday.num_days_from_monday());
            let week_start = local_date - Duration::days(days_since_monday);
            (week_start, week_start + Duration::days(7))
        }
        PeriodUnit::Month => {
            let month_start = NaiveDate::from_ymd_opt(local_date.year(), local_date.month(), 1)
                .unwrap_or(local_date);
            let next_month = if local_date.month() == 12 {
                NaiveDate::from_ymd_opt(local_date.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(local_date.year(), local_date.month() + 1, 1)
            }
            .unwrap_or(month_start);
            (month_start, next_month)
        }
    };

    // Convert local-midnight dates back through the tz; on DST transitions
    // pick the earliest valid instant. `from_local_datetime` returns Single
    // or Ambiguous variants; we accept the earliest.
    let start = tz
        .from_local_datetime(&start_date.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(now);
    let end = tz
        .from_local_datetime(&end_date.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(now + Duration::days(1));

    (start, end)
}

/// True when `current_end` is in the past relative to `now`, indicating the
/// ledger needs to be advanced to a fresh window.
pub fn period_has_rolled_over(current_end: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now >= current_end
}

mod chrono_tz_serde {
    use chrono_tz::Tz;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub(super) fn serialize<S: Serializer>(tz: &Tz, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(tz.name())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Tz, D::Error> {
        let raw = String::deserialize(de)?;
        Tz::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn per_invocation_end_never_rolls_over() {
        let now = Utc.with_ymd_and_hms(2026, 5, 21, 12, 30, 0).unwrap();
        let (start, end) = period_bounds(&BudgetPeriod::PerInvocation, now);
        assert_eq!(start, now);
        assert_eq!(end, DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn rolling_24h_window_extends_forward_from_now() {
        let now = Utc.with_ymd_and_hms(2026, 5, 21, 12, 30, 0).unwrap();
        let (start, end) = period_bounds(&BudgetPeriod::Rolling24h, now);
        assert_eq!(end - start, Duration::hours(24));
        assert_eq!(start, now);
    }

    #[test]
    fn calendar_day_resets_at_local_midnight() {
        let tz = chrono_tz::America::Los_Angeles;
        let now = Utc.with_ymd_and_hms(2026, 5, 21, 23, 59, 0).unwrap();
        let (start, end) = period_bounds(
            &BudgetPeriod::Calendar {
                tz,
                unit: PeriodUnit::Day,
            },
            now,
        );
        // In LA on 2026-05-21 23:59 UTC ≈ 16:59 PDT; window is 2026-05-21 to 2026-05-22 local.
        let local_start = start.with_timezone(&tz);
        let local_end = end.with_timezone(&tz);
        assert_eq!(local_start.hour(), 0);
        assert_eq!(local_end.hour(), 0);
        assert_eq!(
            local_end.date_naive() - local_start.date_naive(),
            Duration::days(1)
        );
    }

    #[test]
    fn calendar_month_handles_end_of_year_rollover() {
        let tz = chrono_tz::UTC;
        let now = Utc.with_ymd_and_hms(2026, 12, 15, 10, 0, 0).unwrap();
        let (start, end) = period_bounds(
            &BudgetPeriod::Calendar {
                tz,
                unit: PeriodUnit::Month,
            },
            now,
        );
        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 12);
        assert_eq!(end.year(), 2027);
        assert_eq!(end.month(), 1);
    }

    #[test]
    fn thresholds_validation_rejects_pause_below_warn() {
        let bad = BudgetThresholds {
            warn_at: 0.9,
            pause_at: 0.5,
        };
        assert!(matches!(
            bad.validate(),
            Err(BudgetThresholdsError::PauseBelowWarn)
        ));
    }

    #[test]
    fn thresholds_validation_rejects_negative() {
        let bad = BudgetThresholds {
            warn_at: -0.1,
            pause_at: 0.5,
        };
        assert!(matches!(
            bad.validate(),
            Err(BudgetThresholdsError::Negative)
        ));
    }

    #[test]
    fn budget_period_round_trips_through_serde() {
        let period = BudgetPeriod::Calendar {
            tz: chrono_tz::America::New_York,
            unit: PeriodUnit::Week,
        };
        let json = serde_json::to_string(&period).unwrap();
        let back: BudgetPeriod = serde_json::from_str(&json).unwrap();
        assert_eq!(period, back);
    }

    #[test]
    fn rolling_24h_round_trips_through_serde() {
        let period = BudgetPeriod::Rolling24h;
        let json = serde_json::to_string(&period).unwrap();
        let back: BudgetPeriod = serde_json::from_str(&json).unwrap();
        assert_eq!(period, back);
    }

    use chrono::Timelike;
}
