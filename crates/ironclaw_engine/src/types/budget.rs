//! Cost-based budgets — the unit of autonomous-spend authorization.
//!
//! A [`Budget`] caps what a scope (user, project, mission, thread, or background
//! invocation) may spend over a period. The [`BudgetLedger`] tracks running
//! totals within a single period. Reservations are recorded by the store before
//! each costed operation (LLM call, costed tool) and reconciled against actual
//! cost after the operation completes.
//!
//! USD is the primary budget dimension. Tokens and wall-clock time are optional
//! secondary caps; iteration count is not a budget dimension at all (see issue
//! #2843 — iteration caps were the arbitrary-limit bug this module replaces).
//!
//! All budget decisions are recorded to the `budget_events` table for audit.

use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::mission::MissionId;
use crate::types::project::ProjectId;
use crate::types::thread::ThreadId;

// ── Identifiers ─────────────────────────────────────────────

/// Unique identifier for a [`Budget`] row.
///
/// Follows the existing engine ID convention (`ProjectId`, `MissionId`,
/// `ThreadId`): a tuple newtype over `Uuid` with a public inner field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetId(pub Uuid);

impl BudgetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BudgetId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BudgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a [`BudgetReservation`] (in-flight, not yet
/// reconciled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReservationId(pub Uuid);

impl ReservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ReservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Scope ───────────────────────────────────────────────────

/// What the budget applies to.
///
/// Each variant names the scope's owning identifier. The store looks up
/// budgets by `(scope_kind, scope_id)`; serialization chooses the opaque
/// string via [`BudgetScope::scope_id`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope_kind", rename_all = "snake_case")]
pub enum BudgetScope {
    /// Cap on total autonomous spend for a user across all their work.
    User { user_id: String },
    /// Cap on spend within a project (e.g. "$2/day on this project").
    Project {
        user_id: String,
        project_id: ProjectId,
    },
    /// Cap on spend for a long-running mission (e.g. "$10/week on daily-standup").
    Mission {
        user_id: String,
        mission_id: MissionId,
    },
    /// Cap on spend for a single thread (per-invocation).
    Thread {
        user_id: String,
        thread_id: ThreadId,
    },
    /// Cap on spend for a single background invocation (heartbeat tick,
    /// routine fire, container job). Per-invocation period only.
    BackgroundInvocation {
        user_id: String,
        #[serde(rename = "background_kind")]
        kind: BackgroundKind,
        /// Opaque correlation id from the scheduler (e.g. the job id or the
        /// `routine_run` id). Used to reconcile reservation to event.
        correlation_id: String,
    },
}

impl BudgetScope {
    /// The user this scope is owned by. Every budget belongs to a user —
    /// there is no global/shared budget.
    pub fn user_id(&self) -> &str {
        match self {
            Self::User { user_id }
            | Self::Project { user_id, .. }
            | Self::Mission { user_id, .. }
            | Self::Thread { user_id, .. }
            | Self::BackgroundInvocation { user_id, .. } => user_id,
        }
    }

    /// `scope_kind` column value for the DB row.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::Project { .. } => "project",
            Self::Mission { .. } => "mission",
            Self::Thread { .. } => "thread",
            Self::BackgroundInvocation { .. } => "background",
        }
    }

    /// `scope_id` column value for the DB row. Opaque string.
    ///
    /// - User: the user id
    /// - Project: the project uuid
    /// - Mission: the mission uuid
    /// - Thread: the thread uuid
    /// - BackgroundInvocation: `"<kind>:<user_id>:<correlation_id>"`
    ///
    /// The `user_id` segment is embedded in the background scope_id
    /// because `correlation_id` is a scheduler-local identifier (e.g.
    /// `tick-42`, `run-7`) that is NOT globally unique. Two users'
    /// heartbeat-tick-42 budgets must hash to different scope_ids so
    /// the `uq_budgets_*_active` unique indexes don't collapse them,
    /// and `list_active_budgets_for_scope(..)` doesn't return another
    /// user's row.
    pub fn scope_id(&self) -> String {
        match self {
            Self::User { user_id } => user_id.clone(),
            Self::Project { project_id, .. } => project_id.0.to_string(),
            Self::Mission { mission_id, .. } => mission_id.0.to_string(),
            Self::Thread { thread_id, .. } => thread_id.0.to_string(),
            Self::BackgroundInvocation {
                user_id,
                kind,
                correlation_id,
            } => format!("{}:{}:{}", kind.as_str(), user_id, correlation_id),
        }
    }
}

