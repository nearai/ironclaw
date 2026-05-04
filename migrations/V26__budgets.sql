-- Cost-based budgets (issue #2843).
--
-- Introduces three tables backing the `BudgetEnforcer` runtime:
--   1. budgets         — immutable allocation rows (scope, limit, period)
--   2. budget_ledgers  — running totals per (budget, period)
--   3. budget_events   — audit trail of reserve/reconcile/release/deny/approve
--
-- Design principle: atomic reservation via conditional UPDATE against
-- budget_ledgers. The constraint
-- `reserved_usd + spent_usd + requested <= limit_usd` is enforced in SQL,
-- not application code, so concurrent readers cannot oversubscribe.

-- ==================== budgets ====================

CREATE TABLE budgets (
    id UUID PRIMARY KEY,

    -- Denormalised user_id for fast per-user lookups; always equals the
    -- user who owns the scope (every budget is owned by exactly one user).
    user_id TEXT NOT NULL,

    -- 'user' | 'project' | 'mission' | 'thread' | 'background'
    scope_kind TEXT NOT NULL,

    -- Opaque string; meaning depends on scope_kind:
    --   user       -> user_id
    --   project    -> project uuid as string
    --   mission    -> mission uuid as string
    --   thread     -> thread uuid as string
    --   background -> "<kind>:<user_id>:<correlation_id>"
    --                 (e.g. "heartbeat:alice:tick-42"; user_id is embedded
    --                 because correlation_id alone is not globally unique)
    scope_id TEXT NOT NULL,

    -- Primary cap: USD. Zero means "no spend allowed" (hard gate).
    limit_usd NUMERIC(14, 6) NOT NULL,

    -- Optional secondary caps.
    limit_tokens BIGINT,
    limit_wall_clock_secs BIGINT,

    -- 'per_invocation' | 'rolling_24h' | 'calendar'
    period_kind TEXT NOT NULL,

    -- IANA timezone name, only populated for period_kind='calendar'.
    period_tz TEXT,

    -- 'day' | 'week' | 'month', only populated for period_kind='calendar'.
    period_unit TEXT,

    -- 'default' | 'user_override' | 'inherited'
    source TEXT NOT NULL,

    active BOOLEAN NOT NULL DEFAULT TRUE,

    created_at TIMESTAMPTZ NOT NULL,

    -- Audit: which user set this row. Equals user_id for 'default' rows.
    created_by TEXT NOT NULL
);

-- "One active budget per scope+period" uniqueness.
--
-- A table-level UNIQUE(..., period_unit, ...) would NOT enforce this,
-- because PostgreSQL treats NULL as distinct inside UNIQUE constraints
-- (per SQL standard). Periods that leave period_unit NULL
-- (per_invocation, rolling_24h) could then be duplicated for the same
-- scope. Two partial indexes carry the invariant:
--   * calendar periods    — period_unit is non-NULL; include it in key
--   * non-calendar periods — period_unit is NULL; key on period_kind only
-- Both indexes are partial on `active IS TRUE` so historical (inactive)
-- rows remain writable for audit.
CREATE UNIQUE INDEX uq_budgets_calendar_active
    ON budgets (scope_kind, scope_id, period_kind, period_unit)
    WHERE period_kind = 'calendar' AND active IS TRUE;
CREATE UNIQUE INDEX uq_budgets_non_calendar_active
    ON budgets (scope_kind, scope_id, period_kind)
    WHERE period_kind <> 'calendar' AND active IS TRUE;

CREATE INDEX idx_budgets_scope ON budgets (scope_kind, scope_id) WHERE active = TRUE;
CREATE INDEX idx_budgets_user_active ON budgets (user_id) WHERE active = TRUE;

-- ==================== budget_ledgers ====================

CREATE TABLE budget_ledgers (
    -- Composite key: a single budget has one ledger row per period.
    budget_id UUID NOT NULL REFERENCES budgets (id) ON DELETE CASCADE,
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,

    -- Settled spend. Only increases via `reconcile`.
    spent_usd NUMERIC(14, 6) NOT NULL DEFAULT 0,

    -- In-flight reservations. Increases on `reserve`, decreases on
    -- `reconcile` (replaced by actual spent_usd delta) or `release`.
    -- spent_usd + reserved_usd is the committed total.
    reserved_usd NUMERIC(14, 6) NOT NULL DEFAULT 0,

    -- Cumulative tokens used (settled).
    tokens_used BIGINT NOT NULL DEFAULT 0,

    updated_at TIMESTAMPTZ NOT NULL,

    PRIMARY KEY (budget_id, period_start),

    -- Invariant: nothing goes negative.
    CHECK (spent_usd >= 0),
    CHECK (reserved_usd >= 0),
    CHECK (tokens_used >= 0)
);

CREATE INDEX idx_budget_ledgers_period_end ON budget_ledgers (budget_id, period_end DESC);

-- ==================== budget_events ====================

CREATE TABLE budget_events (
    id UUID PRIMARY KEY,

    budget_id UUID NOT NULL REFERENCES budgets (id) ON DELETE CASCADE,

    -- Optional correlation to the thread that triggered this event.
    -- NULL for background-invocation scopes that have no thread yet and
    -- for user-initiated overrides.
    thread_id UUID,

    -- Reservation this event pertains to, when applicable. Set for
    -- `reserve`, `reconcile`, `release`; NULL for pure audit events
    -- (`deny`, `approve`, `override`). Lets auditors correlate a
    -- reserve row with its matching reconcile/release row after the
    -- fact — useful when diagnosing leaked `reserved_usd` headroom.
    reservation_id UUID,

    -- 'reserve' | 'reconcile' | 'release' | 'deny' | 'approve' | 'override'
    event_kind TEXT NOT NULL,

    -- Monetary amount associated with the event (positive for reserve/spent,
    -- NULL when not applicable e.g. approve/override).
    amount_usd NUMERIC(14, 6),

    tokens BIGINT,

    -- Human-readable reason; for denials names the scope-kind that tripped.
    reason TEXT,

    -- Audit: which user caused this event (system id for automatic events).
    actor_user_id TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_budget_events_budget_created
    ON budget_events (budget_id, created_at DESC);

CREATE INDEX idx_budget_events_thread
    ON budget_events (thread_id)
    WHERE thread_id IS NOT NULL;

CREATE INDEX idx_budget_events_reservation
    ON budget_events (reservation_id)
    WHERE reservation_id IS NOT NULL;
