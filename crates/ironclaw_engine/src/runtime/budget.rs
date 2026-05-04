//! Cost-based budget enforcement — the runtime side of issue #2843.
//!
//! The enforcer is the single chokepoint every costed operation goes
//! through. Call sites:
//!
//! - `ExecutionLoop::step` before every LLM call (reserve →
//!   `LlmBackend::complete` → reconcile).
//! - Background schedulers (heartbeat, routines, missions, jobs)
//!   before dispatching a tick.
//!
//! The enforcer walks the cascade in order — user → project → mission
//! → thread → background — and calls `Store::reserve_atomic` on each.
//! A denial at any level aborts the reservation and releases every
//! scope that had already been granted above it.
//!
//! USD is the primary dimension. Token caps and wall-clock caps are
//! optional secondary backstops; iteration count is not a budget
//! dimension at all.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;

use crate::traits::store::{AtomicReserveOutcome, BudgetEventKind, BudgetEventRecord, Store};
use crate::types::budget::{
    BudgetDenial, BudgetError, BudgetId, BudgetLimit, BudgetPeriod, BudgetReservation, BudgetScope,
    BudgetWarning, PeriodUnit, ReservationId, ReservationTicket, WarningTier,
};
use crate::types::thread::ThreadId;

/// Enforcement mode mirroring `ironclaw::config::BudgetEnforcementMode`
/// in the host crate. Duplicated here because the engine crate must
/// not take a dependency on the host — both sides hold the same shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    Off,
    Shadow,
    Warn,
    Enforce,
}

impl EnforcementMode {
    pub fn is_recording(self) -> bool {
        !matches!(self, Self::Off)
    }
    pub fn is_denying(self) -> bool {
        matches!(self, Self::Warn | Self::Enforce)
    }
    pub fn gates_approvals(self) -> bool {
        matches!(self, Self::Enforce)
    }
}

/// Runtime configuration for the enforcer. These are the knobs the
/// host's `BudgetConfig` passes in at startup.
#[derive(Debug, Clone)]
pub struct EnforcerConfig {
    pub mode: EnforcementMode,
    pub warn_threshold: f64,
    pub approval_threshold: f64,
}

impl Default for EnforcerConfig {
    fn default() -> Self {
        Self {
            mode: EnforcementMode::Off,
            warn_threshold: 0.75,
            approval_threshold: 0.90,
        }
    }
}

/// Enforces cost-based budgets against the cascade of scopes for an
/// operation. Thread-safe; share via `Arc<BudgetEnforcer>`.
pub struct BudgetEnforcer {
    store: Arc<dyn Store>,
    cfg: EnforcerConfig,
}

impl BudgetEnforcer {
    pub fn new(store: Arc<dyn Store>, cfg: EnforcerConfig) -> Self {
        Self { store, cfg }
    }

    pub fn mode(&self) -> EnforcementMode {
        self.cfg.mode
    }

