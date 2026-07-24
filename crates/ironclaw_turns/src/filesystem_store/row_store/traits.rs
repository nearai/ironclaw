use async_trait::async_trait;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::UserId;
use tracing::Instrument;

use crate::{
    CancelRunRequest, CancelRunResponse, EventCursor, GetLoopCheckpointRequest, GetRunStateRequest,
    LoopCheckpointRecord, LoopCheckpointStore, PutLoopCheckpointRequest, ResumeTurnRequest,
    ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileResolver,
    SpawnTreeReservation, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse,
    TurnAdmissionPolicy, TurnError, TurnEventPage, TurnEventProjectionSource, TurnRunId,
    TurnRunRecord, TurnRunState, TurnScope, TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimRunsRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest,
        HeartbeatRequest, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, RelinquishRunRequest,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

use super::{
    FilesystemTurnStateRowStore, PendingRowCommit, RunStateTransitionTarget,
    delta::{
        SnapshotDelta, blocked_run_targeted_delta, claimed_run_targeted_delta, full_snapshot_delta,
        retry_turn_full_delta, row_store_durable_delta, run_state_targeted_delta,
        run_state_with_idempotency_targeted_delta, submit_turn_targeted_delta,
    },
    turn_state_write_span,
};
use crate::filesystem_store::{
    profile_resolver::PreResolvedRunProfileResolver, projection, runner_lease::RunnerLeaseOverlay,
};

#[async_trait]
impl<F> TurnStateStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution = run_profile_resolver
            .resolve_run_profile(crate::RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..crate::RunProfileResolutionRequest::interactive_default()
            })
            .await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        let max_idempotency_records = self.limits.max_idempotency_records;
        let idempotency_key = request.idempotency_key.clone();
        let outcome = self
            .apply_with_targeted_delta(
                RunnerLeaseOverlay::None,
                |store| {
                    let request = request.clone();
                    let pre_resolved = pre_resolved.clone();
                    async move {
                        // `ThreadBusy` is transient — the engine persists nothing
                        // for it — so surface it as the OUTER error to keep the
                        // stale-cache refresh/retry firing. Every other outcome is
                        // wrapped in `Ok` so its durable state is ALWAYS committed:
                        // `Accepted` (turn/run/lock/idempotency/events) and the
                        // rejections the engine records an idempotency record for
                        // (`AdmissionRejected`, invalid request, requested_run_id
                        // conflict, profile-resolution error). The plain `Err` path
                        // discarded that record, so a duplicate submit re-evaluated
                        // instead of replaying the same rejection (#6263).
                        match store
                            .submit_turn(request, admission_policy, &pre_resolved)
                            .await
                        {
                            Err(TurnError::ThreadBusy(busy)) => Err(TurnError::ThreadBusy(busy)),
                            other => Ok(other),
                        }
                    }
                },
                move |snapshot, latest_event_cursor, store, result| match result {
                    Ok(response) => {
                        if snapshot.idempotency_records.len() >= max_idempotency_records {
                            return full_snapshot_delta(snapshot, store);
                        }
                        submit_turn_targeted_delta(
                            snapshot,
                            latest_event_cursor,
                            store,
                            response,
                            &idempotency_key,
                        )
                    }
                    // Rejection: persist the idempotency record the engine recorded
                    // so a duplicate submit replays the same rejection and the
                    // envelope is reconstructable after reopen. A rejection's only
                    // durable change is that record, so a full diff yields exactly
                    // it.
                    Err(_) => full_snapshot_delta(snapshot, store),
                },
            )
            .instrument(turn_state_write_span(
                "submit_turn",
                Some(&request.scope),
                request.requested_run_id.as_ref(),
            ))
            .await;
        outcome?
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let max_idempotency_records = self.limits.max_idempotency_records;
        let scope = request.scope.clone();
        let run_id = request.run_id;
        self.apply_with_targeted_delta(
            RunnerLeaseOverlay::None,
            |store| {
                let request = request.clone();
                async move { store.resume_turn(request).await }
            },
            move |snapshot, latest_event_cursor, store, response| {
                if snapshot.idempotency_records.len() >= max_idempotency_records {
                    return full_snapshot_delta(snapshot, store);
                }
                run_state_with_idempotency_targeted_delta(
                    snapshot,
                    latest_event_cursor,
                    store,
                    response.run_id,
                    &scope,
                    crate::TurnIdempotencyOperationKind::Resume,
                )
            },
        )
        .instrument(turn_state_write_span(
            "resume_turn",
            Some(&request.scope),
            Some(&run_id),
        ))
        .await
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        let scope = request.scope.clone();
        let run_id = request.run_id;
        // Wrap the engine result so its durable side effects are ALWAYS committed,
        // matching the engine's own persistence: on success a new Queued run plus
        // a linked loop checkpoint, and on `ThreadBusy` / `RunNotRetryable` a
        // persisted (non-replayable) retry idempotency record. The plain `apply`
        // path discarded every state change on an `Err`, so a rejected retry
        // silently dropped the idempotency record the engine had recorded, and a
        // successful retry's new loop checkpoint was clobbered by
        // `preserve_loop_checkpoints` (#6263). `retry_turn_full_delta` captures
        // both without that clobber. `AdmissionRejected` persists nothing, so its
        // diff is empty and nothing is written.
        let outcome = self
            .apply_with_targeted_delta(
                RunnerLeaseOverlay::None,
                |store| {
                    let request = request.clone();
                    async move { Ok(store.retry_turn(request).await) }
                },
                |snapshot, _latest_event_cursor, store, _response| {
                    retry_turn_full_delta(snapshot, store)
                },
            )
            .instrument(turn_state_write_span(
                "retry_turn",
                Some(&scope),
                Some(&run_id),
            ))
            .await;
        outcome?
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let span = turn_state_write_span(
            "request_cancel",
            Some(&request.scope),
            Some(&request.run_id),
        );
        async move {
            let previous = self.prepare_cancel_requested_runner_lease(&request).await?;
            let max_idempotency_records = self.limits.max_idempotency_records;
            let max_terminal_records = self.limits.max_terminal_records;
            let scope = request.scope.clone();
            let result = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::Run(request.run_id),
                    |store| {
                        let request = request.clone();
                        async move { store.request_cancel(request).await }
                    },
                    move |snapshot, latest_event_cursor, store, response| {
                        if snapshot.idempotency_records.len() >= max_idempotency_records {
                            return full_snapshot_delta(snapshot, store);
                        }
                        let terminal_records = snapshot
                            .runs
                            .iter()
                            .filter(|record| record.status.is_terminal())
                            .count();
                        if response.status.is_terminal() && terminal_records >= max_terminal_records
                        {
                            return full_snapshot_delta(snapshot, store);
                        }
                        run_state_with_idempotency_targeted_delta(
                            snapshot,
                            latest_event_cursor,
                            store,
                            response.run_id,
                            &scope,
                            crate::TurnIdempotencyOperationKind::Cancel,
                        )
                    },
                )
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(
                    previous,
                    TurnStatus::CancelRequested,
                )
                .await;
            }
            let response = result?;
            if response.status.is_terminal() {
                self.runner_lease_store()
                    .delete_best_effort(response.run_id)
                    .await;
            }
            Ok(response)
        }
        .instrument(span)
        .await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        // Read-your-writes (#6263 Step 3): under healthy write-behind a
        // non-critical mutation returns `Ok` after updating the hot snapshot but
        // before its durable append, so a durable-row read here could miss it
        // (`ScopeNotFound` or a stale status). Serve from the hot snapshot — the
        // single-writer authority under write-behind. Write-through (durable ==
        // cache) and the degraded path (cache cleared) read the durable rows.
        let state = if self.is_write_behind_healthy() {
            match self.read_run_state_from_hot_cache(&request).await? {
                Some(state) => Some(state),
                // Hot-cache miss under healthy write-behind: the hot snapshot is
                // bounded and evicts OLD TERMINAL runs, whose durable rows still
                // persist and must stay queryable (the eviction contract). Flush
                // our pending write-behind tail (so a not-yet-durable own write
                // can't be a false miss) then read the durable rows (#6298
                // IronLoop f6).
                None => {
                    self.flush_pending_write_behind_for_read().await?;
                    self.read_run_state_from_durable_rows(&request).await?
                }
            }
        } else {
            self.read_run_state_from_durable_rows(&request).await?
        };
        state.ok_or(TurnError::ScopeNotFound)
    }

    async fn get_run_state_for_cancellation(
        &self,
        request: GetRunStateRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.read_run_state_from_hot_cache(&request)
            .await?
            .ok_or(TurnError::ScopeNotFound)
    }
}

