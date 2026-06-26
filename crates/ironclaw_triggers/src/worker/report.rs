use ironclaw_host_api::{TenantId, Timestamp};
use ironclaw_turns::TurnRunId;

use crate::TriggerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerTickReport {
    pub now: Timestamp,
    pub active_records: usize,
    pub due_records: usize,
    pub results: Vec<TriggerPollerFireReport>,
}

impl TriggerPollerTickReport {
    pub(super) fn new(now: Timestamp) -> Self {
        Self {
            now,
            active_records: 0,
            due_records: 0,
            results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerFireReport {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub outcome: TriggerPollerFireOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerPollerFireOutcome {
    Submitted {
        run_id: TurnRunId,
    },
    Replayed {
        original_run_id: TurnRunId,
    },
    RetryableFailed {
        reason: TriggerPollerFailureReason,
    },
    PermanentFailed {
        reason: TriggerPollerFailureReason,
    },
    /// A `Once` trigger hit a permanent pre-submission failure and was
    /// completed so it cannot re-fire the same terminal schedule slot forever.
    OncePermanentFailed {
        reason: TriggerPollerFailureReason,
    },
    ClearedTerminalActive {
        run_id: TurnRunId,
    },
    /// Reserved for a future cleanup path that can atomically terminate a gated
    /// run and clear its active fire in one operation. Current active cleanup
    /// keeps blocked approval/auth runs locked until they become terminal.
    ClearedBlockedActive {
        run_id: TurnRunId,
    },
    ActiveRunLookupFailed {
        run_id: TurnRunId,
        reason: TriggerPollerFailureReason,
    },
    SkippedAlreadyCleared {
        run_id: TurnRunId,
    },
    SkippedAlreadyActive {
        active_fire_slot: Timestamp,
        active_run_ref: Option<TurnRunId>,
    },
    DueFireFailed {
        reason: TriggerPollerFailureReason,
    },
    SkippedNotDue,
    SkippedNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerPollerFailureReason {
    Backend,
    InvalidTriggerId,
    InvalidFireIdentityComponent,
    InvalidRecord,
    InvalidPollerConfig,
    InvalidSchedule,
    InvalidMaterialization,
    BlockedMaterialization,
    NotFound,
    SourceNoFire,
    ActiveRunLookup,
}
