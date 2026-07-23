use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, ThreadId, Timestamp};
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::{
    TriggerError, TriggerFire, TriggerId, TriggerMaterializedPrompt, TriggerRecord,
    TriggerRunHistoryStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedTriggerSubmitRequest {
    fire: TriggerFire,
    materialized_prompt: TriggerMaterializedPrompt,
    received_at: Timestamp,
}

impl TrustedTriggerSubmitRequest {
    /// Create a sealed trusted trigger submit request.
    ///
    /// `materialized_prompt` must have been produced from the exact `fire`
    /// supplied here. The worker is the only crate allowed to pair those values,
    /// so downstream trusted submitters cannot forge or mix prompt content and
    /// trigger identity.
    pub(crate) fn new(
        fire: TriggerFire,
        materialized_prompt: TriggerMaterializedPrompt,
        received_at: Timestamp,
    ) -> Self {
        Self {
            fire,
            materialized_prompt,
            received_at,
        }
    }

    pub fn fire(&self) -> &TriggerFire {
        &self.fire
    }

    pub fn materialized_prompt(&self) -> &TriggerMaterializedPrompt {
        &self.materialized_prompt
    }

    pub fn content_ref(&self) -> &crate::TriggerInboundContentRef {
        self.materialized_prompt.content_ref()
    }

    pub fn received_at(&self) -> Timestamp {
        self.received_at
    }

    pub fn into_parts(self) -> (TriggerFire, TriggerMaterializedPrompt, Timestamp) {
        (self.fire, self.materialized_prompt, self.received_at)
    }

    /// Test-only constructor that bypasses the `pub(crate)` seal.
    ///
    /// Production code always creates submit requests inside the trigger worker
    /// (`due_fire.rs`), which is the only caller allowed to pair a `TriggerFire`
    /// with its materialized prompt. This helper lets downstream crates (e.g.
    /// `ironclaw_conversations`) test their `TrustedTriggerFireSubmitter` impls
    /// without pulling in the full worker. Gated on `test-support` feature so
    /// it ships zero bytes in production binaries.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_test(
        fire: TriggerFire,
        materialized_prompt: TriggerMaterializedPrompt,
        received_at: Timestamp,
    ) -> Self {
        Self::new(fire, materialized_prompt, received_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedTriggerFireSubmitOutcome {
    Accepted {
        run_id: TurnRunId,
        submitted_at: Timestamp,
        /// Scope of the submitted run, available for post-submit hooks (e.g.
        /// triggered-run delivery) that need to poll the run state.
        turn_scope: TurnScope,
    },
    Replayed {
        original_run_id: TurnRunId,
        replayed_at: Timestamp,
        /// Canonical thread id for the replayed fire.
        ///
        /// The submission path resolves conversation binding before determining
        /// whether a fire is new or replayed, so the canonical `ThreadId` is
        /// available at this point. `None` means no canonical thread was
        /// resolved.
        thread_id: Option<ThreadId>,
    },
}

#[async_trait]
pub trait TrustedTriggerFireSubmitter: Send + Sync {
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerAcceptedFireSettlement {
    pub fire: TriggerFire,
    pub run_id: TurnRunId,
    pub turn_scope: TurnScope,
}

#[async_trait]
pub trait TriggerFireSettlementObserver: Send + Sync {
    async fn on_accepted_fire_settled(&self, event: TriggerAcceptedFireSettlement);
}

#[derive(Debug, Default)]
pub struct NoopTriggerFireSettlementObserver;

#[async_trait]
impl TriggerFireSettlementObserver for NoopTriggerFireSettlementObserver {
    async fn on_accepted_fire_settled(&self, _event: TriggerAcceptedFireSettlement) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerActiveRunStateRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: TurnRunId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerActiveRunState {
    Missing,
    Nonterminal,
    /// The run is parked on a gate that needs human interaction (tool-approval
    /// or auth) which an unattended scheduled fire cannot satisfy. Cleanup keeps
    /// the active fire locked until the underlying turn reaches a terminal state;
    /// clearing it earlier would need to atomically terminate the turn as well,
    /// otherwise the run could later resume after failed trigger history was
    /// recorded.
    Blocked {
        kind: BlockedActiveRunKind,
    },
    Terminal {
        status: TriggerRunHistoryStatus,
    },
}

/// Why a blocked active run is parked, at the granularity user-facing read
/// surfaces need ("waiting for your approval" vs "reconnect an account").
/// In-memory lookup vocabulary only — never persisted (#5886).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedActiveRunKind {
    Approval,
    Auth,
    Other,
}

/// Lookup that resolves every run as [`TriggerActiveRunState::Missing`], for
/// callers that have no run-state source (mirrors `NoopTriggerCreateHook`).
/// Consumers treat `Missing` conservatively, so this never fabricates a hold.
#[derive(Debug, Default)]
pub struct MissingTriggerActiveRunLookup;

#[async_trait]
pub trait TriggerActiveRunLookup: Send + Sync {
    /// Resolve a single active-run state.
    ///
    /// The default composition-root implementation reads a full
    /// `TurnPersistenceSnapshot` for each call, so batch-oriented
    /// implementations should prefer overriding `active_run_states` and
    /// handling single-record lookups through the shared batch path when
    /// they need to amortize snapshot reads.
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError>;

    /// Resolve active run states for a batch of requests.
    ///
    /// Implementations must return exactly one result per request, in the same
    /// order as the input vector. Callers use positional matching to preserve
    /// per-trigger cleanup report semantics across batched backend reads.
    async fn active_run_states(
        &self,
        requests: Vec<TriggerActiveRunStateRequest>,
    ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
        let mut results = Vec::with_capacity(requests.len());
        for request in requests {
            results.push(self.active_run_state(request).await);
        }
        results
    }
}

#[async_trait]
impl TriggerActiveRunLookup for MissingTriggerActiveRunLookup {
    async fn active_run_state(
        &self,
        _request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError> {
        Ok(TriggerActiveRunState::Missing)
    }
}

/// Display cap for `active_hold.elapsed_occurrences`; shared by every read
/// surface so they all render "99+" identically instead of drifting (#5886).
pub const ACTIVE_HOLD_ELAPSED_OCCURRENCES_CAP: u32 = 99;

/// Default timeout for a standalone `active_holds_for_records` caller (one
/// not already deriving a remaining-budget duration from an outer deadline).
/// Its one real caller (`builtin.trigger_list`) is a model-facing capability
/// inside a live agent turn, and `active_hold` is display-only decoration —
/// so this must fail fast toward the "omit active_hold" fallback rather than
/// block the turn on a slow or wedged snapshot source (#5886).
pub const ACTIVE_HOLD_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// User-facing reason a trigger's active fire is holding the poller back, at
/// the granularity read surfaces need ("waiting for your approval" vs
/// "reconnect an account") (#5886).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveHoldReason {
    Approval,
    Auth,
    InProgress,
    Other,
}

/// Display-only projection of why a trigger is held plus how many scheduled
/// occurrences have elapsed since the hold began. Owned here because it is
/// derived entirely from `TriggerRecord` + `TriggerActiveRunState`; callers
/// only map this to their own wire type (#5886).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveHoldProjection {
    pub reason: ActiveHoldReason,
    pub since: Option<Timestamp>,
    pub elapsed_occurrences: Option<u32>,
    pub elapsed_occurrences_capped: bool,
}

/// Derive the hold projection for a record whose active fire resolved to
/// `run_state`, shared by every read surface that renders `active_hold`
/// (#5886).
///
/// `run_state` is `None` when the record has claimed a fire slot
/// (`active_fire_slot.is_some()`) but has not yet recorded an
/// `active_run_ref` — the async claim-to-accept window in
/// `worker::due_fire::process_claimed_fire`. There is no run to look up yet,
/// so this resolves directly to `ActiveHoldReason::Other` instead of the
/// caller silently omitting the hold for that window (#5886).
///
/// `elapsed_occurrences` counts elapsed schedule occurrences since `since` —
/// it is NOT a count of runs the poller attempted or skipped. It keeps
/// accruing while the trigger is paused, or whenever the poller isn't
/// running, because it is derived purely from wall-clock cron slots, not
/// from poller activity.
///
/// Terminal runs are omitted (cleanup will release the fire) and `Missing`
/// stays conservative (possibly a stale snapshot) — omission always means
/// "show nothing", never an error.
pub fn active_hold_projection(
    record: &TriggerRecord,
    run_state: Option<TriggerActiveRunState>,
    now: Timestamp,
) -> Option<ActiveHoldProjection> {
    let reason = match run_state {
        None => ActiveHoldReason::Other,
        Some(TriggerActiveRunState::Blocked { kind }) => match kind {
            BlockedActiveRunKind::Approval => ActiveHoldReason::Approval,
            BlockedActiveRunKind::Auth => ActiveHoldReason::Auth,
            BlockedActiveRunKind::Other => ActiveHoldReason::Other,
        },
        Some(TriggerActiveRunState::Nonterminal) => ActiveHoldReason::InProgress,
        Some(TriggerActiveRunState::Terminal { .. }) | Some(TriggerActiveRunState::Missing) => {
            return None;
        }
    };
    let since = record.active_fire_slot;
    let elapsed = since.and_then(|slot| {
        match record.schedule.elapsed_occurrences_between(
            slot,
            now,
            ACTIVE_HOLD_ELAPSED_OCCURRENCES_CAP,
        ) {
            Ok(count) => Some(count),
            Err(error) => {
                // silent-ok: display-only elapsed-occurrence counter; a
                // malformed stored schedule must not fail a read surface
                // (#5886).
                tracing::debug!(%error, "elapsed-occurrence derivation failed for active hold");
                None
            }
        }
    });
    Some(ActiveHoldProjection {
        reason,
        since,
        elapsed_occurrences: elapsed.map(|s| s.count),
        elapsed_occurrences_capped: elapsed.map(|s| s.capped).unwrap_or_default(),
    })
}

/// Batch-resolve active-run states for `records` and derive each one's
/// [`ActiveHoldProjection`], shared by every read surface that renders
/// `active_hold` (#5886). Records with a claimed fire slot but no
/// `active_run_ref` yet skip the lookup entirely (see
/// [`active_hold_projection`]); records with both resolve through
/// `active_run_lookup`, timeout-wrapped so a slow snapshot source cannot
/// delay or fail the caller. Lookup failure and timeout both degrade to "no
/// hold" per-record — this is a display-only projection.
pub async fn active_holds_for_records(
    active_run_lookup: &dyn TriggerActiveRunLookup,
    records: &[TriggerRecord],
    now: Timestamp,
    timeout: Duration,
) -> HashMap<TriggerId, ActiveHoldProjection> {
    let mut holds = HashMap::new();
    let mut requests = Vec::new();
    let mut requested_records = Vec::new();
    for record in records {
        let Some(fire_slot) = record.active_fire_slot else {
            continue;
        };
        match record.active_run_ref {
            Some(run_id) => {
                requests.push(TriggerActiveRunStateRequest {
                    tenant_id: record.tenant_id.clone(),
                    trigger_id: record.trigger_id,
                    fire_slot,
                    run_id,
                });
                requested_records.push(record);
            }
            None => {
                if let Some(hold) = active_hold_projection(record, None, now) {
                    holds.insert(record.trigger_id, hold);
                }
            }
        }
    }
    if requests.is_empty() {
        return holds;
    }
    let Ok(states) =
        tokio::time::timeout(timeout, active_run_lookup.active_run_states(requests)).await
    else {
        // silent-ok: display-only active-hold projection; a slow snapshot
        // source must not fail or delay the caller (#5886).
        tracing::debug!("active-run lookup timed out while deriving active holds");
        return holds;
    };
    for (record, state) in requested_records.into_iter().zip(states) {
        let state = match state {
            Ok(state) => state,
            Err(error) => {
                // silent-ok: display-only active-hold projection; snapshot
                // lookup failure must not fail the caller (#5886).
                tracing::debug!(%error, "active-run lookup failed while deriving an active hold");
                continue;
            }
        };
        if let Some(hold) = active_hold_projection(record, Some(state), now) {
            holds.insert(record.trigger_id, hold);
        }
    }
    holds
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use super::*;
    use crate::{TriggerSchedule, TriggerSourceKind, TriggerState};

    fn test_record(
        active_fire_slot: Option<Timestamp>,
        active_run_ref: Option<TurnRunId>,
    ) -> TriggerRecord {
        let now = Utc::now();
        TriggerRecord {
            trigger_id: TriggerId::new(),
            tenant_id: TenantId::new("tenant-alpha").expect("valid tenant"),
            creator_user_id: UserId::new("user-alpha").expect("valid user"),
            agent_id: Some(AgentId::new("agent-alpha").expect("valid agent")),
            project_id: Some(ProjectId::new("project-alpha").expect("valid project")),
            name: "daily".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::Cron {
                expression: "0 9 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            prompt: "check mail".to_string(),
            delivery_target: None,
            automation: crate::TriggerAutomation::UserSchedule,
            state: TriggerState::Scheduled,
            next_run_at: now,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot,
            active_run_ref,
            created_at: now,
        }
    }

    #[test]
    fn active_hold_projection_maps_blocked_and_nonterminal_states() {
        let now = Utc::now();
        let record = test_record(
            Some(now - chrono::Duration::days(3)),
            Some(TurnRunId::new()),
        );

        let hold = active_hold_projection(
            &record,
            Some(TriggerActiveRunState::Blocked {
                kind: BlockedActiveRunKind::Approval,
            }),
            now,
        )
        .expect("blocked approval yields a hold");
        assert_eq!(hold.reason, ActiveHoldReason::Approval);
        assert_eq!(hold.since, record.active_fire_slot);
        assert!(hold.elapsed_occurrences.is_some());

        let hold = active_hold_projection(&record, Some(TriggerActiveRunState::Nonterminal), now)
            .expect("nonterminal yields a hold");
        assert_eq!(hold.reason, ActiveHoldReason::InProgress);
    }

    #[test]
    fn active_hold_projection_omits_missing_and_terminal() {
        let now = Utc::now();
        let record = test_record(Some(now), Some(TurnRunId::new()));
        assert!(
            active_hold_projection(&record, Some(TriggerActiveRunState::Missing), now).is_none()
        );
        assert!(
            active_hold_projection(
                &record,
                Some(TriggerActiveRunState::Terminal {
                    status: TriggerRunHistoryStatus::Ok,
                }),
                now,
            )
            .is_none()
        );
    }

    /// A claimed-but-not-yet-accepted record (`active_fire_slot` set,
    /// `active_run_ref` still `None`) has no run to look up. This must resolve
    /// directly to `Other` instead of the caller silently reporting no hold
    /// for the async claim-to-accept window (#5886).
    #[test]
    fn active_hold_projection_claimed_but_unaccepted_resolves_to_other() {
        let now = Utc::now();
        let record = test_record(Some(now), None);
        let hold = active_hold_projection(&record, None, now)
            .expect("claimed-but-unaccepted fire yields a hold");
        assert_eq!(hold.reason, ActiveHoldReason::Other);
        assert_eq!(hold.since, record.active_fire_slot);
    }

    /// Batching must route claimed-but-unaccepted records straight to
    /// [`active_hold_projection`] without ever calling the lookup — there is
    /// no `active_run_ref` to build a request from (#5886).
    #[tokio::test]
    async fn active_holds_for_records_skips_lookup_for_claimed_but_unaccepted() {
        struct PanicLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for PanicLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                panic!("lookup must not be called for a claimed-but-unaccepted record");
            }
        }

        let now = Utc::now();
        let record = test_record(Some(now), None);
        let holds = active_holds_for_records(
            &PanicLookup,
            std::slice::from_ref(&record),
            now,
            Duration::from_secs(5),
        )
        .await;
        let hold = holds.get(&record.trigger_id).expect("hold present");
        assert_eq!(hold.reason, ActiveHoldReason::Other);
    }

    #[tokio::test]
    async fn active_holds_for_records_degrades_on_lookup_error() {
        struct ErrLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for ErrLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                Err(TriggerError::NotFound)
            }
        }

        let now = Utc::now();
        let record = test_record(Some(now), Some(TurnRunId::new()));
        let holds = active_holds_for_records(
            &ErrLookup,
            std::slice::from_ref(&record),
            now,
            Duration::from_secs(5),
        )
        .await;
        assert!(holds.is_empty());
    }

    /// The batch path zips `requested_records` against the lookup's returned
    /// states positionally, trusting `active_run_states`'s "same order as
    /// input" contract. This pins that a reordering-safe backend still
    /// attributes each state to the correct trigger, not swapped (#5886).
    #[tokio::test]
    async fn active_holds_for_records_maps_batch_results_by_position_not_identity() {
        struct PositionalLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for PositionalLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                panic!("batch caller must use active_run_states, not per-request active_run_state");
            }

            async fn active_run_states(
                &self,
                requests: Vec<TriggerActiveRunStateRequest>,
            ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
                requests
                    .into_iter()
                    .enumerate()
                    .map(|(index, _)| {
                        Ok(if index == 0 {
                            TriggerActiveRunState::Blocked {
                                kind: BlockedActiveRunKind::Approval,
                            }
                        } else {
                            TriggerActiveRunState::Nonterminal
                        })
                    })
                    .collect()
            }
        }

        let now = Utc::now();
        let record_a = test_record(
            Some(now - chrono::Duration::days(1)),
            Some(TurnRunId::new()),
        );
        let record_b = test_record(
            Some(now - chrono::Duration::hours(2)),
            Some(TurnRunId::new()),
        );

        let holds = active_holds_for_records(
            &PositionalLookup,
            &[record_a.clone(), record_b.clone()],
            now,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(
            holds.get(&record_a.trigger_id).map(|hold| hold.reason),
            Some(ActiveHoldReason::Approval),
            "first record in the batch must map to the first returned state"
        );
        assert_eq!(
            holds.get(&record_b.trigger_id).map(|hold| hold.reason),
            Some(ActiveHoldReason::InProgress),
            "second record in the batch must map to the second returned state, not swapped"
        );
    }

    /// `active_holds_for_records` wraps the lookup in a `tokio::time::timeout`
    /// so a slow snapshot source degrades to "no hold" instead of hanging or
    /// failing the caller. This exercises the deadline-exceeded branch
    /// specifically, mirroring the already-tested lookup-error degrade (#5886).
    #[tokio::test]
    async fn active_holds_for_records_degrades_on_timeout() {
        struct SlowLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for SlowLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(TriggerActiveRunState::Blocked {
                    kind: BlockedActiveRunKind::Approval,
                })
            }
        }

        let now = Utc::now();
        let record = test_record(Some(now), Some(TurnRunId::new()));
        let holds = active_holds_for_records(
            &SlowLookup,
            std::slice::from_ref(&record),
            now,
            Duration::from_millis(10),
        )
        .await;
        assert!(holds.is_empty(), "timed-out lookup must degrade to no hold");
    }

    /// A malformed persisted schedule (shouldn't happen given create-time
    /// validation, but storage can be corrupted) must not panic or fail the
    /// caller: `elapsed_occurrences_between` errors, and
    /// `active_hold_projection` silently omits `elapsed_occurrences` while
    /// still surfacing the hold (#5886).
    #[test]
    fn active_hold_projection_omits_elapsed_occurrences_on_malformed_schedule() {
        let now = Utc::now();
        let mut record = test_record(
            Some(now - chrono::Duration::days(1)),
            Some(TurnRunId::new()),
        );
        record.schedule = TriggerSchedule::Cron {
            expression: "not a valid cron expression".to_string(),
            timezone: "UTC".to_string(),
        };

        let hold = active_hold_projection(&record, Some(TriggerActiveRunState::Nonterminal), now)
            .expect("malformed schedule must still yield a hold");
        assert_eq!(hold.reason, ActiveHoldReason::InProgress);
        assert_eq!(hold.since, record.active_fire_slot);
        assert!(
            hold.elapsed_occurrences.is_none(),
            "elapsed-occurrence derivation must degrade silently, not surface a stale count"
        );
    }

    /// An empty batch must return immediately without invoking the lookup at
    /// all, mirroring the claimed-but-unaccepted "never calls the lookup"
    /// pattern above (#5886).
    #[tokio::test]
    async fn active_holds_for_records_empty_slice_skips_lookup() {
        struct PanicLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for PanicLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                panic!("lookup must not be called for an empty batch");
            }
        }

        let holds =
            active_holds_for_records(&PanicLookup, &[], Utc::now(), Duration::from_secs(5)).await;
        assert!(holds.is_empty());
    }
}