/// Category of background work that holds its own per-invocation budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    Heartbeat,
    RoutineLightweight,
    RoutineStandard,
    MissionTick,
    ContainerJob,
    UserInitiated,
}

impl BackgroundKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Heartbeat => "heartbeat",
            Self::RoutineLightweight => "routine_lightweight",
            Self::RoutineStandard => "routine_standard",
            Self::MissionTick => "mission_tick",
            Self::ContainerJob => "container_job",
            Self::UserInitiated => "user_initiated",
        }
    }
}

// ── Limits & periods ────────────────────────────────────────

/// The caps a budget enforces. USD is primary and always set; tokens and
/// wall-clock are optional secondary caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimit {
    /// Primary dollar cap. Zero means "no spend allowed" (hard gate, not
    /// unbounded). Use [`BudgetLimit::unlimited_usd`] for an absent cap
    /// when composing defaults.
    pub usd: Decimal,
    /// Optional **best-effort** cap on cumulative input+output tokens.
    /// Unlike `usd`, token accounting is not atomic: tokens are settled
    /// on reconcile (not on reserve), and the enforcer's check against
    /// this cap inspects only the already-settled `tokens_used` column.
    /// Concurrent reserves at high utilization can therefore exceed
    /// this cap by the in-flight amount. USD is the authoritative
    /// dimension; set this only as a secondary guardrail.
    pub tokens: Option<u64>,
    /// Optional wall-clock cap in seconds. Same best-effort caveat as
    /// `tokens` — not enforced atomically by the Store layer.
    pub wall_clock_secs: Option<u64>,
}

impl BudgetLimit {
    pub fn usd_only(usd: Decimal) -> Self {
        Self {
            usd,
            tokens: None,
            wall_clock_secs: None,
        }
    }

    /// A sentinel limit expressing "effectively no cap" for the USD
    /// dimension. Used when a scope is disabled and a caller still
    /// wants to hand the enforcer a well-formed [`BudgetLimit`].
    /// Callers MUST NOT persist this value — unlimited budgets are
    /// represented by absent rows, not sentinel values.
    pub fn unlimited_usd() -> Self {
        // `HARD_CAP_BUDGET_USD` in `ironclaw_common` is the absolute
        // invariant; this sentinel matches it so nothing downstream
        // silently allows more than the invariant.
        Self {
            usd: Decimal::new(10_000, 2),
            tokens: None,
            wall_clock_secs: None,
        }
    }
}

/// How the budget's period rolls over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BudgetPeriod {
    /// Each reserve call gets its own ledger row — there is no shared
    /// bucket across calls in "the same invocation". This is by
    /// design: `PerInvocation` models the single costed operation
    /// (one LLM call, one tool use, one scheduler tick) as the unit
    /// of spend. If you want multiple calls to share a rolling bucket,
    /// use [`BudgetPeriod::Rolling24h`] instead. See `period_bounds`
    /// in `crate::runtime::budget` for exact period arithmetic.
    PerInvocation,
    /// Engine-defined 24-hour budget window, **quantised to UTC
    /// midnight** — not a true sliding-window "last 24h" computation.
    /// A user who spends $4.99 at 23:59 and $0.02 at 00:00 therefore
    /// sees the two charges bucket into separate daily ledgers. The
    /// quantised design keeps ledger reads stable under concurrent
    /// reservations; truly-rolling would require per-reservation
    /// timestamp rows and is not implemented. See `period_bounds`
    /// in `crate::runtime::budget`.
    Rolling24h,
    /// Aligned to the calendar `unit` boundary in `tz` (IANA name).
    ///
    /// **Current implementation caveat:** the engine ships without
    /// `chrono-tz`, so the `tz` field is retained on the wire for
    /// forward compatibility but the period arithmetic anchors
    /// everything to **UTC**. Operators choosing a non-UTC timezone
    /// should expect UTC-aligned rollovers until proper IANA support
    /// lands. See `calendar_period` in `crate::runtime::budget`.
    Calendar { tz: String, unit: PeriodUnit },
}

