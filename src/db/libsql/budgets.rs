//! libSQL implementation of [`BudgetStore`].
//!
//! Atomicity model: all reserve / reconcile / release operations run
//! inside a `BEGIN IMMEDIATE` transaction so concurrent writers
//! serialize. The in-band invariant checked in SQL is
//! `spent_usd + reserved_usd + requested_usd <= limit_usd`.
//!
//! Because libSQL stores `Decimal` values as TEXT (to preserve
//! `rust_decimal` precision; see `.claude/rules/database.md`), the
//! comparison cannot be a pure-SQL `WHERE` clause — the transaction
//! reads the ledger, parses into `Decimal` in Rust, checks headroom,
//! then UPDATEs. `BEGIN IMMEDIATE` ensures no other writer can change
//! the row between the SELECT and the UPDATE.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_engine::ThreadId;
use ironclaw_engine::types::budget::{
    Budget, BudgetId, BudgetLedger, BudgetLimit, BudgetPeriod, BudgetScope, BudgetSource,
    PeriodUnit, ReservationId,
};
use ironclaw_engine::types::mission::MissionId;
use ironclaw_engine::types::project::ProjectId;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::BudgetStore;
use crate::db::libsql::{
    LibSqlBackend, fmt_ts, get_decimal, get_i64, get_opt_text, get_text, get_ts,
};
use crate::error::DatabaseError;

