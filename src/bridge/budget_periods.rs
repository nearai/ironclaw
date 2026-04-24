//! Period arithmetic for budget ledgers (host side).
//!
//! Duplicates the small helper from
//! `ironclaw_engine::runtime::budget::period_bounds` because it isn't
//! re-exported and the host `HybridStore` needs it to translate between
//! the engine `Store` trait (takes `now`) and the host `BudgetStore`
//! trait (takes explicit period bounds).
//!
//! Keep the two copies in sync — the calendar/rolling semantics are
//! identical.
//!
//! **`PerInvocation` semantics:** each call produces its own
//! `(now, now + 1h)` window. By design, a single `PerInvocation`
//! budget does not share a ledger across multiple reserve calls — the
//! unit is the single costed operation. Cross-call capping requires
//! [`BudgetPeriod::Rolling24h`] or a [`BudgetPeriod::Calendar`]
//! period. See the matching docstring on
//! [`BudgetPeriod::PerInvocation`].
//!
//! **`Calendar { tz, .. }` caveat:** the engine ships without
//! `chrono-tz`, so `tz` is accepted on the wire but arithmetic below
//! anchors to UTC. Mirrors `ironclaw_engine::runtime::budget::calendar_period`.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use ironclaw_engine::types::budget::{BudgetPeriod, PeriodUnit};

pub fn period_bounds(period: &BudgetPeriod, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    match period {
        BudgetPeriod::PerInvocation => (now, now + Duration::hours(1)),
        BudgetPeriod::Rolling24h => {
            let start = quantise_utc_day(now);
            (start, start + Duration::days(1))
        }
        BudgetPeriod::Calendar { tz: _, unit } => {
            let start = match unit {
                PeriodUnit::Day => quantise_utc_day(now),
                PeriodUnit::Week => {
                    let day = now.date_naive();
                    let offset = day.weekday().num_days_from_monday() as u64;
                    let monday = day - chrono::Days::new(offset);
                    monday
                        .and_hms_opt(0, 0, 0)
                        .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
                        .unwrap_or(now)
                }
                PeriodUnit::Month => {
                    let nd = now.date_naive();
                    let first = chrono::NaiveDate::from_ymd_opt(nd.year(), nd.month(), 1)
                        .expect("first of month always valid"); // safety: year/month come from a valid NaiveDate, day=1 is always in range
                    first
                        .and_hms_opt(0, 0, 0)
                        .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
                        .unwrap_or(now)
                }
            };
            let end = match unit {
                PeriodUnit::Day => start + Duration::days(1),
                PeriodUnit::Week => start + Duration::days(7),
                PeriodUnit::Month => start + Duration::days(31),
            };
            (start, end)
        }
    }
}

fn quantise_utc_day(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
        .unwrap_or(now)
}