impl BudgetPeriod {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::PerInvocation => "per_invocation",
            Self::Rolling24h => "rolling_24h",
            Self::Calendar { .. } => "calendar",
        }
    }
}

/// Calendar-aligned period unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodUnit {
    Day,
    Week,
    Month,
}

impl PeriodUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

// ── Provenance ──────────────────────────────────────────────

/// Where a [`Budget`] came from — separates defaults from explicit user
/// overrides for audit and UI purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetSource {
    /// Auto-created from `BudgetConfig` defaults on first demand.
    Default,
    /// Set explicitly by a user action (CLI, web UI override dialog).
    UserOverride,
    /// Inherited from a parent scope (sub-thread from parent thread, etc.).
    InheritedFromParent,
}

impl BudgetSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::UserOverride => "user_override",
            Self::InheritedFromParent => "inherited",
        }
    }
}

// ── Budget row ──────────────────────────────────────────────

/// Immutable allocation record: "this scope may spend up to this much per
/// this period, starting at this time."
///
/// A single scope can have multiple active budgets with different periods
/// (e.g. both a rolling-24h cap and a calendar-month cap). The enforcer
/// checks every active budget for each scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: BudgetId,
    pub scope: BudgetScope,
    pub limit: BudgetLimit,
    pub period: BudgetPeriod,
    pub source: BudgetSource,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    /// The user who set this budget (audit). For `Default`-source rows this
    /// equals the scope's owning user.
    pub created_by: String,
}

// ── Ledger ──────────────────────────────────────────────────

/// Running totals for a single period of a single budget.
///
/// One row per `(budget_id, period_start)`. `spent_usd` reflects settled
/// reconciliations; `reserved_usd` reflects in-flight reservations.
/// Available headroom is `limit.usd - (spent_usd + reserved_usd)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLedger {
    pub budget_id: BudgetId,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub spent_usd: Decimal,
    pub reserved_usd: Decimal,
    pub tokens_used: u64,
    pub updated_at: DateTime<Utc>,
}

impl BudgetLedger {
    /// Remaining USD headroom given a limit. Saturates at zero — never
    /// returns a negative value.
    pub fn remaining_usd(&self, limit: &BudgetLimit) -> Decimal {
        let committed = self.spent_usd + self.reserved_usd;
        if committed >= limit.usd {
            Decimal::ZERO
        } else {
            limit.usd - committed
        }
    }

    /// Fraction of the dollar limit already committed (spent + reserved).
    /// Returns 0.0 for an unlimited-like limit (zero-cap inputs return 1.0
    /// so callers see "saturated" rather than divide-by-zero).
    pub fn utilization(&self, limit: &BudgetLimit) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        if limit.usd.is_zero() {
            return 1.0;
        }
        let committed = self.spent_usd + self.reserved_usd;
        committed.to_f64().unwrap_or(0.0) / limit.usd.to_f64().unwrap_or(1.0)
    }
}

// ── Reservations ────────────────────────────────────────────

/// An outstanding (not-yet-reconciled) reservation against one budget.
///
/// The enforcer hands these back to the caller wrapped inside a
/// [`ReservationTicket`]; the caller passes the ticket back to
/// `reconcile` or `release` once the costed operation finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReservation {
    pub id: ReservationId,
    pub budget_id: BudgetId,
    pub reserved_usd: Decimal,
    pub reserved_tokens: u64,
    pub created_at: DateTime<Utc>,
}

/// A successful reservation against the full cascade.
///
/// Holds one [`BudgetReservation`] per scope in the cascade (user, project,
/// mission, thread) so `reconcile`/`release` can update every ledger
/// atomically. Warnings indicate thresholds crossed during this reservation
/// so callers can surface UI hints without a second DB round-trip.
///
/// `actor_user_id` and `thread_id` are captured at reserve time and carried
/// through reconcile/release so the enforcer can emit `budget_events` audit
/// rows without looking the scope back up. A ticket never spans users — all
/// scopes in a single cascade share one owning user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReservationTicket {
    pub reservations: Vec<BudgetReservation>,
    pub warnings: Vec<BudgetWarning>,
    /// The user owning every reservation in this ticket.
    pub actor_user_id: String,
    /// Thread the cascade was reserved against, if the input scopes
    /// included a `Thread` variant. Only used for `budget_events`
    /// audit-row correlation.
    pub thread_id: Option<ThreadId>,
}

