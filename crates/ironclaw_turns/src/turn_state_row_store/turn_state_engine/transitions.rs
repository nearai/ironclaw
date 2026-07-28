//! Run-transition family: lifecycle transitions, lease recovery, lock/queue
//! helpers, and the `TurnRunTransitionPort` trait implementation.
use super::run_record::slot_info_for;
use super::*;
use async_trait::async_trait;

/// Resolve the secret-scrubbed, model-visible detail to attach to a lifecycle
/// event. Only `Failed` events carry the failure record's detail; every other
/// event kind has no failure cause and yields `None`.
fn failure_detail_for_event(
    kind: &TurnEventKind,
    failure: Option<&SanitizedFailure>,
) -> Option<String> {
    (*kind == TurnEventKind::Failed)
        .then(|| failure.and_then(|failure| failure.detail().map(str::to_string)))
        .flatten()
}

/// Clear the runner-lease fields on a record leaving a runner's control
/// (terminal, relinquish, or block). Callers stamp the event cursor and handle
/// the active lock / queue separately — those differ by transition.
fn clear_runner_lease(record: &mut RunRecord) {
    record.runner_id = None;
    record.lease_token = None;
    record.lease_expires_at = None;
}

enum AppliedLoopTransition {
    Applied {
        record: Box<RunRecord>,
        state: Box<TurnRunState>,
        prune_terminal: bool,
    },
    Rejected {
        record: Box<RunRecord>,
        error: TurnError,
    },
}

fn ensure_active_lease(
    record: &RunRecord,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
    now: crate::TurnTimestamp,
) -> Result<(), TurnError> {
    if record.runner_id != Some(runner_id) || record.lease_token != Some(lease_token) {
        return Err(TurnError::LeaseMismatch);
    }
    if record
        .lease_expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        });
    }
    Ok(())
}

struct LeaseExpiredOutcome {
    status: TurnStatus,
    failure: Option<SanitizedFailure>,
    event_kind: TurnEventKind,
    event_detail: Option<String>,
}

/// Resolution of an expired-lease `Running`/`CancelRequested` run (#6284).
enum ExpiredLeaseResolution {
    /// Re-queue to a claimable `Queued` state (checkpointless, safe to re-drive).
    Requeue,
    /// Move to a terminal status. `attach_checkpoint` re-attaches the latest
    /// resumable loop checkpoint when the resulting status is `Failed`.
    Terminal {
        outcome: LeaseExpiredOutcome,
        attach_checkpoint: bool,
    },
}

/// Terminal `Failed(lease_expired)` — used only for a checkpointed run whose
/// re-drive path is the resumable checkpoint. A checkpointless run is never
/// stranded with this reason (see `crash_retry_exhausted_outcome`).
fn lease_expired_failed_outcome() -> LeaseExpiredOutcome {
    let failure = SanitizedFailure::from_trusted_static("lease_expired");
    LeaseExpiredOutcome {
        status: TurnStatus::Failed,
        failure: Some(failure.clone()),
        event_kind: TurnEventKind::Failed,
        event_detail: Some(failure.into_category()),
    }
}

/// Terminal `Failed(crash_retry_exhausted)` — a genuine invariant for a
/// checkpointless run that has exhausted its crash-retry budget (#6284).
fn crash_retry_exhausted_outcome() -> LeaseExpiredOutcome {
    let failure = SanitizedFailure::from_trusted_static("crash_retry_exhausted");
    LeaseExpiredOutcome {
        status: TurnStatus::Failed,
        failure: Some(failure.clone()),
        event_kind: TurnEventKind::Failed,
        event_detail: Some(failure.into_category()),
    }
}

impl Inner {
    /// Decide how an expired-lease run is resolved (#6284).
    ///
    /// - `CancelRequested` → terminal `Cancelled`: cancellation IS a genuine
    ///   invariant, unchanged.
    /// - `Running` that recorded ANY loop checkpoint → terminal
    ///   `Failed(lease_expired)` with the latest resumable checkpoint attached
    ///   (today's behavior, unchanged). The checkpoint means the run already did
    ///   work (a resumable one re-drives from the checkpoint; a non-resumable
    ///   `Final`-only one has no re-drive path but must NOT be re-run from
    ///   scratch — re-drive-from-checkpoint is a separate concern).
    /// - `Running` that recorded NO loop checkpoint (crashed before BeforeModel =
    ///   before any side effect, safe to re-drive) → re-queue to a claimable
    ///   state, UNLESS `claim_count` has reached the crash-retry bound, in which
    ///   case terminal `Failed(crash_retry_exhausted)` (a genuine invariant,
    ///   model-visible — NOT `lease_expired`).
    fn resolve_expired_lease(&self, record: &RunRecord) -> ExpiredLeaseResolution {
        if record.status.get() == TurnStatus::CancelRequested {
            return ExpiredLeaseResolution::Terminal {
                outcome: LeaseExpiredOutcome {
                    status: TurnStatus::Cancelled,
                    failure: None,
                    event_kind: TurnEventKind::Cancelled,
                    event_detail: None,
                },
                attach_checkpoint: false,
            };
        }
        // `Running` with an expired lease. A run that recorded any loop checkpoint
        // already ran past its first checkpoint (did work); keep today's terminal
        // `Failed(lease_expired)` (+ latest resumable checkpoint, if any).
        if self.run_has_loop_checkpoint(&record.scope, record.turn_id, record.run_id) {
            return ExpiredLeaseResolution::Terminal {
                outcome: lease_expired_failed_outcome(),
                attach_checkpoint: true,
            };
        }
        // Checkpoint-less: crashed before any side effect — safe to re-drive,
        // bounded by `claim_count`.
        if record.claim_count >= u64::from(self.limits.max_crash_recovery_reclaims) {
            return ExpiredLeaseResolution::Terminal {
                outcome: crash_retry_exhausted_outcome(),
                attach_checkpoint: false,
            };
        }
        ExpiredLeaseResolution::Requeue
    }