#[async_trait]
impl<F> TurnSpawnTreeStateStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution = run_profile_resolver
            .resolve_run_profile(crate::RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..crate::RunProfileResolutionRequest::interactive_default()
            })
            .await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            let pre_resolved = pre_resolved.clone();
            async move {
                store
                    .submit_child_turn(request, admission_policy, &pre_resolved)
                    .await
            }
        })
        .instrument(turn_state_write_span(
            "submit_child_turn",
            Some(&request.child_scope),
            request.requested_run_id.as_ref(),
        ))
        .await
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        Ok(projection::children_of(&snapshot, scope, run_id))
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::Run(run_id))
            .await?;
        Ok(projection::run_record(&snapshot, scope, run_id))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            store
                .reserve_tree_descendants(scope, root_run_id, delta, cap)
                .await
        })
        .instrument(turn_state_write_span(
            "reserve_tree_descendants",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            store
                .release_tree_descendants(scope, root_run_id, delta, idempotency_key)
                .await
        })
        .instrument(turn_state_write_span(
            "release_tree_descendants",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            store
                .prune_released_child(scope, root_run_id, child_run_id)
                .await
        })
        .instrument(turn_state_write_span(
            "prune_released_child",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }
}

#[async_trait]
impl<F> TurnEventProjectionSource for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        self.read_turn_events_from_durable_rows(scope, owner_user_id, after, limit)
            .await
    }

    async fn read_turn_event_log_after(
        &self,
        after: Option<crate::EventCursor>,
        limit: usize,
    ) -> Result<crate::TurnEventPage, TurnError> {
        self.read_turn_event_log_from_durable_rows(after, limit)
            .await
    }
}