    /// Attempt to reserve `requested_usd` against every scope in the
    /// cascade. Returns on first denial; rolls back any reservations
    /// granted above that level.
    ///
    /// Cascade order is the input `scopes` vec — the caller is expected
    /// to pass user → project → mission → thread (and optionally a
    /// background-invocation scope at the tail for background work).
    /// The enforcer does NOT reorder.
    ///
    /// In `Off` mode this is a no-op returning an empty ticket.
    /// In `Shadow` mode it performs all DB writes but never denies.
    pub async fn reserve(
        &self,
        scopes: &[BudgetScope],
        requested_usd: Decimal,
        requested_tokens: u64,
        now: DateTime<Utc>,
    ) -> Result<Result<ReservationTicket, BudgetDenial>, BudgetError> {
        // Audit context captured once per cascade — every scope in a
        // single `reserve` call belongs to one user. An empty `scopes`
        // vec still succeeds (returns an empty ticket); the empty
        // `actor_user_id` is fine because there are no events to emit.
        let actor_user_id = scopes
            .first()
            .map(|s| s.user_id().to_string())
            .unwrap_or_default();
        let thread_id = thread_id_from_scopes(scopes);

        if matches!(self.cfg.mode, EnforcementMode::Off) {
            return Ok(Ok(ReservationTicket {
                reservations: Vec::new(),
                warnings: Vec::new(),
                actor_user_id,
                thread_id,
            }));
        }

        let mut granted: Vec<(BudgetId, BudgetScope, AtomicReserveOutcome, Decimal)> = Vec::new();
        let mut warnings: Vec<BudgetWarning> = Vec::new();

        for scope in scopes {
            let budgets = self.store.list_active_budgets_for_scope(scope).await?;
            if budgets.is_empty() {
                continue;
            }
            for budget in &budgets {
                // `period_bounds` is not called inline — the Store
                // layer owns period arithmetic internally via its own
                // `now`-based computation. We just read back the
                // ledger for the current period.
                let ledger = self
                    .store
                    .get_or_create_ledger_for_period(budget.id, now)
                    .await?;

                let projected_committed = ledger.spent_usd + ledger.reserved_usd + requested_usd;
                let utilization_after = if budget.limit.usd.is_zero() {
                    1.0
                } else {
                    ratio(projected_committed, budget.limit.usd)
                };

                // 100% overshoot → hard deny (always, even in Shadow we
                // still *record* the attempted reservation via
                // reserve_atomic below, which will just fail — but in
                // Shadow we coerce failure to success).
                if self.cfg.mode.is_denying() && projected_committed > budget.limit.usd {
                    self.record_audit(
                        BudgetEventKind::Deny,
                        budget.id,
                        None,
                        thread_id,
                        Some(requested_usd),
                        Some(requested_tokens),
                        Some(format!(
                            "usd_exhausted: projected={projected_committed} limit={}",
                            budget.limit.usd
                        )),
                        &actor_user_id,
                        now,
                    )
                    .await;
                    self.rollback_with_audit(
                        &granted,
                        thread_id,
                        &actor_user_id,
                        "rollback_usd_deny",
                        now,
                    )
                    .await;
                    return Ok(Err(BudgetDenial::ExhaustedUsd {
                        first_exhausted: scope.clone(),
                        limit: budget.limit.usd,
                        spent: ledger.spent_usd,
                        requested: requested_usd,
                    }));
                }

                // Approval gate: only the `Enforce` mode inserts the
                // gate. `Warn` treats it as soft (info-level warning
                // instead of denial).
                if utilization_after >= self.cfg.approval_threshold
                    && self.cfg.mode.gates_approvals()
                {
                    self.record_audit(
                        BudgetEventKind::Deny,
                        budget.id,
                        None,
                        thread_id,
                        Some(requested_usd),
                        Some(requested_tokens),
                        Some(format!(
                            "approval_required: utilization={utilization_after:.3}"
                        )),
                        &actor_user_id,
                        now,
                    )
                    .await;
                    self.rollback_with_audit(
                        &granted,
                        thread_id,
                        &actor_user_id,
                        "rollback_approval_required",
                        now,
                    )
                    .await;
                    return Ok(Err(BudgetDenial::RequiresApproval {
                        scope: scope.clone(),
                        utilization: utilization_after,
                    }));
                }

                // Token cap backstop — **best-effort, not atomic**.
                // `ledger.tokens_used` is only advanced on reconcile
                // (see `reconcile_reservation`), and `reserve_atomic`
                // does not increment a reserved-tokens column, so two
                // concurrent reserves near the cap can both pass this
                // check. Documented on `BudgetLimit::tokens`: USD is
                // the authoritative dimension; tokens are a secondary
                // guardrail only.
                if let Some(token_limit) = budget.limit.tokens
                    && self.cfg.mode.is_denying()
                    && ledger.tokens_used + requested_tokens > token_limit
                {
                    self.record_audit(
                        BudgetEventKind::Deny,
                        budget.id,
                        None,
                        thread_id,
                        Some(requested_usd),
                        Some(requested_tokens),
                        Some(format!(
                            "tokens_exhausted: used={}+requested={requested_tokens} limit={token_limit}",
                            ledger.tokens_used
                        )),
                        &actor_user_id,
                        now,
                    )
                    .await;
                    self.rollback_with_audit(
                        &granted,
                        thread_id,
                        &actor_user_id,
                        "rollback_tokens_deny",
                        now,
                    )
                    .await;
                    return Ok(Err(BudgetDenial::ExhaustedTokens {
                        first_exhausted: scope.clone(),
                        limit: token_limit,
                        used: ledger.tokens_used,
                    }));
                }

                // Now commit the reservation. In Shadow we ignore a
                // `None` return (DB-level deny) and proceed anyway,
                // which we implement by writing the reservation with
                // the DB limit temporarily boosted — but that requires
                // schema changes. Simpler: in Shadow we skip the
                // `reserve_atomic` and pretend to have a reservation.
                if matches!(self.cfg.mode, EnforcementMode::Shadow) {
                    // Shadow: record a synthetic outcome so the
                    // enforcer appears to have reserved, and audit the
                    // attempted amount.
                    let shadow_outcome = AtomicReserveOutcome {
                        reservation_id: ReservationId::new(),
                        budget_id: budget.id,
                        reserved_usd: requested_usd,
                        reserved_tokens: requested_tokens,
                        ledger: ledger.clone(),
                    };
                    self.record_audit(
                        BudgetEventKind::Reserve,
                        budget.id,
                        Some(shadow_outcome.reservation_id),
                        thread_id,
                        Some(requested_usd),
                        Some(requested_tokens),
                        Some("shadow".into()),
                        &actor_user_id,
                        now,
                    )
                    .await;
                    granted.push((budget.id, scope.clone(), shadow_outcome, requested_usd));
                    // Still attach warnings so telemetry is rich.
                    if let Some(w) = warning_for(scope, &budget.limit, utilization_after, &self.cfg)
                    {
                        warnings.push(w);
                    }
                    continue;
                }

                let outcome = match self
                    .store
                    .reserve_atomic(budget.id, requested_usd, requested_tokens, now)
                    .await?
                {
                    Some(o) => o,
                    None => {
                        // DB-level denial (concurrent reserver won).
                        self.record_audit(
                            BudgetEventKind::Deny,
                            budget.id,
                            None,
                            thread_id,
                            Some(requested_usd),
                            Some(requested_tokens),
                            Some("usd_exhausted_db_level".into()),
                            &actor_user_id,
                            now,
                        )
                        .await;
                        self.rollback_with_audit(
                            &granted,
                            thread_id,
                            &actor_user_id,
                            "rollback_usd_deny_db",
                            now,
                        )
                        .await;
                        return Ok(Err(BudgetDenial::ExhaustedUsd {
                            first_exhausted: scope.clone(),
                            limit: budget.limit.usd,
                            spent: ledger.spent_usd + ledger.reserved_usd,
                            requested: requested_usd,
                        }));
                    }
                };

                self.record_audit(
                    BudgetEventKind::Reserve,
                    budget.id,
                    Some(outcome.reservation_id),
                    thread_id,
                    Some(requested_usd),
                    Some(requested_tokens),
                    None,
                    &actor_user_id,
                    now,
                )
                .await;

                granted.push((budget.id, scope.clone(), outcome, requested_usd));

                if let Some(w) = warning_for(scope, &budget.limit, utilization_after, &self.cfg) {
                    warnings.push(w);
                }
            }
        }

        let reservations: Vec<BudgetReservation> = granted
            .iter()
            .map(|(budget_id, _scope, outcome, _amt)| BudgetReservation {
                id: outcome.reservation_id,
                budget_id: *budget_id,
                reserved_usd: outcome.reserved_usd,
                reserved_tokens: outcome.reserved_tokens,
                created_at: outcome.ledger.updated_at,
            })
            .collect();

        Ok(Ok(ReservationTicket {
            reservations,
            warnings,
            actor_user_id,
            thread_id,
        }))
    }