    fn recover_expired_leases(
        &mut self,
        request: RecoverExpiredLeasesRequest,
    ) -> RecoverExpiredLeasesResponse {
        let expired_run_ids = self
            .records
            .iter()
            .filter_map(|(run_id, record)| {
                if !matches!(
                    record.status.get(),
                    TurnStatus::Running | TurnStatus::CancelRequested
                ) {
                    return None;
                }
                if request
                    .scope_filter
                    .as_ref()
                    .is_some_and(|scope| scope != &record.scope)
                {
                    return None;
                }
                if record
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at <= request.now)
                {
                    Some(*run_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut recovered = Vec::with_capacity(expired_run_ids.len());
        for run_id in expired_run_ids {
            let Some(mut record) = self.records.remove(&run_id) else {
                continue;
            };
            match self.resolve_expired_lease(&record) {
                ExpiredLeaseResolution::Requeue => {
                    // #6284: a checkpointless `Running` run whose lease expired
                    // crashed BEFORE its first loop checkpoint (before BeforeModel,
                    // before any side effect) and is always safe to re-drive. Do
                    // NOT strand it terminal `Failed(lease_expired)` — re-queue it
                    // to a claimable `Queued` state so the scheduler re-drives it.
                    // The same-thread active lock is KEPT (a `Queued` run holds its
                    // lock for active-run exclusivity); `claim_count` is preserved
                    // so the crash-retry bound still advances across cycles.
                    let transition = record.status.set(TurnStatus::Queued);
                    self.apply_status_transition(transition, &record);
                    clear_runner_lease(&mut record);
                    record.event_cursor = self.next_cursor();
                    self.update_active_lock(&record, request.now);
                    self.queued_runs.push_back(record.run_id);
                    let state = record.state();
                    // Running → Queued mirrors relinquish's lifecycle event
                    // (`RunnerHeartbeat`); the `LifecyclePublishingTurnStateStore`
                    // wrapper independently classifies the recovered `Queued`
                    // state the same way (`event_kind_for_state`), so the internal
                    // event log and the published stream agree.
                    self.push_event(&record, TurnEventKind::RunnerHeartbeat, None, None);
                    recovered.push(state);
                    self.records.insert(run_id, record);
                }
                ExpiredLeaseResolution::Terminal {
                    outcome,
                    attach_checkpoint,
                } => {
                    let transition = record.status.set(outcome.status);
                    self.apply_status_transition(transition, &record);
                    if attach_checkpoint && record.status.get() == TurnStatus::Failed {
                        record.checkpoint_id = self.latest_resumable_loop_checkpoint(
                            &record.scope,
                            record.turn_id,
                            run_id,
                        );
                    }
                    record.failure = outcome.failure;
                    self.release_terminal_lease(&mut record);
                    let state = record.state();
                    let event_detail =
                        failure_detail_for_event(&outcome.event_kind, record.failure.as_ref());
                    self.push_event(
                        &record,
                        outcome.event_kind,
                        outcome.event_detail,
                        event_detail,
                    );
                    self.mark_terminal(record.run_id);
                    recovered.push(state);
                    self.records.insert(run_id, record);
                }
            }
        }
        self.prune_terminal_records();
        RecoverExpiredLeasesResponse { recovered }
    }

    fn pop_matching_queued_run(&mut self, scope_filter: Option<&TurnScope>) -> Option<TurnRunId> {
        let queued_count = self.queued_runs.len();
        for _ in 0..queued_count {
            let run_id = self.queued_runs.pop_front()?;
            let Some(record) = self.records.get(&run_id) else {
                continue;
            };
            if record.status.get() != TurnStatus::Queued {
                continue;
            }
            let scope_ok = scope_filter.is_none_or(|scope| scope == &record.scope);
            let cap_ok = self.concurrency.can_claim(&slot_info_for(record));
            if scope_ok && cap_ok {
                return Some(run_id);
            }
            self.queued_runs.push_back(run_id);
        }
        None
    }

    fn claim_matching_queued_run(
        &mut self,
        runner_id: crate::TurnRunnerId,
        lease_token: TurnLeaseToken,
        scope_filter: Option<&TurnScope>,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let Some(run_id) = self.pop_matching_queued_run(scope_filter) else {
            return Ok(None);
        };
        let mut record = self.take_record(run_id)?;
        let now = Utc::now();
        let transition = record.status.set(TurnStatus::Running);
        record.runner_id = Some(runner_id);
        record.lease_token = Some(lease_token);
        record.lease_expires_at = Some(self.next_lease_expiry(now));
        record.last_heartbeat_at = Some(now);
        record.claim_count = record.claim_count.saturating_add(1);
        record.event_cursor = self.next_cursor();
        self.update_active_lock(&record, now);
        self.apply_status_transition(transition, &record);
        let claimed = ClaimedTurnRun {
            state: record.state(),
            resolved_run_profile: record.profile.resolved.clone(),
            runner_id,
            lease_token,
        };
        self.push_event(&record, TurnEventKind::RunnerClaimed, None, None);
        self.records.insert(run_id, record);
        Ok(Some(claimed))
    }

    fn remove_queued_run(&mut self, run_id: TurnRunId) {
        self.queued_runs
            .retain(|queued_run_id| *queued_run_id != run_id);
    }

    pub(super) fn resume_turn_once(
        &mut self,
        request: &ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let mut record = self.take_record(request.run_id)?;
        let result = (|| {
            if record.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            let resumable_status = match request.precondition.required_status() {
                Some(required) => record.status.get() == required,
                None => matches!(
                    record.status.get(),
                    TurnStatus::BlockedApproval
                        | TurnStatus::BlockedAuth
                        | TurnStatus::BlockedResource
                ),
            };
            if !resumable_status {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Queued,
                });
            }
            if record.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if record.gate_ref.as_ref() != Some(&request.gate_resolution_ref) {
                return Err(TurnError::InvalidRequest {
                    reason: "gate resolution reference mismatch".to_string(),
                });
            }
            let now = Utc::now();
            let transition = record.status.set(TurnStatus::Queued);
            self.apply_status_transition(transition, &record);
            record.resume_disposition = request.resume_disposition.clone();
            record.gate_ref = None;
            record.credential_requirements = Vec::new();
            record.source_binding_ref = request.source_binding_ref.clone();
            record.reply_target_binding_ref = request.reply_target_binding_ref.clone();
            record.event_cursor = self.next_cursor();
            self.update_active_lock(&record, now);
            self.queued_runs.push_back(record.run_id);
            let response = ResumeTurnResponse {
                run_id: record.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
            };
            self.push_event(&record, TurnEventKind::Resumed, None, None);
            Ok(response)
        })();
        self.records.insert(record.run_id, record);
        result
    }

    pub(super) fn retry_turn_once(
        &mut self,
        request: &RetryTurnRequest,
        admission_limit_provider: &dyn TurnAdmissionLimitProvider,
    ) -> Result<RetryTurnResponse, TurnError> {
        let (
            lock_key,
            source_checkpoint,
            scope,
            actor,
            turn_id,
            profile,
            accepted_message_ref,
            parent_run_id,
            subagent_depth,
            spawn_tree_root_run_id,
            product_context,
        ) = {
            let Some(failed) = self.records.get(&request.run_id) else {
                return Err(TurnError::ScopeNotFound);
            };
            if failed.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            if failed.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if failed.status.get() != TurnStatus::Failed || !self.is_latest_run_for_turn(failed) {
                return Err(TurnError::RunNotRetryable {
                    run_id: request.run_id,
                });
            }
            let source_checkpoint = match failed.checkpoint_id {
                Some(source_checkpoint_id) => {
                    let Some(source_checkpoint) =
                        self.retryable_loop_checkpoint(failed, source_checkpoint_id)
                    else {
                        return Err(TurnError::RunNotRetryable {
                            run_id: request.run_id,
                        });
                    };
                    Some(source_checkpoint)
                }
                None => {
                    if self.run_has_loop_checkpoint(&failed.scope, failed.turn_id, failed.run_id) {
                        return Err(TurnError::RunNotRetryable {
                            run_id: request.run_id,
                        });
                    }
                    None
                }
            };
            (
                TurnActiveLockKey::from(&failed.scope),
                source_checkpoint,
                failed.scope.clone(),
                failed.actor.clone(),
                failed.turn_id,
                failed.profile.clone(),
                failed.accepted_message_ref.clone(),
                failed.parent_run_id,
                failed.subagent_depth,
                failed.spawn_tree_root_run_id,
                failed.product_context.clone(),
            )
        };
        if let Some(response) = self.thread_busy(&lock_key) {
            return Err(TurnError::ThreadBusy(response));
        }

        let now = Utc::now();
        let mut new_run_id = fresh_turn_run_id();
        while self.records.contains_key(&new_run_id) {
            new_run_id = fresh_turn_run_id();
        }
        let admission_class = profile.admission_class.clone();
        if let Err(rejection) = self.reserve_admission(
            new_run_id,
            admission_class,
            &scope,
            &actor,
            admission_limit_provider,
        ) {
            return Err(TurnError::AdmissionRejected(rejection));
        }
        let retry_checkpoint_id = source_checkpoint.as_ref().map(|source_checkpoint| {
            self.link_loop_checkpoint_for_retry(source_checkpoint, new_run_id, now)
        });
        let event_cursor = self.next_cursor();
        let mut record = RunRecord::queued(QueuedRunFields {
            scope,
            actor,
            turn_id,
            run_id: new_run_id,
            profile,
            accepted_message_ref,
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            event_cursor,
            received_at: now,
        });
        record.checkpoint_id = retry_checkpoint_id;
        record.parent_run_id = parent_run_id;
        record.subagent_depth = subagent_depth;
        record.spawn_tree_root_run_id = spawn_tree_root_run_id;
        record.product_context = product_context;
        self.active_locks.insert(
            lock_key.clone(),
            TurnActiveLockRecord {
                key: lock_key,
                run_id: new_run_id,
                status: TurnStatus::Queued,
                lock_version: TurnLockVersion::new(1),
                acquired_at: now,
                updated_at: now,
            },
        );
        self.queued_runs.push_back(new_run_id);
        self.records.insert(new_run_id, record.clone());
        self.push_event(&record, TurnEventKind::Resumed, None, None);
        Ok(RetryTurnResponse {
            run_id: new_run_id,
            status: TurnStatus::Queued,
            event_cursor,
        })
    }

    pub(super) fn request_cancel_once(
        &mut self,
        request: &CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let mut record = self.take_record(request.run_id)?;
        let result = (|| {
            if record.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            if record.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if record.status.get().is_terminal() {
                return Ok(CancelRunResponse {
                    run_id: record.run_id,
                    status: record.status.get(),
                    event_cursor: record.event_cursor,
                    already_terminal: true,
                    actor: Some(record.actor.clone()),
                });
            }
            let (next_status, event_kind) = match record.status.get() {
                TurnStatus::Queued
                | TurnStatus::BlockedApproval
                | TurnStatus::BlockedAuth
                | TurnStatus::BlockedResource
                | TurnStatus::BlockedDependentRun => {
                    (TurnStatus::Cancelled, TurnEventKind::Cancelled)
                }
                TurnStatus::Running | TurnStatus::CancelRequested => {
                    (TurnStatus::CancelRequested, TurnEventKind::CancelRequested)
                }
                status => {
                    return Ok(CancelRunResponse {
                        run_id: record.run_id,
                        status,
                        event_cursor: record.event_cursor,
                        already_terminal: true,
                        actor: Some(record.actor.clone()),
                    });
                }
            };
            let now = Utc::now();
            let transition = record.status.set(next_status);
            self.apply_status_transition(transition, &record);
            if record.status.get().is_terminal() {
                record.failure = None;
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
            } else {
                self.update_active_lock(&record, now);
            }
            record.event_cursor = self.next_cursor();
            let response = CancelRunResponse {
                run_id: record.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
                already_terminal: false,
                actor: Some(record.actor.clone()),
            };
            self.push_event(
                &record,
                event_kind,
                Some(request.reason.category().to_string()),
                None,
            );
            if record.status.get().is_terminal() {
                self.mark_terminal(record.run_id);
            }
            Ok(response)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    /// The identical tail every terminal transition (complete / cancel / fail)
    /// runs after setting the new status and any failure/checkpoint fields:
    /// clear the runner lease, stamp a fresh event cursor, and release the
    /// active-thread lock + queue slot. The caller then reads `state()` and
    /// pushes its own terminal event.
    fn release_terminal_lease(&mut self, record: &mut RunRecord) {
        clear_runner_lease(record);
        record.event_cursor = self.next_cursor();
        self.release_active_lock(record);
        self.remove_queued_run(record.run_id);
    }

    fn cancel_completion_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, Utc::now())?;
            if record.status.get() != TurnStatus::CancelRequested {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Cancelled,
                });
            }
            // CancelRequested → Cancelled: the run was Running when it was incremented at
            // claim; decrement now that the runner is fully releasing it.
            let transition = record.status.set(TurnStatus::Cancelled);
            self.apply_status_transition(transition, &record);
            record.failure = None;
            self.release_terminal_lease(&mut record);
            let state = record.state();
            self.push_event(&record, TurnEventKind::Cancelled, None, None);
            self.mark_terminal(record.run_id);
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    fn terminal_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        status: TurnStatus,
        failure: Option<SanitizedFailure>,
        kind: TurnEventKind,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, Utc::now())?;
            if record.status.get() == TurnStatus::CancelRequested
                || record.status.get().is_terminal()
            {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: status,
                });
            }
            // Old status must be Running (only non-CancelRequested, non-terminal status with a
            // lease); decrement the per-user running counter.
            let transition = record.status.set(status);
            self.apply_status_transition(transition, &record);
            if status == TurnStatus::Failed {
                // Preserve retryability: resolve to the latest resumable
                // checkpoint (BeforeModel/BeforeBlock) so lease-expired and
                // externally-failed runs can be retried, matching their
                // user-facing "Retry the run." summary. Resolves to None when
                // no resumable checkpoint exists, keeping the projected
                // `retryable` flag consistent with `retry_turn` validation
                // (both gate on a resumable-kind checkpoint).
                record.checkpoint_id = self.latest_resumable_loop_checkpoint(
                    &record.scope,
                    record.turn_id,
                    record.run_id,
                );
            }
            record.failure = failure.clone();
            self.release_terminal_lease(&mut record);
            let state = record.state();
            let event_detail = failure_detail_for_event(&kind, failure.as_ref());
            self.push_event(
                &record,
                kind,
                failure.map(SanitizedFailure::into_category),
                event_detail,
            );
            self.mark_terminal(record.run_id);
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    fn apply_validated_loop_exit_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        mapping: LoopExitMapping,
        model_usage: Option<crate::run_profile::LoopModelUsage>,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            if let Err(error) = ensure_active_lease(&record, runner_id, lease_token, Utc::now()) {
                return AppliedLoopTransition::Rejected {
                    record: Box::new(record),
                    error,
                };
            }
            // The loop reports its cumulative per-run usage at every exit (the
            // execution state carries the running total across block/resume
            // legs), so replace rather than accumulate — a block→resume→complete
            // sequence would otherwise double-count the pre-block legs. A run
            // that reported no usage leaves the prior total intact.
            if let Some(usage) = model_usage {
                record.model_usage = Some(usage);
            }
            match mapping {
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed) => {
                    self.complete_claimed_record(record)
                }
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => self
                    .cancel_or_fail_claimed_record(
                        record,
                        SanitizedFailure::from_trusted_static("interrupted_unexpectedly"),
                    ),
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked {
                    checkpoint_id,
                    state_ref,
                    reason,
                    blocked_activity_id,
                }) => self.block_claimed_record(
                    record,
                    checkpoint_id,
                    state_ref,
                    reason,
                    blocked_activity_id,
                ),
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure }) => {
                    self.fail_claimed_record(record, failure)
                }
                LoopExitMapping::RecoveryRequired { failure } => {
                    self.cancel_or_fail_claimed_record(record, failure)
                }
            }
        })();
        self.commit_transition(result)
    }

    fn commit_transition(
        &mut self,
        transition: AppliedLoopTransition,
    ) -> Result<TurnRunState, TurnError> {
        match transition {
            AppliedLoopTransition::Applied {
                record,
                state,
                prune_terminal,
            } => {
                self.records.insert(record.run_id, *record);
                if prune_terminal {
                    self.prune_terminal_records();
                }
                Ok(*state)
            }
            AppliedLoopTransition::Rejected { record, error } => {
                self.records.insert(record.run_id, *record);
                Err(error)
            }
        }
    }

    fn complete_claimed_record(&mut self, mut record: RunRecord) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Completed,
                },
            };
        }
        // Running → Completed: decrement per-user running counter.
        let transition = record.status.set(TurnStatus::Completed);
        self.apply_status_transition(transition, &record);
        record.failure = None;
        self.release_terminal_lease(&mut record);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Completed, None, None);
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn cancel_claimed_record(&mut self, mut record: RunRecord) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::CancelRequested {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Cancelled,
                },
            };
        }
        // CancelRequested → Cancelled via loop exit: the run was Running when incremented at
        // claim; decrement now that the runner is fully releasing it.
        let transition = record.status.set(TurnStatus::Cancelled);
        self.apply_status_transition(transition, &record);
        record.failure = None;
        self.release_terminal_lease(&mut record);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Cancelled, None, None);
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn block_claimed_record(
        &mut self,
        mut record: RunRecord,
        checkpoint_id: TurnCheckpointId,
        state_ref: crate::run_profile::LoopCheckpointStateRef,
        reason: BlockedReason,
        blocked_activity_id: Option<crate::CapabilityActivityId>,
    ) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: reason.status(),
                },
            };
        }
        // Running → Blocked: decrement per-user running counter.
        let now = Utc::now();
        let transition = record.status.set(reason.status());
        self.apply_status_transition(transition, &record);
        record.checkpoint_id = Some(checkpoint_id);
        record.gate_ref = Some(reason.gate_ref().clone());
        record.blocked_activity_id = blocked_activity_id;
        record.credential_requirements = reason.credential_requirements().to_vec();
        clear_runner_lease(&mut record);
        record.event_cursor = self.next_cursor();
        self.record_checkpoint(
            &record,
            checkpoint_id,
            state_ref,
            reason.gate_ref().clone(),
            now,
        );
        self.update_active_lock(&record, now);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Blocked, None, None);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: false,
        }
    }

    fn fail_claimed_record(
        &mut self,
        mut record: RunRecord,
        failure: SanitizedFailure,
    ) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Failed,
                },
            };
        }
        let retry_checkpoint_id =
            self.latest_resumable_loop_checkpoint(&record.scope, record.turn_id, record.run_id);
        // Running → Failed: decrement per-user running counter.
        let transition = record.status.set(TurnStatus::Failed);
        self.apply_status_transition(transition, &record);
        record.checkpoint_id = retry_checkpoint_id;
        record.failure = Some(failure.clone());
        self.release_terminal_lease(&mut record);
        let state = record.state();
        let event_detail = failure.detail().map(str::to_string);
        self.push_event(
            &record,
            TurnEventKind::Failed,
            Some(failure.into_category()),
            event_detail,
        );
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn cancel_or_fail_claimed_record(
        &mut self,
        record: RunRecord,
        failure: SanitizedFailure,
    ) -> AppliedLoopTransition {
        let from = record.status.get();
        match from {
            TurnStatus::Running => {
                // Mirror terminal_transition: a runner-reported failure keeps the
                // run retryable from its latest resumable checkpoint rather than
                // discarding it.
                self.fail_claimed_record(record, failure)
            }
            TurnStatus::CancelRequested => self.cancel_claimed_record(record),
            _ => AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Failed,
                },
            },
        }
    }

    fn runner_failure_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        failure: SanitizedFailure,
    ) -> Result<TurnRunState, TurnError> {
        let record = self.take_record(run_id)?;
        let transition = (|| {
            if let Err(error) = ensure_active_lease(&record, runner_id, lease_token, Utc::now()) {
                return AppliedLoopTransition::Rejected {
                    record: Box::new(record),
                    error,
                };
            }
            self.cancel_or_fail_claimed_record(record, failure)
        })();
        self.commit_transition(transition)
    }

    fn relinquish_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
    ) -> Result<TurnRunState, TurnError> {
        let now = Utc::now();
        let mut record = self.take_record(run_id)?;
        let mut requeue = false;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, now)?;
            if !matches!(
                record.status.get(),
                TurnStatus::Running | TurnStatus::CancelRequested
            ) {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Queued,
                });
            }
            let (new_status, failure, event_kind) = match record.status.get() {
                TurnStatus::Running => {
                    // Running → Queued (relinquish): decrement per-user running counter.
                    requeue = true;
                    (TurnStatus::Queued, None, TurnEventKind::RunnerHeartbeat)
                }
                TurnStatus::CancelRequested => {
                    // CancelRequested → Cancelled (relinquish): the run was Running when
                    // incremented at claim; decrement now.
                    (TurnStatus::Cancelled, None, TurnEventKind::Cancelled)
                }
                _ => unreachable!("status checked above"),
            };
            let transition = record.status.set(new_status);
            self.apply_status_transition(transition, &record);
            record.failure = failure;
            clear_runner_lease(&mut record);
            record.event_cursor = self.next_cursor();
            if requeue {
                self.update_active_lock(&record, now);
            } else {
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
            }
            let state = record.state();
            self.push_event(&record, event_kind, None, None);
            if !requeue {
                self.mark_terminal(record.run_id);
            }
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        if requeue && result.is_ok() {
            self.queued_runs.push_back(run_id);
        }
        if result
            .as_ref()
            .is_ok_and(|state| state.status.is_terminal())
        {
            self.prune_terminal_records();
        }
        result
    }

    fn update_active_lock(&mut self, record: &RunRecord, updated_at: crate::TurnTimestamp) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if let Some(lock) = self.active_locks.get_mut(&lock_key)
            && lock.run_id == record.run_id
        {
            lock.status = record.status.get();
            lock.lock_version = lock.lock_version.incremented();
            lock.updated_at = updated_at;
        }
    }

    fn touch_active_lock(&mut self, record: &RunRecord, updated_at: crate::TurnTimestamp) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if let Some(lock) = self.active_locks.get_mut(&lock_key)
            && lock.run_id == record.run_id
        {
            lock.updated_at = updated_at;
        }
    }

    pub(super) fn thread_busy(&self, lock_key: &TurnActiveLockKey) -> Option<ThreadBusy> {
        let active_lock = self.active_locks.get(lock_key)?;
        let record = self.records.get(&active_lock.run_id)?;
        record
            .status
            .get()
            .keeps_active_lock()
            .then_some(ThreadBusy {
                active_run_id: active_lock.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
            })
    }

    fn is_latest_run_for_turn(&self, record: &RunRecord) -> bool {
        !self.records.values().any(|candidate| {
            candidate.turn_id == record.turn_id && candidate.event_cursor > record.event_cursor
        })
    }

    fn retryable_loop_checkpoint(
        &self,
        record: &RunRecord,
        checkpoint_id: TurnCheckpointId,
    ) -> Option<LoopCheckpointRecord> {
        self.loop_checkpoints
            .get(&checkpoint_id)
            .filter(|checkpoint| {
                checkpoint.scope == record.scope
                    && checkpoint.turn_id == record.turn_id
                    && checkpoint.run_id == record.run_id
                    && matches!(
                        checkpoint.kind,
                        crate::run_profile::LoopCheckpointKind::BeforeModel
                            | crate::run_profile::LoopCheckpointKind::BeforeBlock
                    )
            })
            .cloned()
    }

    fn link_loop_checkpoint_for_retry(
        &mut self,
        source: &LoopCheckpointRecord,
        retry_run_id: TurnRunId,
        created_at: crate::TurnTimestamp,
    ) -> TurnCheckpointId {
        let checkpoint_id = TurnCheckpointId::new();
        self.loop_checkpoints.insert(
            checkpoint_id,
            LoopCheckpointRecord {
                checkpoint_id,
                scope: source.scope.clone(),
                turn_id: source.turn_id,
                run_id: retry_run_id,
                state_ref: source.state_ref.clone(),
                schema_id: source.schema_id.clone(),
                schema_version: source.schema_version,
                kind: source.kind,
                gate_ref: source.gate_ref.clone(),
                created_at,
            },
        );
        checkpoint_id
    }

    /// Whether the run recorded ANY loop checkpoint (of any kind). A run with no
    /// loop checkpoint at all crashed BEFORE its first checkpoint — before
    /// BeforeModel, before any side effect — which is the precise #6284
    /// "safe to re-drive from scratch" condition. A run that recorded a
    /// checkpoint (even a non-resumable `Final` one) already did work and must
    /// NOT be re-driven from scratch.
    pub(super) fn run_has_loop_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> bool {
        self.loop_checkpoints.values().any(|checkpoint| {
            checkpoint.scope == *scope
                && checkpoint.turn_id == turn_id
                && checkpoint.run_id == run_id
        })
    }

    pub(super) fn failed_run_retryable(&self, record: &RunRecord) -> bool {
        record.checkpoint_id.is_some()
            || !self.run_has_loop_checkpoint(&record.scope, record.turn_id, record.run_id)
    }

    fn latest_resumable_loop_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> Option<TurnCheckpointId> {
        self.loop_checkpoints
            .values()
            .filter(|checkpoint| {
                checkpoint.scope == *scope
                    && checkpoint.turn_id == turn_id
                    && checkpoint.run_id == run_id
                    && matches!(
                        checkpoint.kind,
                        crate::run_profile::LoopCheckpointKind::BeforeModel
                            | crate::run_profile::LoopCheckpointKind::BeforeBlock
                    )
            })
            .max_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.checkpoint_id.as_uuid().cmp(&b.checkpoint_id.as_uuid()))
            })
            .map(|checkpoint| checkpoint.checkpoint_id)
    }

    fn record_checkpoint(
        &mut self,
        record: &RunRecord,
        checkpoint_id: TurnCheckpointId,
        state_ref: crate::run_profile::LoopCheckpointStateRef,
        gate_ref: crate::GateRef,
        created_at: crate::TurnTimestamp,
    ) {
        let sequence = self
            .checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.run_id == record.run_id)
            .count()
            .saturating_add(1) as u64;
        self.checkpoints.push(TurnCheckpointRecord {
            checkpoint_id,
            run_id: record.run_id,
            scope: Some(record.scope.clone()),
            sequence,
            status: record.status.get(),
            gate_ref,
            kind: crate::run_profile::LoopCheckpointKind::BeforeBlock,
            state_ref,
            created_at,
        });
    }

    fn release_active_lock(&mut self, record: &RunRecord) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if self
            .active_locks
            .get(&lock_key)
            .is_some_and(|lock| lock.run_id == record.run_id)
        {
            self.active_locks.remove(&lock_key);
        }
    }

    fn mark_terminal(&mut self, run_id: TurnRunId) {
        self.release_admission(run_id);
        self.terminal_runs.push_back(run_id);
    }

    pub(super) fn prune_terminal_records(&mut self) {
        while self.terminal_runs.len() > self.limits.max_terminal_records {
            let Some(run_id) = self.terminal_runs.pop_front() else {
                break;
            };
            if self
                .records
                .get(&run_id)
                .is_some_and(|record| record.status.get().is_terminal())
                && !self
                    .tree_reservations
                    .keys()
                    .any(|reservation| reservation.root_run_id == run_id)
            {
                if let Some(record) = self.records.remove(&run_id) {
                    let turn_id = record.turn_id;
                    if !self
                        .records
                        .values()
                        .any(|record| record.turn_id == turn_id)
                    {
                        self.turns.remove(&turn_id);
                    }
                }
                self.admission_reservations.remove(&run_id);
            }
        }
    }
}

