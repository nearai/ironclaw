//! Row-store configuration limits (`TurnStateStoreLimits`) and their defaults.
use super::DEFAULT_RUNNER_LEASE_TTL_SECONDS;
use chrono::Duration as ChronoDuration;

const MAX_EVENTS: usize = 10_000;
const MAX_TERMINAL_RECORDS: usize = 10_000;
const MAX_IDEMPOTENCY_RECORDS: usize = 10_000;

/// Default crash-retry bound for lease recovery of a checkpointless run (#6284).
/// Small and consistent with the crate's other bounded-retry counters — a
/// checkpointless run may be re-driven a handful of times across crashes before
/// it is terminal-failed with `crash_retry_exhausted`.
const DEFAULT_MAX_CRASH_RECOVERY_RECLAIMS: u32 = 5;

/// Default backpressure bound for the row store's async write-behind window.
/// Sized so a burst of non-critical churn (queued/running/cancel-requested
/// transitions) can coalesce into batched journal appends without an unbounded
/// backlog, while keeping the worst-case crash-loss window small.
const DEFAULT_MAX_PENDING_WRITE_BEHIND_DELTAS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStateStoreLimits {
    pub max_events: usize,
    pub max_terminal_records: usize,
    pub max_idempotency_records: usize,
    pub runner_lease_ttl: ChronoDuration,
    /// Max runs in `TurnStatus::Running` per (tenant_id, owner user_id).
    /// `None` = unlimited (current behavior).
    ///
    /// Cap attribution uses the explicit thread owner when present
    /// (`TurnThreadOwner::ExplicitUser`). For `ActorFallback` owners the
    /// submitting actor's `user_id` is used instead, so those runs are still
    /// capped. Only genuinely `Ownerless` runs (no owner at all) are uncapped
    /// and never contribute to this counter.
    pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `ScheduledTrigger` origin.
    /// `None` = unlimited. Runs without `product_context` are never counted.
    pub max_concurrent_trigger_runs: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `Inbound` or `WebUi` origin.
    /// `None` = unlimited. Runs without `product_context` are never counted.
    pub max_concurrent_conversation_runs: Option<std::num::NonZeroU32>,
    /// Crash-retry bound for lease recovery of a checkpointless run (#6284).
    ///
    /// A run whose lease expires while `Running` with NO resumable loop
    /// checkpoint crashed BEFORE its first checkpoint (before BeforeModel, before
    /// any side effect) and is safe to re-drive: `recover_expired_leases`
    /// re-queues it to a claimable state instead of stranding it terminal
    /// `Failed`. `claim_count` (incremented on every claim) bounds that loop —
    /// once it reaches this value the run is instead terminal-failed with the
    /// genuine-invariant reason `crash_retry_exhausted` (never `lease_expired`),
    /// so a run that keeps crashing pre-checkpoint cannot re-drive forever.
    pub max_crash_recovery_reclaims: u32,
    /// Backpressure bound for the row store's async write-behind mode.
    /// Non-critical transitions return `Ok` immediately after enqueue without
    /// awaiting the durable ack, so the enqueued-but-un-acked delta window is
    /// otherwise unbounded. When the window reaches this cap, the next
    /// non-critical op awaits the OLDEST pending ack before returning —
    /// bounding both memory and the crash-loss window.
    pub max_pending_write_behind_deltas: usize,
}

impl Default for TurnStateStoreLimits {
    fn default() -> Self {
        Self {
            max_events: MAX_EVENTS,
            max_terminal_records: MAX_TERMINAL_RECORDS,
            max_idempotency_records: MAX_IDEMPOTENCY_RECORDS,
            runner_lease_ttl: ChronoDuration::seconds(DEFAULT_RUNNER_LEASE_TTL_SECONDS),
            max_concurrent_runs_per_user: None,
            max_concurrent_trigger_runs: None,
            max_concurrent_conversation_runs: None,
            max_crash_recovery_reclaims: DEFAULT_MAX_CRASH_RECOVERY_RECLAIMS,
            max_pending_write_behind_deltas: DEFAULT_MAX_PENDING_WRITE_BEHIND_DELTAS,
        }
    }
}

impl TurnStateStoreLimits {
    pub fn set_max_events(mut self, max_events: usize) -> Self {
        self.max_events = max_events;
        self
    }

    pub fn set_max_terminal_records(mut self, max_terminal_records: usize) -> Self {
        self.max_terminal_records = max_terminal_records;
        self
    }

    pub fn set_max_idempotency_records(mut self, max_idempotency_records: usize) -> Self {
        self.max_idempotency_records = max_idempotency_records;
        self
    }

    pub fn set_runner_lease_ttl(mut self, runner_lease_ttl: ChronoDuration) -> Self {
        self.runner_lease_ttl = runner_lease_ttl;
        self
    }

    pub fn set_max_concurrent_runs_per_user(
        mut self,
        max_concurrent_runs_per_user: std::num::NonZeroU32,
    ) -> Self {
        self.max_concurrent_runs_per_user = Some(max_concurrent_runs_per_user);
        self
    }

    pub fn clear_max_concurrent_runs_per_user(mut self) -> Self {
        self.max_concurrent_runs_per_user = None;
        self
    }

    pub fn set_max_concurrent_trigger_runs(
        mut self,
        max_concurrent_trigger_runs: std::num::NonZeroU32,
    ) -> Self {
        self.max_concurrent_trigger_runs = Some(max_concurrent_trigger_runs);
        self
    }

    pub fn clear_max_concurrent_trigger_runs(mut self) -> Self {
        self.max_concurrent_trigger_runs = None;
        self
    }

    pub fn set_max_concurrent_conversation_runs(
        mut self,
        max_concurrent_conversation_runs: std::num::NonZeroU32,
    ) -> Self {
        self.max_concurrent_conversation_runs = Some(max_concurrent_conversation_runs);
        self
    }

    pub fn clear_max_concurrent_conversation_runs(mut self) -> Self {
        self.max_concurrent_conversation_runs = None;
        self
    }

    pub fn set_max_crash_recovery_reclaims(mut self, max_crash_recovery_reclaims: u32) -> Self {
        self.max_crash_recovery_reclaims = max_crash_recovery_reclaims;
        self
    }

    pub fn set_max_pending_write_behind_deltas(
        mut self,
        max_pending_write_behind_deltas: usize,
    ) -> Self {
        self.max_pending_write_behind_deltas = max_pending_write_behind_deltas;
        self
    }
}