    /// Settle a prior reservation with `actual_usd` — never fails due to
    /// budget, only due to plumbing.
    pub async fn reconcile(
        &self,
        ticket: &ReservationTicket,
        actual_usd: Decimal,
        actual_tokens: u64,
        now: DateTime<Utc>,
    ) -> Result<(), BudgetError> {
        if matches!(self.cfg.mode, EnforcementMode::Off) {
            return Ok(());
        }
        // Split the actual spend proportionally across reservations.
        // In practice all reservations in a ticket are for the same
        // underlying LLM call, so the simplest fair split is: each
        // reservation eats its proportional share of `actual_usd`.
        let total_reserved: Decimal = ticket.reservations.iter().map(|r| r.reserved_usd).sum();
        if total_reserved.is_zero() {
            return Ok(());
        }
        for res in &ticket.reservations {
            let fraction = res.reserved_usd / total_reserved;
            let share = actual_usd * fraction;
            // Tokens: integer split, accept small rounding at the tail.
            let token_share =
                (actual_tokens as f64 * ratio(res.reserved_usd, total_reserved)).round() as u64;
            self.store
                .reconcile_reservation(
                    res.id,
                    res.budget_id,
                    res.reserved_usd,
                    share,
                    token_share,
                    now,
                )
                .await?;
            self.record_audit(
                BudgetEventKind::Reconcile,
                res.budget_id,
                Some(res.id),
                ticket.thread_id,
                Some(share),
                Some(token_share),
                None,
                &ticket.actor_user_id,
                now,
            )
            .await;
        }
        Ok(())
    }

    /// Release reservations without recording spend (thread aborted,
    /// LLM errored before any work).
    pub async fn release(
        &self,
        ticket: &ReservationTicket,
        now: DateTime<Utc>,
    ) -> Result<(), BudgetError> {
        if matches!(self.cfg.mode, EnforcementMode::Off) {
            return Ok(());
        }
        for res in &ticket.reservations {
            self.store
                .release_reservation(res.id, res.budget_id, res.reserved_usd, now)
                .await?;
            self.record_audit(
                BudgetEventKind::Release,
                res.budget_id,
                Some(res.id),
                ticket.thread_id,
                Some(res.reserved_usd),
                None,
                None,
                &ticket.actor_user_id,
                now,
            )
            .await;
        }
        Ok(())
    }

    /// Roll back every reservation granted above a cascade-level denial.
    /// Each release emits a `Release` audit row carrying `reason` so a
    /// downstream reader can stitch the deny/rollback pair together.
    async fn rollback_with_audit(
        &self,
        granted: &[(BudgetId, BudgetScope, AtomicReserveOutcome, Decimal)],
        thread_id: Option<ThreadId>,
        actor_user_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) {
        for (budget_id, _, outcome, _) in granted {
            // Best-effort on the store write; if release fails here we
            // at worst leak some `reserved_usd` that the next period
            // rollover or the explicit release path will reclaim.
            let _ = self
                .store
                .release_reservation(
                    outcome.reservation_id,
                    *budget_id,
                    outcome.reserved_usd,
                    now,
                )
                .await;
            self.record_audit(
                BudgetEventKind::Release,
                *budget_id,
                Some(outcome.reservation_id),
                thread_id,
                Some(outcome.reserved_usd),
                None,
                Some(reason.to_string()),
                actor_user_id,
                now,
            )
            .await;
        }
    }