#[async_trait]
impl TurnRunTransitionPort for TurnStateEngine {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.claim_matching_queued_run(
            request.runner_id,
            request.lease_token,
            request.scope_filter.as_ref(),
        )
    }

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut claimed_runs = Vec::new();
        for _ in 0..request.max_runs {
            let Some(claimed) = inner.claim_matching_queued_run(
                request.runner_id,
                TurnLeaseToken::new(),
                request.scope_filter.as_ref(),
            )?
            else {
                break;
            };
            claimed_runs.push(claimed);
        }
        Ok(claimed_runs)
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            if record.status.get() != TurnStatus::Running {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Running,
                });
            }
            record.last_heartbeat_at = Some(now);
            record.lease_expires_at = Some(inner.next_lease_expiry(now));
            record.event_cursor = inner.next_cursor();
            inner.touch_active_lock(&record, now);
            inner.push_event(&record, TurnEventKind::RunnerHeartbeat, None, None);
            Ok(record.event_cursor)
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        Ok(inner.recover_expired_leases(request))
    }

    async fn latest_resumable_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<TurnCheckpointId>, TurnError> {
        let inner = self.lock_inner()?;
        Ok(inner.latest_resumable_loop_checkpoint(scope, turn_id, run_id))
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            if record.status.get() != TurnStatus::Running {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Running,
                });
            }
            request
                .snapshot
                .validate()
                .map_err(|reason| TurnError::InvalidRequest { reason })?;
            if let Some(existing) = &record.resolved_model_route {
                if existing != &request.snapshot {
                    return Err(TurnError::Conflict {
                        reason: "run already has a different resolved model route".to_string(),
                    });
                }
                return Ok(record.state());
            }
            record.resolved_model_route = Some(request.snapshot);
            inner.touch_active_lock(&record, now);
            Ok(record.state())
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let result = {
            let mut inner = self.lock_inner()?;
            let mut record = inner.take_record(request.run_id)?;
            let inner_result = (|| {
                let now = Utc::now();
                ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
                if !matches!(record.status.get(), TurnStatus::Running) {
                    return Err(TurnError::InvalidTransition {
                        from: record.status.get(),
                        to: request.reason.status(),
                    });
                }
                // Running → Blocked: decrement per-user running counter.
                let transition = record.status.set(request.reason.status());
                inner.apply_status_transition(transition, &record);
                record.checkpoint_id = Some(request.checkpoint_id);
                record.gate_ref = Some(request.reason.gate_ref().clone());
                record.blocked_activity_id = None;
                record.credential_requirements = request.reason.credential_requirements().to_vec();
                clear_runner_lease(&mut record);
                record.event_cursor = inner.next_cursor();
                inner.record_checkpoint(
                    &record,
                    request.checkpoint_id,
                    request.state_ref,
                    request.reason.gate_ref().clone(),
                    now,
                );
                inner.update_active_lock(&record, now);
                let state = record.state();
                inner.push_event(&record, TurnEventKind::Blocked, None, None);
                Ok(state)
            })();
            inner.records.insert(record.run_id, record);
            inner_result
        };
        // A run just parked on a gate — track it for durable terminal cleanup
        // and persist so the blocked turn survives a process restart (off the hot
        // path; only fires on a block).
        if result.is_ok() {
            self.mark_gate_persisted(request.run_id);
            self.persist_blocked_state().await;
        }
        result
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.terminal_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                TurnStatus::Completed,
                None,
                TurnEventKind::Completed,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.cancel_completion_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.terminal_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                TurnStatus::Failed,
                Some(request.failure),
                TurnEventKind::Failed,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        // A runner failure can also terminate a run (Running → Failed,
        // CancelRequested → Cancelled), so it must converge the durable snapshot
        // for a gate-persisted run exactly like the other terminal paths —
        // otherwise a restart could resurrect the finished run as live.
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.runner_failure_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                request.failure,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.relinquish_transition(request.run_id, request.runner_id, request.lease_token)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let tracked_before = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.apply_validated_loop_exit_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                request.mapping,
                request.model_usage,
            )
        };
        // A validated loop exit can either park a run on a gate or terminate one
        // (possibly a previously-blocked one). Persist so the durable snapshot
        // stays accurate across restart, and converge the terminal case so a
        // finished run is not rehydrated as live.
        if result.is_ok() && self.block_persistence.is_some() {
            if self.run_is_blocked(request.run_id) {
                self.mark_gate_persisted(request.run_id);
                self.persist_blocked_state().await;
            } else if tracked_before {
                self.persist_blocked_state().await;
                self.clear_gate_persisted(request.run_id);
            }
        }
        result
    }
}