#[async_trait]
impl<F> LoopCheckpointStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let span = turn_state_write_span(
            "put_loop_checkpoint",
            Some(&request.scope),
            Some(&request.run_id),
        );
        async move {
            self.ensure_not_degraded().await?;
            let (record, ack) = {
                // Serialize the durable enqueue on the shared snapshot-state lock
                // — the same lock every apply path holds across its
                // read-seq -> enqueue window — so the delta journal observes a
                // consistent append order without a separate commit gate.
                let mut guard = self.snapshot_state.lock().await;
                self.ensure_snapshot_cache_for_mutation(&mut guard).await?;
                let cached_store = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    std::sync::Arc::clone(&state.store)
                };
                // Insert THROUGH the cached engine so its loop-checkpoint set stays
                // authoritative: the engine's terminal/fail transition resolves the
                // latest resumable loop checkpoint from that in-memory set, and a
                // snapshot-only `apply_delta` (which mutates only the snapshot rows
                // and indexes, never `state.store`) left the cached engine blind to
                // it — so a `fail_run` on the same cached engine dropped the run's
                // `checkpoint_id` and silently made the failed run non-retryable
                // (#6263). The record's durable row is persisted below.
                let record = cached_store.put_loop_checkpoint(request).await?;
                // A loop checkpoint is normally non-critical — under write-behind it
                // lazy-flushes, and a gate-park's checkpoint is flushed by the block
                // transition's synchronous barrier. The EXCEPTION is
                // `BeforeSideEffect`: it is written immediately before a capability's
                // external side effect, and expired-lease recovery treats the ABSENCE
                // of a durable checkpoint as proof no side effect occurred (and
                // requeues the run). If it lazy-flushed, a crash after the capability
                // ran but before the flush would replay a non-idempotent side effect
                // (#6298 IronLoop f5). Keep it synchronous (critical) so it is durable
                // BEFORE the capability executes.
                let checkpoint_critical = matches!(
                    record.kind,
                    crate::run_profile::LoopCheckpointKind::BeforeSideEffect
                );
                let delta =
                    SnapshotDelta::default().set_loop_checkpoints_upsert(vec![record.clone()]);
                // Bound the pending window BEFORE enqueue on the async path
                // (#6263 Step 3): reserve a slot here, under snapshot_state, so
                // concurrent checkpoints can't grow the journal channel past the
                // cap while a flush is in flight — the same reserve→enqueue→track
                // flow every other async commit uses.
                if !checkpoint_critical && let Err(error) = self.reserve_write_behind_slot().await {
                    *guard = None;
                    return Err(error);
                }
                let ack = self.enqueue_delta(row_store_durable_delta(delta.clone()))?;
                if let Some(state) = guard.as_mut()
                    && let Err(error) = state.apply_delta(delta, state.journal_seq)
                {
                    // A failed in-memory apply may leave the cached snapshot
                    // partially mutated; drop it so the next read rebuilds from
                    // durable state. (The durable enqueue already succeeded, so
                    // recovery replays it — the cache is the only thing to
                    // invalidate; the engine already holds the record.)
                    *guard = None;
                    return Err(error);
                }
                // Track the ack in the bounded window and return `None` on the
                // async path so `commit_pending` lazy-flushes (no await); on
                // write-through it returns the ack unchanged to be awaited.
                let ack = self
                    .track_write_behind_ack_if_async(checkpoint_critical, ack)
                    .await;
                (record, ack)
            };
            self.commit_pending(
                PendingRowCommit { value: record, ack },
                "timed out waiting for loop checkpoint row-store append",
            )
            .await
        }
        .instrument(span)
        .await
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        self.read_loop_checkpoint_from_durable_rows(&request).await
    }
}