    /// Append one row to the `budget_events` audit table. Errors are
    /// logged and swallowed — the audit write is a side-effect of the
    /// business-critical operation that already completed, and failing
    /// a successful reservation because the audit DB is down would be
    /// worse than a gap in the audit trail. Monitoring should still
    /// alert on the `record_budget_event failed` log line.
    #[allow(clippy::too_many_arguments)]
    async fn record_audit(
        &self,
        event_kind: BudgetEventKind,
        budget_id: BudgetId,
        reservation_id: Option<ReservationId>,
        thread_id: Option<ThreadId>,
        amount_usd: Option<Decimal>,
        tokens: Option<u64>,
        reason: Option<String>,
        actor_user_id: &str,
        now: DateTime<Utc>,
    ) {
        let event = BudgetEventRecord {
            budget_id,
            thread_id,
            reservation_id,
            event_kind,
            amount_usd,
            tokens,
            reason,
            actor_user_id: actor_user_id.to_string(),
            created_at: now,
        };
        if let Err(e) = self.store.record_budget_event(&event).await {
            tracing::warn!(
                budget_id = %budget_id,
                event_kind = event_kind.as_str(),
                "record_budget_event failed: {e}",
            );
        }
    }
}

/// Scan a cascade for a `Thread` scope. Used for `budget_events` audit
/// rows; if the cascade has no thread (e.g. a standalone
/// `BackgroundInvocation`), returns `None`.
fn thread_id_from_scopes(scopes: &[BudgetScope]) -> Option<ThreadId> {
    scopes.iter().find_map(|s| match s {
        BudgetScope::Thread { thread_id, .. } => Some(*thread_id),
        _ => None,
    })
}

fn warning_for(
    scope: &BudgetScope,
    _limit: &BudgetLimit,
    utilization: f64,
    cfg: &EnforcerConfig,
) -> Option<BudgetWarning> {
    if utilization >= cfg.approval_threshold {
        // Already handled as a denial above; no warning needed.
        None
    } else if utilization >= cfg.warn_threshold {
        Some(BudgetWarning {
            scope: scope.clone(),
            tier: WarningTier::Warn,
            utilization,
        })
    } else if utilization >= 0.50 {
        Some(BudgetWarning {
            scope: scope.clone(),
            tier: WarningTier::Info,
            utilization,
        })
    } else {
        None
    }
}

fn ratio(num: Decimal, denom: Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    let n = num.to_f64().unwrap_or(0.0);
    let d = denom.to_f64().unwrap_or(1.0);
    if d == 0.0 { 1.0 } else { n / d }
}

/// Compute the `(period_start, period_end)` for `now` under `period`.
///
/// - `PerInvocation`: a one-hour window starting at `now`. The ledger
///   row is synthetic — each invocation gets its own ledger.
/// - `Rolling24h`: quantised to UTC-midnight so the ledger is stable
///   across concurrent reservations. Documented trade-off: a user who
///   spends $4.99 at 23:59 and $0.02 at 00:00 sees both charged to
///   different daily ledgers. Truly-rolling would require
///   per-reservation timestamp rows, which doubles the schema cost
///   for a minor correctness win.
/// - `Calendar { tz, unit }`: aligned to `unit` boundary in `tz`.
pub fn period_bounds(period: &BudgetPeriod, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    match period {
        BudgetPeriod::PerInvocation => (now, now + Duration::hours(1)),
        BudgetPeriod::Rolling24h => {
            let start = quantise_utc_day(now);
            (start, start + Duration::days(1))
        }
        BudgetPeriod::Calendar { tz, unit } => calendar_period(tz, *unit, now),
    }
}

fn quantise_utc_day(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
        .unwrap_or(now)
}

fn calendar_period(
    tz_name: &str,
    unit: PeriodUnit,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    // For unknown or unparseable timezones, fall back to UTC — the
    // alternative (panic/error) would break reservations for a minor
    // config issue, and UTC is always a sane approximation.
    //
    // NOTE: proper IANA timezone support would require `chrono-tz`.
    // The engine crate doesn't carry that dep today; this function
    // currently treats `tz_name` as an opaque label and anchors
    // everything to UTC. Documented here so future maintainers don't
    // think they're fixing a bug by replacing it.
    let _ = tz_name;
    let start = match unit {
        PeriodUnit::Day => quantise_utc_day(now),
        PeriodUnit::Week => {
            let day = now.date_naive();
            // ISO week starts Monday (weekday 0). Back up to Monday.
            let offset = day.weekday().num_days_from_monday() as i64;
            let monday = day - chrono::Days::new(offset as u64);
            monday
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
                .unwrap_or(now)
        }
        PeriodUnit::Month => {
            let nd = now.date_naive();
            let first = chrono::NaiveDate::from_ymd_opt(nd.year(), nd.month(), 1)
                .expect("ymd(year, month, 1) always valid"); // safety: year/month come from a valid NaiveDate, day=1 is always in range
            first
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
                .unwrap_or(now)
        }
    };
    let end = match unit {
        PeriodUnit::Day => start + Duration::days(1),
        PeriodUnit::Week => start + Duration::days(7),
        PeriodUnit::Month => {
            // Approximate month-end as +31 days; the period is a ledger
            // bucket so exact month boundary doesn't matter as long as
            // the start is stable.
            start + Duration::days(31)
        }
    };
    (start, end)
}