impl ReservationTicket {
    pub fn is_empty(&self) -> bool {
        self.reservations.is_empty()
    }

    pub fn total_reserved_usd(&self) -> Decimal {
        self.reservations
            .iter()
            .map(|r| r.reserved_usd)
            .sum::<Decimal>()
    }
}

/// A threshold-crossing notice attached to a successful reservation.
///
/// Emitted when the cascade reserve pushes a scope above a warn/approval
/// threshold. Does not deny — the reservation still succeeded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetWarning {
    pub scope: BudgetScope,
    pub tier: WarningTier,
    pub utilization: f64,
}

/// Warning severity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningTier {
    /// ≥ 50% utilization — log only.
    Info,
    /// ≥ 75% utilization — surface to user via `ThreadEvent::BudgetWarning`.
    Warn,
}

// ── Denial ──────────────────────────────────────────────────

/// Why a reservation was refused. Not an error variant — denial is an
/// expected, first-class outcome that callers branch on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum BudgetDenial {
    /// Dollar cap exhausted. `first_exhausted` names the lowest scope
    /// in the cascade that would go over.
    #[error(
        "budget exhausted at {} scope: {spent} spent of {limit} USD (requested {requested})",
        first_exhausted.kind_str()
    )]
    ExhaustedUsd {
        first_exhausted: BudgetScope,
        limit: Decimal,
        spent: Decimal,
        requested: Decimal,
    },

    /// Token cap exhausted.
    #[error(
        "token budget exhausted at {} scope: {used} of {limit} tokens",
        first_exhausted.kind_str()
    )]
    ExhaustedTokens {
        first_exhausted: BudgetScope,
        limit: u64,
        used: u64,
    },

    /// Over the approval threshold (default 90%) — user must explicitly
    /// extend or approve before this reservation can proceed.
    #[error(
        "budget approval required at {} scope ({utilization:.0}% utilization)",
        scope.kind_str(),
        utilization = utilization * 100.0
    )]
    RequiresApproval {
        scope: BudgetScope,
        utilization: f64,
    },
}

// ── Store errors ────────────────────────────────────────────