#[async_trait]
impl<F> TurnRunTransitionPort for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let span = turn_state_write_span("claim_next_run", request.scope_filter.as_ref(), None);
        async move {
            let claimed = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::None,
                    |store| {
                        let request = request.clone();
                        async move { store.claim_next_run(request).await }
                    },
                    claimed_run_targeted_delta,
                )
                .await?;
            if let Some(claimed) = &claimed
                && let Err(error) = self
                    .seed_runner_lease_from_cached_run(claimed.state.run_id)
                    .await
            {
                self.compensate_failed_claim(claimed).await;
                return Err(error);
            }
            Ok(claimed)
        }
        .instrument(span)
        .await
    }

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let span = turn_state_write_span("claim_next_runs", request.scope_filter.as_ref(), None);
        async move {
            let claimed = self
                .apply(RunnerLeaseOverlay::None, |store| {
                    let request = request.clone();
                    async move { store.claim_next_runs(request).await }
                })
                .await?;
            for run in &claimed {
                if let Err(error) = self
                    .seed_runner_lease_from_cached_run(run.state.run_id)
                    .await
                {
                    for claimed_run in &claimed {
                        self.compensate_failed_claim(claimed_run).await;
                    }
                    return Err(error);
                }
            }
            Ok(claimed)
        }
        .instrument(span)
        .await
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.heartbeat_runner_lease(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let result = self
            .apply(RunnerLeaseOverlay::All, |store| {
                let request = request.clone();
                async move { store.recover_expired_leases(request).await }
            })
            .instrument(turn_state_write_span(
                "recover_expired_leases",
                request.scope_filter.as_ref(),
                None,
            ))
            .await;
        if let Ok(response) = &result {
            for state in &response.recovered {
                self.runner_lease_store()
                    .delete_best_effort(state.run_id)
                    .await;
            }
        }
        result
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(RunnerLeaseOverlay::Run(request.run_id), |store| {
            let request = request.clone();
            async move { store.record_model_route_snapshot(request).await }
        })
        .instrument(turn_state_write_span(
            "record_model_route_snapshot",
            None,
            Some(&request.run_id),
        ))
        .await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition_with_targeted_delta(
            "block_run",
            RunStateTransitionTarget {
                run_id: request.run_id,
                runner_id: request.runner_id,
                lease_token: request.lease_token,
                retired_status: request.reason.status(),
            },
            |store| {
                let request = request.clone();
                async move { store.block_run(request).await }
            },
            blocked_run_targeted_delta,
        )
        .await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let span = turn_state_write_span("complete_run", None, Some(&request.run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(
                    request.run_id,
                    request.runner_id,
                    request.lease_token,
                    TurnStatus::Completed,
                )
                .await?;
            let max_terminal_records = self.limits.max_terminal_records;
            let result = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::Run(request.run_id),
                    |store| {
                        let request = request.clone();
                        async move { store.complete_run(request).await }
                    },
                    move |snapshot, latest_event_cursor, store, state| {
                        let terminal_records = snapshot
                            .runs
                            .iter()
                            .filter(|record| record.status.is_terminal())
                            .count();
                        if terminal_records >= max_terminal_records {
                            return full_snapshot_delta(snapshot, store);
                        }
                        run_state_targeted_delta(
                            snapshot,
                            latest_event_cursor,
                            store,
                            state.run_id,
                            &state.scope,
                        )
                    },
                )
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, TurnStatus::Completed)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let max_terminal_records = self.limits.max_terminal_records;
        self.apply_run_state_transition_with_targeted_delta(
            "cancel_run",
            RunStateTransitionTarget {
                run_id: request.run_id,
                runner_id: request.runner_id,
                lease_token: request.lease_token,
                retired_status: TurnStatus::Cancelled,
            },
            |store| {
                let request = request.clone();
                async move { store.cancel_run(request).await }
            },
            move |snapshot, latest_event_cursor, store, state| {
                let terminal_records = snapshot
                    .runs
                    .iter()
                    .filter(|record| record.status.is_terminal())
                    .count();
                if terminal_records >= max_terminal_records {
                    return full_snapshot_delta(snapshot, store);
                }
                run_state_targeted_delta(
                    snapshot,
                    latest_event_cursor,
                    store,
                    state.run_id,
                    &state.scope,
                )
            },
        )
        .await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "fail_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move { store.fail_run(request).await }
            },
        )
        .await
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "record_runner_failure",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move { store.record_runner_failure(request).await }
            },
        )
        .await
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "relinquish_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Queued,
            |store| {
                let request = request.clone();
                async move { store.relinquish_run(request).await }
            },
        )
        .await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let max_terminal_records = self.limits.max_terminal_records;
        self.apply_run_state_transition_with_targeted_delta(
            "apply_validated_loop_exit",
            RunStateTransitionTarget {
                run_id: request.run_id,
                runner_id: request.runner_id,
                lease_token: request.lease_token,
                retired_status: retired_status_for_loop_exit(&request.mapping),
            },
            |store| {
                let request = request.clone();
                async move { store.apply_validated_loop_exit(request).await }
            },
            move |snapshot, latest_event_cursor, store, state| {
                let terminal_records = snapshot
                    .runs
                    .iter()
                    .filter(|record| record.status.is_terminal())
                    .count();
                if state.status.is_terminal() && terminal_records >= max_terminal_records {
                    return full_snapshot_delta(snapshot, store);
                }
                if state.status.is_blocked() {
                    return blocked_run_targeted_delta(snapshot, latest_event_cursor, store, state);
                }
                run_state_targeted_delta(
                    snapshot,
                    latest_event_cursor,
                    store,
                    state.run_id,
                    &state.scope,
                )
            },
        )
        .await
    }
}

fn retired_status_for_loop_exit(mapping: &crate::LoopExitMapping) -> TurnStatus {
    match mapping {
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed) => {
            TurnStatus::Completed
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => {
            TurnStatus::Cancelled
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked { reason, .. }) => {
            reason.status()
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { .. })
        | crate::LoopExitMapping::RecoveryRequired { .. } => TurnStatus::Failed,
    }
}