use chrono::{Datelike, TimeZone};

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::store::{AtomicReserveOutcome, BudgetEventRecord};
    use crate::types::budget::{
        BackgroundKind, Budget, BudgetLedger, BudgetLimit, BudgetPeriod, BudgetSource,
        ReservationId,
    };
    use crate::types::error::EngineError;
    use crate::types::project::ProjectId;
    use async_trait::async_trait;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// In-memory fake Store that implements just the budget methods
    /// the enforcer actually calls. Non-budget methods panic if hit —
    /// any accidental call means the enforcer's contract slipped.
    struct FakeStore {
        budgets: Mutex<HashMap<BudgetId, Budget>>,
        ledgers: Mutex<HashMap<BudgetId, BudgetLedger>>,
        scope_index: Mutex<HashMap<(String, String), Vec<BudgetId>>>,
        events: Mutex<Vec<BudgetEventRecord>>,
        reserve_calls: AtomicU64,
    }

    impl FakeStore {
        fn new() -> Self {
            Self {
                budgets: Mutex::new(HashMap::new()),
                ledgers: Mutex::new(HashMap::new()),
                scope_index: Mutex::new(HashMap::new()),
                events: Mutex::new(Vec::new()),
                reserve_calls: AtomicU64::new(0),
            }
        }

        fn add(&self, b: Budget) {
            self.scope_index
                .lock()
                .unwrap()
                .entry((b.scope.kind_str().to_string(), b.scope.scope_id()))
                .or_default()
                .push(b.id);
            self.budgets.lock().unwrap().insert(b.id, b);
        }

        fn set_ledger(&self, id: BudgetId, spent: Decimal, reserved: Decimal, tokens: u64) {
            let now = Utc::now();
            self.ledgers.lock().unwrap().insert(
                id,
                BudgetLedger {
                    budget_id: id,
                    period_start: quantise_utc_day(now),
                    period_end: quantise_utc_day(now) + Duration::days(1),
                    spent_usd: spent,
                    reserved_usd: reserved,
                    tokens_used: tokens,
                    updated_at: now,
                },
            );
        }

        fn ledger_of(&self, id: BudgetId) -> BudgetLedger {
            self.ledgers
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_else(|| BudgetLedger {
                    budget_id: id,
                    period_start: quantise_utc_day(Utc::now()),
                    period_end: quantise_utc_day(Utc::now()) + Duration::days(1),
                    spent_usd: Decimal::ZERO,
                    reserved_usd: Decimal::ZERO,
                    tokens_used: 0,
                    updated_at: Utc::now(),
                })
        }
    }

    #[async_trait]
    impl Store for FakeStore {
        async fn save_thread(&self, _t: &crate::types::thread::Thread) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_thread(
            &self,
            _id: crate::types::thread::ThreadId,
        ) -> Result<Option<crate::types::thread::Thread>, EngineError> {
            unimplemented!()
        }
        async fn list_threads(
            &self,
            _p: ProjectId,
            _u: &str,
        ) -> Result<Vec<crate::types::thread::Thread>, EngineError> {
            unimplemented!()
        }
        async fn update_thread_state(
            &self,
            _id: crate::types::thread::ThreadId,
            _s: crate::types::thread::ThreadState,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn save_step(&self, _s: &crate::types::step::Step) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_steps(
            &self,
            _id: crate::types::thread::ThreadId,
        ) -> Result<Vec<crate::types::step::Step>, EngineError> {
            unimplemented!()
        }
        async fn append_events(
            &self,
            _e: &[crate::types::event::ThreadEvent],
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_events(
            &self,
            _id: crate::types::thread::ThreadId,
        ) -> Result<Vec<crate::types::event::ThreadEvent>, EngineError> {
            unimplemented!()
        }
        async fn save_project(
            &self,
            _p: &crate::types::project::Project,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_project(
            &self,
            _id: ProjectId,
        ) -> Result<Option<crate::types::project::Project>, EngineError> {
            unimplemented!()
        }
        async fn save_memory_doc(
            &self,
            _d: &crate::types::memory::MemoryDoc,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_memory_doc(
            &self,
            _id: crate::types::memory::DocId,
        ) -> Result<Option<crate::types::memory::MemoryDoc>, EngineError> {
            unimplemented!()
        }
        async fn list_memory_docs(
            &self,
            _p: ProjectId,
            _u: &str,
        ) -> Result<Vec<crate::types::memory::MemoryDoc>, EngineError> {
            unimplemented!()
        }
        async fn save_lease(
            &self,
            _l: &crate::types::capability::CapabilityLease,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_active_leases(
            &self,
            _thread_id: crate::types::thread::ThreadId,
        ) -> Result<Vec<crate::types::capability::CapabilityLease>, EngineError> {
            unimplemented!()
        }
        async fn revoke_lease(
            &self,
            _id: crate::types::capability::LeaseId,
            _r: &str,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }

        async fn save_mission(
            &self,
            _m: &crate::types::mission::Mission,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }
        async fn load_mission(
            &self,
            _id: crate::types::mission::MissionId,
        ) -> Result<Option<crate::types::mission::Mission>, EngineError> {
            unimplemented!()
        }
        async fn list_missions(
            &self,
            _p: ProjectId,
            _u: &str,
        ) -> Result<Vec<crate::types::mission::Mission>, EngineError> {
            unimplemented!()
        }
        async fn update_mission_status(
            &self,
            _id: crate::types::mission::MissionId,
            _s: crate::types::mission::MissionStatus,
        ) -> Result<(), EngineError> {
            unimplemented!()
        }

        // Budget methods — the only ones the enforcer calls.

        async fn list_active_budgets_for_scope(
            &self,
            scope: &BudgetScope,
        ) -> Result<Vec<Budget>, BudgetError> {
            let key = (scope.kind_str().to_string(), scope.scope_id());
            let ids = self
                .scope_index
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let budgets = self.budgets.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| budgets.get(id).cloned())
                .collect())
        }

        async fn get_or_create_ledger_for_period(
            &self,
            budget_id: BudgetId,
            now: DateTime<Utc>,
        ) -> Result<BudgetLedger, BudgetError> {
            let mut ledgers = self.ledgers.lock().unwrap();
            Ok(ledgers
                .entry(budget_id)
                .or_insert_with(|| BudgetLedger {
                    budget_id,
                    period_start: quantise_utc_day(now),
                    period_end: quantise_utc_day(now) + Duration::days(1),
                    spent_usd: Decimal::ZERO,
                    reserved_usd: Decimal::ZERO,
                    tokens_used: 0,
                    updated_at: now,
                })
                .clone())
        }

        async fn reserve_atomic(
            &self,
            budget_id: BudgetId,
            requested_usd: Decimal,
            requested_tokens: u64,
            now: DateTime<Utc>,
        ) -> Result<Option<AtomicReserveOutcome>, BudgetError> {
            self.reserve_calls.fetch_add(1, Ordering::SeqCst);
            let budget_limit = self
                .budgets
                .lock()
                .unwrap()
                .get(&budget_id)
                .map(|b| b.limit.usd)
                .ok_or_else(|| BudgetError::UnknownBudget(budget_id))?;
            let mut ledgers = self.ledgers.lock().unwrap();
            let ledger = ledgers.entry(budget_id).or_insert_with(|| BudgetLedger {
                budget_id,
                period_start: quantise_utc_day(now),
                period_end: quantise_utc_day(now) + Duration::days(1),
                spent_usd: Decimal::ZERO,
                reserved_usd: Decimal::ZERO,
                tokens_used: 0,
                updated_at: now,
            });
            if ledger.spent_usd + ledger.reserved_usd + requested_usd > budget_limit {
                return Ok(None);
            }
            ledger.reserved_usd += requested_usd;
            ledger.updated_at = now;
            Ok(Some(AtomicReserveOutcome {
                reservation_id: ReservationId::new(),
                budget_id,
                reserved_usd: requested_usd,
                reserved_tokens: requested_tokens,
                ledger: ledger.clone(),
            }))
        }

        async fn reconcile_reservation(
            &self,
            _rid: ReservationId,
            budget_id: BudgetId,
            original_reserved_usd: Decimal,
            actual_usd: Decimal,
            actual_tokens: u64,
            now: DateTime<Utc>,
        ) -> Result<(), BudgetError> {
            let mut ledgers = self.ledgers.lock().unwrap();
            let ledger = ledgers
                .get_mut(&budget_id)
                .ok_or_else(|| BudgetError::Store {
                    reason: "no ledger".into(),
                })?;
            ledger.spent_usd += actual_usd;
            ledger.reserved_usd = if original_reserved_usd > ledger.reserved_usd {
                Decimal::ZERO
            } else {
                ledger.reserved_usd - original_reserved_usd
            };
            ledger.tokens_used += actual_tokens;
            ledger.updated_at = now;
            Ok(())
        }

        async fn release_reservation(
            &self,
            rid: ReservationId,
            budget_id: BudgetId,
            original_reserved_usd: Decimal,
            now: DateTime<Utc>,
        ) -> Result<(), BudgetError> {
            self.reconcile_reservation(rid, budget_id, original_reserved_usd, Decimal::ZERO, 0, now)
                .await
        }

        async fn record_budget_event(&self, event: &BudgetEventRecord) -> Result<(), BudgetError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    fn mk_budget(scope: BudgetScope, limit_usd: Decimal) -> Budget {
        Budget {
            id: BudgetId::new(),
            scope,
            limit: BudgetLimit {
                usd: limit_usd,
                tokens: None,
                wall_clock_secs: None,
            },
            period: BudgetPeriod::Rolling24h,
            source: BudgetSource::Default,
            active: true,
            created_at: Utc::now(),
            created_by: "system".into(),
        }
    }

    fn enforce(store: Arc<FakeStore>, mode: EnforcementMode) -> BudgetEnforcer {
        let cfg = EnforcerConfig {
            mode,
            warn_threshold: 0.75,
            approval_threshold: 0.90,
        };
        BudgetEnforcer::new(store, cfg)
    }

    #[tokio::test]
    async fn off_mode_never_touches_the_store() {
        let store = Arc::new(FakeStore::new());
        store.add(mk_budget(
            BudgetScope::User {
                user_id: "alice".into(),
            },
            dec!(1.00),
        ));
        let enf = enforce(Arc::clone(&store) as Arc<FakeStore>, EnforcementMode::Off);
        let ticket = enf
            .reserve(
                &[BudgetScope::User {
                    user_id: "alice".into(),
                }],
                dec!(50.00),
                0,
                Utc::now(),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(ticket.is_empty());
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn denies_at_first_exhausted_scope_in_cascade() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let project = BudgetScope::Project {
            user_id: "alice".into(),
            project_id: ProjectId::new(),
        };

        // User has plenty ($5), project is nearly exhausted ($1 limit,
        // $0.95 spent).
        let user_b = mk_budget(user.clone(), dec!(5.00));
        let project_b = mk_budget(project.clone(), dec!(1.00));
        let project_id = project_b.id;
        store.add(user_b);
        store.add(project_b);
        store.set_ledger(project_id, dec!(0.95), Decimal::ZERO, 0);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let denial = enf
            .reserve(&[user, project.clone()], dec!(0.10), 0, Utc::now())
            .await
            .unwrap()
            .unwrap_err();

        match denial {
            BudgetDenial::ExhaustedUsd {
                first_exhausted, ..
            } => assert_eq!(first_exhausted, project),
            other => panic!("expected ExhaustedUsd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn denial_rolls_back_earlier_reservations() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let project = BudgetScope::Project {
            user_id: "alice".into(),
            project_id: ProjectId::new(),
        };

        let user_b = mk_budget(user.clone(), dec!(5.00));
        let project_b = mk_budget(project.clone(), dec!(0.05));
        let user_id = user_b.id;
        store.add(user_b);
        store.add(project_b);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let denial = enf
            .reserve(&[user, project], dec!(0.10), 0, Utc::now())
            .await
            .unwrap();
        assert!(denial.is_err());

        // The user-scope reservation must have been released on denial:
        // reserved_usd on user's ledger should be back to zero.
        let user_ledger = store.ledger_of(user_id);
        assert_eq!(
            user_ledger.reserved_usd,
            Decimal::ZERO,
            "expected rollback, user ledger still has reservation"
        );
    }

    #[tokio::test]
    async fn approval_gate_triggers_above_90_percent() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let b = mk_budget(user.clone(), dec!(1.00));
        let id = b.id;
        store.add(b);
        store.set_ledger(id, dec!(0.85), Decimal::ZERO, 0);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let denial = enf
            .reserve(std::slice::from_ref(&user), dec!(0.10), 0, Utc::now())
            .await
            .unwrap()
            .unwrap_err();
        match denial {
            BudgetDenial::RequiresApproval { scope, .. } => assert_eq!(scope, user),
            other => panic!("expected RequiresApproval, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn warn_threshold_attaches_warning_without_denying() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let b = mk_budget(user.clone(), dec!(1.00));
        let id = b.id;
        store.add(b);
        store.set_ledger(id, dec!(0.70), Decimal::ZERO, 0);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let ticket = enf
            .reserve(std::slice::from_ref(&user), dec!(0.10), 0, Utc::now())
            .await
            .unwrap()
            .unwrap();
        assert!(
            ticket
                .warnings
                .iter()
                .any(|w| matches!(w.tier, WarningTier::Warn)),
            "expected Warn-tier warning at 80% utilisation"
        );
    }

    #[tokio::test]
    async fn shadow_mode_records_but_never_denies() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let b = mk_budget(user.clone(), dec!(1.00));
        let id = b.id;
        store.add(b);
        // Already fully exhausted.
        store.set_ledger(id, dec!(1.00), Decimal::ZERO, 0);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Shadow,
        );
        let ticket = enf
            .reserve(&[user], dec!(5.00), 0, Utc::now())
            .await
            .expect("plumbing failure shouldn't happen")
            .expect("shadow should never deny, even when exhausted");
        assert_eq!(ticket.reservations.len(), 1);
    }

    #[tokio::test]
    async fn period_bounds_per_invocation_is_one_hour_from_now() {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 34, 56).unwrap();
        let (start, end) = period_bounds(&BudgetPeriod::PerInvocation, now);
        assert_eq!(start, now);
        assert_eq!(end, now + Duration::hours(1));
    }

    #[tokio::test]
    async fn period_bounds_rolling_24h_quantises_to_utc_midnight() {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 34, 56).unwrap();
        let (start, end) = period_bounds(&BudgetPeriod::Rolling24h, now);
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap());
        assert_eq!(end, Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap());
    }

    #[tokio::test]
    async fn period_bounds_calendar_month_starts_first_of_month() {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 34, 56).unwrap();
        let (start, _end) = period_bounds(
            &BudgetPeriod::Calendar {
                tz: "UTC".into(),
                unit: PeriodUnit::Month,
            },
            now,
        );
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap());
    }

    #[tokio::test]
    async fn reconcile_settles_actual_cost_proportionally() {
        let store = Arc::new(FakeStore::new());
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        let b = mk_budget(user.clone(), dec!(5.00));
        let budget_id = b.id;
        store.add(b);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let now = Utc::now();
        let ticket = enf
            .reserve(&[user], dec!(0.50), 1000, now)
            .await
            .unwrap()
            .unwrap();
        enf.reconcile(&ticket, dec!(0.20), 500, now).await.unwrap();

        let ledger = store.ledger_of(budget_id);
        assert_eq!(ledger.spent_usd, dec!(0.20));
        // Reconcile clears the full reservation slot (original=$0.50)
        // regardless of actual spend being lower — reserved_usd is 0.
        assert_eq!(ledger.reserved_usd, Decimal::ZERO);
        assert_eq!(ledger.tokens_used, 500);
    }

    #[tokio::test]
    async fn background_scope_is_valid_last_cascade_entry() {
        let store = Arc::new(FakeStore::new());
        let bg = BudgetScope::BackgroundInvocation {
            user_id: "alice".into(),
            kind: BackgroundKind::Heartbeat,
            correlation_id: "tick-1".into(),
        };
        store.add(mk_budget(bg.clone(), dec!(0.05)));
        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );
        let ticket = enf
            .reserve(&[bg], dec!(0.02), 0, Utc::now())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ticket.reservations.len(), 1);
    }

    /// Regression for #2847 review (Copilot): the enforcer must call
    /// `Store::record_budget_event` on every reserve/reconcile/release/deny
    /// path so the `budget_events` audit table is actually populated.
    /// Before this fix the table stayed empty despite the docs claiming
    /// otherwise.
    #[tokio::test]
    async fn enforcer_emits_audit_events_on_reserve_and_reconcile() {
        let store = Arc::new(FakeStore::new());
        store.add(mk_budget(
            BudgetScope::User {
                user_id: "alice".into(),
            },
            dec!(1.00),
        ));
        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );

        let ticket = enf
            .reserve(
                &[BudgetScope::User {
                    user_id: "alice".into(),
                }],
                dec!(0.10),
                42,
                Utc::now(),
            )
            .await
            .unwrap()
            .unwrap();

        // 1 Reserve row on success.
        {
            let events = store.events.lock().unwrap();
            assert_eq!(
                events.len(),
                1,
                "reserve should emit exactly one Reserve row"
            );
            assert_eq!(events[0].event_kind, BudgetEventKind::Reserve);
            assert_eq!(events[0].actor_user_id, "alice");
            assert_eq!(events[0].amount_usd, Some(dec!(0.10)));
            assert_eq!(events[0].tokens, Some(42));
            assert!(events[0].reservation_id.is_some());
        }

        enf.reconcile(&ticket, dec!(0.07), 30, Utc::now())
            .await
            .unwrap();

        {
            let events = store.events.lock().unwrap();
            assert_eq!(events.len(), 2, "reconcile should append one Reconcile row");
            assert_eq!(events[1].event_kind, BudgetEventKind::Reconcile);
            assert_eq!(events[1].amount_usd, Some(dec!(0.07)));
            assert_eq!(events[1].tokens, Some(30));
            assert_eq!(events[1].reservation_id, Some(ticket.reservations[0].id));
        }
    }

    /// Regression for #2847 review: when a cascade denial aborts the
    /// reservation, the audit trail must show exactly one `Deny` row for
    /// the budget that exhausted, plus one `Release` row per reservation
    /// the enforcer rolled back at the earlier cascade levels.
    #[tokio::test]
    async fn enforcer_emits_deny_and_rollback_audit_events() {
        let store = Arc::new(FakeStore::new());
        let user_b = mk_budget(
            BudgetScope::User {
                user_id: "alice".into(),
            },
            dec!(1.00),
        );
        let user_budget_id = user_b.id;
        store.add(user_b);
        // Project budget that will deny on the second scope.
        let proj_scope = BudgetScope::Project {
            user_id: "alice".into(),
            project_id: crate::types::project::ProjectId::new(),
        };
        let proj_b = mk_budget(proj_scope.clone(), dec!(0.01));
        let proj_budget_id = proj_b.id;
        store.add(proj_b);

        let enf = enforce(
            Arc::clone(&store) as Arc<FakeStore>,
            EnforcementMode::Enforce,
        );

        let result = enf
            .reserve(
                &[
                    BudgetScope::User {
                        user_id: "alice".into(),
                    },
                    proj_scope,
                ],
                dec!(0.50),
                0,
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(result.is_err(), "project cap should trigger a denial");

        let events = store.events.lock().unwrap();
        // Expected order: Reserve(user), Deny(project), Release(user — rollback).
        assert_eq!(events.len(), 3, "got: {:?}", *events);
        assert_eq!(events[0].event_kind, BudgetEventKind::Reserve);
        assert_eq!(events[0].budget_id, user_budget_id);
        assert_eq!(events[1].event_kind, BudgetEventKind::Deny);
        assert_eq!(events[1].budget_id, proj_budget_id);
        assert!(
            events[1]
                .reason
                .as_deref()
                .unwrap()
                .contains("usd_exhausted")
        );
        assert_eq!(events[2].event_kind, BudgetEventKind::Release);
        assert_eq!(events[2].budget_id, user_budget_id);
        assert_eq!(events[2].reason.as_deref(), Some("rollback_usd_deny"));
    }
}