/// Errors raised by the [`crate::traits::store::Store`] budget operations.
///
/// Denial is NOT an error — denials return `Err(BudgetOpError::Denied(..))`
/// only in the thin adapter that maps to `EngineError`; the DB-level store
/// uses `Result<Option<ReservationTicket>, BudgetError>` so concurrent
/// oversubscription (the first denying row) is distinguishable from a
/// plumbing failure.
#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget config exceeds hard-cap invariant: {reason}")]
    ExceedsHardCap { reason: String },

    #[error("budget store: {reason}")]
    Store { reason: String },

    #[error("budget ledger corrupted: {reason}")]
    CorruptedLedger { reason: String },

    #[error("unknown budget: {0}")]
    UnknownBudget(BudgetId),
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn scope_round_trips_through_serde() {
        let scope = BudgetScope::Project {
            user_id: "alice".into(),
            project_id: ProjectId::new(),
        };
        let json = serde_json::to_string(&scope).unwrap();
        let back: BudgetScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
    }

    #[test]
    fn scope_id_formats_are_stable_across_kinds() {
        let user = BudgetScope::User {
            user_id: "alice".into(),
        };
        assert_eq!(user.kind_str(), "user");
        assert_eq!(user.scope_id(), "alice");

        let bg = BudgetScope::BackgroundInvocation {
            user_id: "alice".into(),
            kind: BackgroundKind::Heartbeat,
            correlation_id: "tick-42".into(),
        };
        assert_eq!(bg.kind_str(), "background");
        // user_id is embedded so that two users' heartbeat:tick-42
        // budgets can't collide on the scope_id unique indexes.
        assert_eq!(bg.scope_id(), "heartbeat:alice:tick-42");
    }

    #[test]
    fn background_scope_ids_differ_across_users() {
        let alice = BudgetScope::BackgroundInvocation {
            user_id: "alice".into(),
            kind: BackgroundKind::Heartbeat,
            correlation_id: "tick-42".into(),
        };
        let bob = BudgetScope::BackgroundInvocation {
            user_id: "bob".into(),
            kind: BackgroundKind::Heartbeat,
            correlation_id: "tick-42".into(),
        };
        // Same kind + correlation_id; different users must yield
        // distinct scope_ids or the DB UNIQUE would shadow one.
        assert_ne!(alice.scope_id(), bob.scope_id());
    }

    #[test]
    fn remaining_saturates_at_zero_when_over_limit() {
        let limit = BudgetLimit::usd_only(dec!(5.00));
        let ledger = BudgetLedger {
            budget_id: BudgetId::new(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            spent_usd: dec!(6.00),
            reserved_usd: dec!(0),
            tokens_used: 0,
            updated_at: Utc::now(),
        };
        assert_eq!(ledger.remaining_usd(&limit), Decimal::ZERO);
    }

    #[test]
    fn remaining_accounts_for_reservations() {
        let limit = BudgetLimit::usd_only(dec!(5.00));
        let ledger = BudgetLedger {
            budget_id: BudgetId::new(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            spent_usd: dec!(2.00),
            reserved_usd: dec!(1.00),
            tokens_used: 0,
            updated_at: Utc::now(),
        };
        assert_eq!(ledger.remaining_usd(&limit), dec!(2.00));
    }

    #[test]
    fn utilization_is_spent_plus_reserved_over_limit() {
        let limit = BudgetLimit::usd_only(dec!(10));
        let ledger = BudgetLedger {
            budget_id: BudgetId::new(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            spent_usd: dec!(5),
            reserved_usd: dec!(2),
            tokens_used: 0,
            updated_at: Utc::now(),
        };
        let util = ledger.utilization(&limit);
        assert!((util - 0.7).abs() < 1e-9, "unexpected util: {util}");
    }

    #[test]
    fn utilization_zero_limit_returns_saturated() {
        let limit = BudgetLimit::usd_only(Decimal::ZERO);
        let ledger = BudgetLedger {
            budget_id: BudgetId::new(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            spent_usd: Decimal::ZERO,
            reserved_usd: Decimal::ZERO,
            tokens_used: 0,
            updated_at: Utc::now(),
        };
        assert_eq!(ledger.utilization(&limit), 1.0);
    }

    #[test]
    fn scope_user_id_is_consistent_across_variants() {
        let project_id = ProjectId::new();
        let scopes = vec![
            BudgetScope::User {
                user_id: "alice".into(),
            },
            BudgetScope::Project {
                user_id: "alice".into(),
                project_id,
            },
            BudgetScope::BackgroundInvocation {
                user_id: "alice".into(),
                kind: BackgroundKind::RoutineStandard,
                correlation_id: "run-1".into(),
            },
        ];
        for scope in scopes {
            assert_eq!(scope.user_id(), "alice");
        }
    }

    #[test]
    fn period_round_trips_calendar_variant() {
        let period = BudgetPeriod::Calendar {
            tz: "America/Los_Angeles".into(),
            unit: PeriodUnit::Week,
        };
        let json = serde_json::to_string(&period).unwrap();
        let back: BudgetPeriod = serde_json::from_str(&json).unwrap();
        assert_eq!(period, back);
    }

    #[test]
    fn denial_message_identifies_scope_kind() {
        let denial = BudgetDenial::ExhaustedUsd {
            first_exhausted: BudgetScope::User {
                user_id: "alice".into(),
            },
            limit: dec!(5),
            spent: dec!(5),
            requested: dec!(0.01),
        };
        let msg = denial.to_string();
        assert!(msg.contains("user"), "msg was: {msg}");
        assert!(msg.contains("5"), "msg was: {msg}");
    }

    #[test]
    fn ticket_sums_reservations_across_scopes() {
        let mk = |amt: Decimal| BudgetReservation {
            id: ReservationId::new(),
            budget_id: BudgetId::new(),
            reserved_usd: amt,
            reserved_tokens: 0,
            created_at: Utc::now(),
        };
        let ticket = ReservationTicket {
            reservations: vec![mk(dec!(0.10)), mk(dec!(0.10)), mk(dec!(0.10))],
            warnings: vec![],
            actor_user_id: String::new(),
            thread_id: None,
        };
        assert_eq!(ticket.total_reserved_usd(), dec!(0.30));
    }
}