#[async_trait]
impl BudgetStore for LibSqlBackend {
    async fn save_budget(&self, budget: &Budget) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO budgets (
                id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                limit_wall_clock_secs, period_kind, period_tz, period_unit,
                source, active, created_at, created_by
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
            )",
            libsql::params![
                budget.id.0.to_string(),
                budget.scope.user_id().to_string(),
                budget.scope.kind_str(),
                budget.scope.scope_id(),
                budget.limit.usd.to_string(),
                budget.limit.tokens.map(|n| n as i64),
                budget.limit.wall_clock_secs.map(|n| n as i64),
                budget.period.kind_str(),
                period_tz_str(&budget.period),
                period_unit_str(&budget.period),
                budget.source.as_str(),
                if budget.active { 1i64 } else { 0i64 },
                fmt_ts(&budget.created_at),
                budget.created_by.clone(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("save_budget: {e}")))?;
        Ok(())
    }

    async fn load_budget(&self, id: BudgetId) -> Result<Option<Budget>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                    limit_wall_clock_secs, period_kind, period_tz, period_unit,
                    source, active, created_at, created_by
                 FROM budgets WHERE id = ?1",
                libsql::params![id.0.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("load_budget: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("load_budget next: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(budget_from_row(&row)?))
    }

    async fn list_active_budgets_for_scope(
        &self,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Vec<Budget>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                    limit_wall_clock_secs, period_kind, period_tz, period_unit,
                    source, active, created_at, created_by
                 FROM budgets
                 WHERE scope_kind = ?1 AND scope_id = ?2 AND active = 1",
                libsql::params![scope_kind, scope_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("list_active_budgets: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("list_active_budgets row: {e}")))?
        {
            out.push(budget_from_row(&row)?);
        }
        Ok(out)
    }

    async fn deactivate_budget(&self, id: BudgetId) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE budgets SET active = 0 WHERE id = ?1",
            libsql::params![id.0.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("deactivate_budget: {e}")))?;
        Ok(())
    }

    async fn get_or_create_ledger_for_period(
        &self,
        budget_id: BudgetId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<BudgetLedger, DatabaseError> {
        let conn = self.connect().await?;
        let start_str = fmt_ts(&period_start);
        let end_str = fmt_ts(&period_end);
        let now_str = fmt_ts(&now);

        // INSERT OR IGNORE so concurrent callers don't duplicate the row.
        conn.execute(
            "INSERT OR IGNORE INTO budget_ledgers (
                budget_id, period_start, period_end, spent_usd, reserved_usd,
                tokens_used, updated_at
            ) VALUES (?1, ?2, ?3, '0', '0', 0, ?4)",
            libsql::params![budget_id.0.to_string(), start_str.clone(), end_str, now_str],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("get_or_create_ledger insert: {e}")))?;

        let mut rows = conn
            .query(
                "SELECT budget_id, period_start, period_end, spent_usd, reserved_usd,
                    tokens_used, updated_at
                 FROM budget_ledgers
                 WHERE budget_id = ?1 AND period_start = ?2",
                libsql::params![budget_id.0.to_string(), start_str],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_or_create_ledger select: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_or_create_ledger next: {e}")))?
        else {
            return Err(DatabaseError::Query(
                "get_or_create_ledger: row disappeared after INSERT OR IGNORE".into(),
            ));
        };
        ledger_from_row(&row)
    }

    async fn reserve_atomic(
        &self,
        budget_id: BudgetId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        requested_usd: Decimal,
        requested_tokens: u64,
        limit_usd: Decimal,
        now: DateTime<Utc>,
    ) -> Result<Option<(ReservationId, BudgetLedger)>, DatabaseError> {
        if requested_usd.is_sign_negative() {
            return Err(DatabaseError::Query(
                "reserve_atomic: requested_usd must be non-negative".into(),
            ));
        }

        let conn = self.connect().await?;
        let start_str = fmt_ts(&period_start);
        let end_str = fmt_ts(&period_end);
        let now_str = fmt_ts(&now);
        let budget_id_str = budget_id.0.to_string();

        // Ensure ledger row exists BEFORE the transaction so concurrent
        // callers don't race on INSERT OR IGNORE inside the critical
        // section. `BEGIN IMMEDIATE` starts a write lock immediately.
        conn.execute(
            "INSERT OR IGNORE INTO budget_ledgers (
                budget_id, period_start, period_end, spent_usd, reserved_usd,
                tokens_used, updated_at
            ) VALUES (?1, ?2, ?3, '0', '0', 0, ?4)",
            libsql::params![
                budget_id_str.clone(),
                start_str.clone(),
                end_str,
                now_str.clone()
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("reserve_atomic seed ledger: {e}")))?;

        // Begin the critical section. `IMMEDIATE` acquires a reserved
        // lock on the database file — any concurrent `BEGIN IMMEDIATE`
        // caller waits on the busy_timeout (5s by default) rather than
        // racing.
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(format!("reserve_atomic BEGIN: {e}")))?;

        let outcome = reserve_within_tx(
            &conn,
            &budget_id_str,
            &start_str,
            requested_usd,
            requested_tokens,
            limit_usd,
            &now_str,
        )
        .await;

        match outcome {
            Ok(result) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(format!("reserve_atomic COMMIT: {e}")))?;
                Ok(result)
            }
            Err(e) => {
                // Best-effort rollback; ignore the result because the
                // original error is what the caller needs to see.
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn reconcile_reservation(
        &self,
        _reservation_id: ReservationId,
        budget_id: BudgetId,
        period_start: DateTime<Utc>,
        original_reserved_usd: Decimal,
        actual_usd: Decimal,
        actual_tokens: u64,
        now: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        // Contract: atomically decrement reserved_usd by the original
        // pre-flight amount (clamped at 0 to guard against pathological
        // double-reconciles) and increment spent_usd by the actual.
        if actual_usd.is_sign_negative() || original_reserved_usd.is_sign_negative() {
            return Err(DatabaseError::Query(
                "reconcile_reservation: amounts must be non-negative".into(),
            ));
        }

        let conn = self.connect().await?;
        let budget_id_str = budget_id.0.to_string();
        let start_str = fmt_ts(&period_start);
        let now_str = fmt_ts(&now);

        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(format!("reconcile BEGIN: {e}")))?;

        let res = async {
            let ledger = read_ledger(&conn, &budget_id_str, &start_str).await?;
            let reserved_new = if original_reserved_usd > ledger.reserved_usd {
                Decimal::ZERO
            } else {
                ledger.reserved_usd - original_reserved_usd
            };
            let spent_new = ledger.spent_usd + actual_usd;
            let tokens_new = ledger.tokens_used + actual_tokens;

            conn.execute(
                "UPDATE budget_ledgers
                 SET spent_usd = ?1, reserved_usd = ?2, tokens_used = ?3,
                     updated_at = ?4
                 WHERE budget_id = ?5 AND period_start = ?6",
                libsql::params![
                    spent_new.to_string(),
                    reserved_new.to_string(),
                    tokens_new as i64,
                    now_str.clone(),
                    budget_id_str.clone(),
                    start_str.clone(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("reconcile UPDATE: {e}")))?;
            Ok::<(), DatabaseError>(())
        }
        .await;

        match res {
            Ok(_) => conn
                .execute("COMMIT", ())
                .await
                .map_err(|e| DatabaseError::Query(format!("reconcile COMMIT: {e}")))
                .map(|_| ()),
            Err(e) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn release_reservation(
        &self,
        reservation_id: ReservationId,
        budget_id: BudgetId,
        period_start: DateTime<Utc>,
        original_reserved_usd: Decimal,
        now: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        // A release is reconcile with zero actual spend.
        self.reconcile_reservation(
            reservation_id,
            budget_id,
            period_start,
            original_reserved_usd,
            Decimal::ZERO,
            0,
            now,
        )
        .await
    }

    async fn record_budget_event(
        &self,
        id: Uuid,
        budget_id: BudgetId,
        thread_id: Option<ThreadId>,
        event_kind: &str,
        amount_usd: Option<Decimal>,
        tokens: Option<u64>,
        reason: Option<&str>,
        actor_user_id: &str,
        created_at: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO budget_events (
                id, budget_id, thread_id, event_kind, amount_usd, tokens,
                reason, actor_user_id, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            libsql::params![
                id.to_string(),
                budget_id.0.to_string(),
                thread_id.map(|t| t.0.to_string()),
                event_kind,
                amount_usd.map(|d| d.to_string()),
                tokens.map(|n| n as i64),
                reason,
                actor_user_id,
                fmt_ts(&created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("record_budget_event: {e}")))?;
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────

async fn reserve_within_tx(
    conn: &libsql::Connection,
    budget_id_str: &str,
    start_str: &str,
    requested_usd: Decimal,
    requested_tokens: u64,
    limit_usd: Decimal,
    now_str: &str,
) -> Result<Option<(ReservationId, BudgetLedger)>, DatabaseError> {
    let ledger = read_ledger(conn, budget_id_str, start_str).await?;
    let committed = ledger.spent_usd + ledger.reserved_usd;
    if committed + requested_usd > limit_usd {
        return Ok(None);
    }
    let reserved_new = ledger.reserved_usd + requested_usd;

    conn.execute(
        "UPDATE budget_ledgers
         SET reserved_usd = ?1, updated_at = ?2
         WHERE budget_id = ?3 AND period_start = ?4",
        libsql::params![reserved_new.to_string(), now_str, budget_id_str, start_str],
    )
    .await
    .map_err(|e| DatabaseError::Query(format!("reserve UPDATE: {e}")))?;

    let updated = BudgetLedger {
        reserved_usd: reserved_new,
        tokens_used: ledger.tokens_used + requested_tokens,
        updated_at: chrono::DateTime::parse_from_rfc3339(now_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(ledger.updated_at),
        ..ledger
    };

    Ok(Some((ReservationId::new(), updated)))
}

async fn read_ledger(
    conn: &libsql::Connection,
    budget_id_str: &str,
    start_str: &str,
) -> Result<BudgetLedger, DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT budget_id, period_start, period_end, spent_usd, reserved_usd,
                tokens_used, updated_at
             FROM budget_ledgers
             WHERE budget_id = ?1 AND period_start = ?2",
            libsql::params![budget_id_str, start_str],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("read_ledger query: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("read_ledger row: {e}")))?
        .ok_or_else(|| {
            DatabaseError::Query(format!(
                "read_ledger: no ledger for budget {budget_id_str} at period {start_str}"
            ))
        })?;
    ledger_from_row(&row)
}

fn ledger_from_row(row: &libsql::Row) -> Result<BudgetLedger, DatabaseError> {
    let budget_id_str = get_text(row, 0);
    let budget_id = Uuid::parse_str(&budget_id_str)
        .map(BudgetId)
        .map_err(|e| DatabaseError::Query(format!("budget_id parse: {e}")))?;
    Ok(BudgetLedger {
        budget_id,
        period_start: get_ts(row, 1),
        period_end: get_ts(row, 2),
        spent_usd: get_decimal(row, 3),
        reserved_usd: get_decimal(row, 4),
        tokens_used: get_i64(row, 5) as u64,
        updated_at: get_ts(row, 6),
    })
}

fn budget_from_row(row: &libsql::Row) -> Result<Budget, DatabaseError> {
    let id = Uuid::parse_str(&get_text(row, 0))
        .map(BudgetId)
        .map_err(|e| DatabaseError::Query(format!("budget id parse: {e}")))?;
    let user_id = get_text(row, 1);
    let kind = get_text(row, 2);
    let scope_id = get_text(row, 3);
    let scope = rehydrate_scope(&kind, &scope_id, user_id.clone())?;
    let limit = BudgetLimit {
        usd: get_decimal(row, 4),
        tokens: row.get::<i64>(5).ok().map(|n| n as u64),
        wall_clock_secs: row.get::<i64>(6).ok().map(|n| n as u64),
    };
    let period_kind = get_text(row, 7);
    let period_tz = get_opt_text(row, 8);
    let period_unit = get_opt_text(row, 9);
    let period = rehydrate_period(&period_kind, period_tz, period_unit)?;
    let source = match get_text(row, 10).as_str() {
        "user_override" => BudgetSource::UserOverride,
        "inherited" => BudgetSource::InheritedFromParent,
        _ => BudgetSource::Default,
    };
    Ok(Budget {
        id,
        scope,
        limit,
        period,
        source,
        active: get_i64(row, 11) != 0,
        created_at: get_ts(row, 12),
        created_by: get_text(row, 13),
    })
}

fn rehydrate_scope(
    kind: &str,
    scope_id: &str,
    user_id: String,
) -> Result<BudgetScope, DatabaseError> {
    Ok(match kind {
        "user" => BudgetScope::User { user_id },
        "project" => {
            let project_uuid = Uuid::parse_str(scope_id)
                .map_err(|e| DatabaseError::Query(format!("project scope id: {e}")))?;
            BudgetScope::Project {
                user_id,
                project_id: ProjectId(project_uuid),
            }
        }
        "mission" => {
            let mission_uuid = Uuid::parse_str(scope_id)
                .map_err(|e| DatabaseError::Query(format!("mission scope id: {e}")))?;
            BudgetScope::Mission {
                user_id,
                mission_id: MissionId(mission_uuid),
            }
        }
        "thread" => {
            let thread_uuid = Uuid::parse_str(scope_id)
                .map_err(|e| DatabaseError::Query(format!("thread scope id: {e}")))?;
            BudgetScope::Thread {
                user_id,
                thread_id: ThreadId(thread_uuid),
            }
        }
        "background" => {
            // Format: "<kind>:<correlation_id>"
            let (kind_str, corr) = scope_id
                .split_once(':')
                .ok_or_else(|| DatabaseError::Query("background scope malformed".into()))?;
            let bk = match kind_str {
                "heartbeat" => ironclaw_engine::types::budget::BackgroundKind::Heartbeat,
                "routine_lightweight" => {
                    ironclaw_engine::types::budget::BackgroundKind::RoutineLightweight
                }
                "routine_standard" => {
                    ironclaw_engine::types::budget::BackgroundKind::RoutineStandard
                }
                "mission_tick" => ironclaw_engine::types::budget::BackgroundKind::MissionTick,
                "container_job" => ironclaw_engine::types::budget::BackgroundKind::ContainerJob,
                "user_initiated" => ironclaw_engine::types::budget::BackgroundKind::UserInitiated,
                other => {
                    return Err(DatabaseError::Query(format!(
                        "unknown background kind '{other}'"
                    )));
                }
            };
            BudgetScope::BackgroundInvocation {
                user_id,
                kind: bk,
                correlation_id: corr.to_string(),
            }
        }
        other => {
            return Err(DatabaseError::Query(format!(
                "unknown scope_kind '{other}'"
            )));
        }
    })
}

fn rehydrate_period(
    kind: &str,
    tz: Option<String>,
    unit: Option<String>,
) -> Result<BudgetPeriod, DatabaseError> {
    Ok(match kind {
        "per_invocation" => BudgetPeriod::PerInvocation,
        "rolling_24h" => BudgetPeriod::Rolling24h,
        "calendar" => {
            let tz = tz.ok_or_else(|| DatabaseError::Query("calendar period missing tz".into()))?;
            let unit = match unit
                .as_deref()
                .ok_or_else(|| DatabaseError::Query("calendar period missing unit".into()))?
            {
                "day" => PeriodUnit::Day,
                "week" => PeriodUnit::Week,
                "month" => PeriodUnit::Month,
                other => {
                    return Err(DatabaseError::Query(format!(
                        "unknown period_unit '{other}'"
                    )));
                }
            };
            BudgetPeriod::Calendar { tz, unit }
        }
        other => {
            return Err(DatabaseError::Query(format!(
                "unknown period_kind '{other}'"
            )));
        }
    })
}

fn period_tz_str(period: &BudgetPeriod) -> Option<String> {
    match period {
        BudgetPeriod::Calendar { tz, .. } => Some(tz.clone()),
        _ => None,
    }
}

fn period_unit_str(period: &BudgetPeriod) -> Option<&'static str> {
    match period {
        BudgetPeriod::Calendar { unit, .. } => Some(unit.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;
    use std::sync::Arc;
    use tokio::task::JoinSet;

    use crate::db::Database;

    /// Helper to spin up a backend with V25 migration applied.
    async fn make_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budgets_test.db");
        let backend = LibSqlBackend::new_local(&path).await.unwrap();
        backend.run_migrations().await.unwrap();
        (backend, dir)
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn alice_user_budget(limit: Decimal) -> Budget {
        Budget {
            id: BudgetId::new(),
            scope: BudgetScope::User {
                user_id: "alice".into(),
            },
            limit: BudgetLimit {
                usd: limit,
                tokens: None,
                wall_clock_secs: None,
            },
            period: BudgetPeriod::Rolling24h,
            source: BudgetSource::Default,
            active: true,
            created_at: ts(1_700_000_000),
            created_by: "alice".into(),
        }
    }

    #[tokio::test]
    async fn save_then_load_round_trips_a_budget() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(5.00));
        be.save_budget(&budget).await.unwrap();
        let loaded = be.load_budget(budget.id).await.unwrap().unwrap();
        assert_eq!(loaded.id, budget.id);
        assert_eq!(loaded.limit.usd, dec!(5.00));
        assert_eq!(loaded.scope.user_id(), "alice");
        assert!(loaded.active);
    }

    #[tokio::test]
    async fn list_active_returns_only_active_rows() {
        let (be, _d) = make_backend().await;
        let b1 = alice_user_budget(dec!(5.00));
        let b2_id = {
            let mut b2 = alice_user_budget(dec!(3.00));
            // distinct period to avoid the unique constraint
            b2.period = BudgetPeriod::Calendar {
                tz: "UTC".into(),
                unit: PeriodUnit::Day,
            };
            let id = b2.id;
            be.save_budget(&b2).await.unwrap();
            id
        };
        be.save_budget(&b1).await.unwrap();
        be.deactivate_budget(b1.id).await.unwrap();

        let active = be
            .list_active_budgets_for_scope("user", "alice")
            .await
            .unwrap();
        let ids: Vec<BudgetId> = active.iter().map(|b| b.id).collect();
        assert!(ids.contains(&b2_id));
        assert!(!ids.contains(&b1.id));
    }

    #[tokio::test]
    async fn reserve_denies_when_exceeding_limit() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(1.00));
        be.save_budget(&budget).await.unwrap();

        let start = ts(1_700_000_000);
        let end = ts(1_700_086_400);

        let ok = be
            .reserve_atomic(
                budget.id,
                start,
                end,
                dec!(0.80),
                100,
                dec!(1.00),
                ts(1_700_000_001),
            )
            .await
            .unwrap();
        assert!(ok.is_some());

        let denied = be
            .reserve_atomic(
                budget.id,
                start,
                end,
                dec!(0.30),
                0,
                dec!(1.00),
                ts(1_700_000_002),
            )
            .await
            .unwrap();
        assert!(denied.is_none(), "should have denied but got reservation");
    }

    #[tokio::test]
    async fn reconcile_settles_reservation_to_spent() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(5.00));
        be.save_budget(&budget).await.unwrap();

        let start = ts(1_700_000_000);
        let end = ts(1_700_086_400);

        let (rid, _) = be
            .reserve_atomic(
                budget.id,
                start,
                end,
                dec!(0.50),
                1000,
                dec!(5.00),
                ts(1_700_000_010),
            )
            .await
            .unwrap()
            .unwrap();

        // Original reservation was $0.50; actual spend was $0.20.
        be.reconcile_reservation(
            rid,
            budget.id,
            start,
            dec!(0.50),
            dec!(0.20),
            500,
            ts(1_700_000_020),
        )
        .await
        .unwrap();

        let ledger = be
            .get_or_create_ledger_for_period(budget.id, start, end, ts(1_700_000_030))
            .await
            .unwrap();
        assert_eq!(ledger.spent_usd, dec!(0.20));
        // reserve(0.50) slot fully cleared on reconcile — reserved_usd
        // is back to zero.
        assert_eq!(ledger.reserved_usd, Decimal::ZERO);
        // tokens_used accumulates only on reconcile (pre-flight
        // `requested_tokens` on reserve is an estimate the schema does
        // not persist).
        assert_eq!(ledger.tokens_used, 500);
    }

    #[tokio::test]
    async fn release_zeroes_reservation_without_recording_spend() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(5.00));
        be.save_budget(&budget).await.unwrap();

        let start = ts(1_700_000_000);
        let end = ts(1_700_086_400);

        let (rid, _) = be
            .reserve_atomic(
                budget.id,
                start,
                end,
                dec!(0.50),
                1000,
                dec!(5.00),
                ts(1_700_000_010),
            )
            .await
            .unwrap()
            .unwrap();

        be.release_reservation(rid, budget.id, start, dec!(0.50), ts(1_700_000_020))
            .await
            .unwrap();

        let ledger = be
            .get_or_create_ledger_for_period(budget.id, start, end, ts(1_700_000_030))
            .await
            .unwrap();
        assert_eq!(ledger.spent_usd, Decimal::ZERO);
        // Release(0.50) zeroed the reservation.
        assert_eq!(ledger.reserved_usd, Decimal::ZERO);
    }

    /// Core concurrency guarantee: N tasks each trying to reserve the
    /// same dollar against a limit that only permits exactly K of them
    /// must see exactly K succeed.
    ///
    /// If this test ever flakes, the atomic-reservation invariant is
    /// broken and threads will spend past budget.
    #[tokio::test]
    async fn concurrent_reservations_never_oversubscribe() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(5.00));
        be.save_budget(&budget).await.unwrap();
        let backend = Arc::new(be);

        let start = ts(1_700_000_000);
        let end = ts(1_700_086_400);
        let each = dec!(0.10);
        let limit = dec!(5.00);
        let n: u32 = 100;
        // 5.00 / 0.10 = 50 should succeed, 50 should deny.

        let mut set = JoinSet::new();
        for i in 0..n {
            let backend = Arc::clone(&backend);
            let budget_id = budget.id;
            set.spawn(async move {
                backend
                    .reserve_atomic(
                        budget_id,
                        start,
                        end,
                        each,
                        0,
                        limit,
                        ts(1_700_000_000 + i as i64),
                    )
                    .await
            });
        }

        let mut granted = 0;
        let mut denied = 0;
        while let Some(res) = set.join_next().await {
            match res.unwrap().unwrap() {
                Some(_) => granted += 1,
                None => denied += 1,
            }
        }

        assert_eq!(
            granted, 50,
            "expected 50 granted reservations, got {granted} (denied={denied})"
        );
        assert_eq!(denied, 50, "expected 50 denials, got {denied}");

        // Sanity: ledger reserved_usd equals 50 * 0.10 = 5.00 exactly.
        let ledger = backend
            .get_or_create_ledger_for_period(budget.id, start, end, ts(1_700_000_999))
            .await
            .unwrap();
        assert_eq!(ledger.reserved_usd, dec!(5.00));
        assert_eq!(ledger.spent_usd, Decimal::ZERO);
    }

    #[tokio::test]
    async fn record_budget_event_appends_audit_row() {
        let (be, _d) = make_backend().await;
        let budget = alice_user_budget(dec!(5.00));
        be.save_budget(&budget).await.unwrap();

        be.record_budget_event(
            Uuid::new_v4(),
            budget.id,
            None,
            "reserve",
            Some(dec!(0.10)),
            Some(100),
            Some("cascade reserve"),
            "alice",
            ts(1_700_000_000),
        )
        .await
        .unwrap();

        let conn = be.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM budget_events WHERE budget_id = ?1",
                libsql::params![budget.id.0.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 1);
    }
}
