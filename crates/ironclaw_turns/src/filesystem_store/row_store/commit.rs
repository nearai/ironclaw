//! The two apply engines (whole-snapshot and targeted-delta), overlay-store
//! acquisition, snapshot-cache preparation for mutation, and the run-state
//! transition wrappers for [`TurnStateRowStore`]. Moved verbatim from
//! the module root during the #6263 decomposition; behavior is unchanged.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use tracing::Instrument;

use crate::filesystem_store::{
    runner_lease::RunnerLeaseOverlay, turn_state_engine::TurnStateEngine,
};
use crate::{
    EventCursor, TurnError, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnRunState,
    TurnStatus,
    runner::{ClaimedTurnRun, RelinquishRunRequest, TurnRunTransitionPort},
};

use super::{
    PendingRowCommit, RowApplyOutcome, RunStateTransitionTarget, TurnStateRowStore,
    delta::{
        RowSnapshotState, SnapshotDelta, preserve_loop_checkpoints, row_store_durable_delta,
        row_store_hot_cache_snapshot, snapshot_delta,
    },
    delta_is_recoverability_critical, turn_state_write_span,
};

impl<F> TurnStateRowStore<F>
where
    F: RootFilesystem,
{
    pub(super) async fn ensure_snapshot_cache_for_mutation(
        &self,
        guard: &mut Option<RowSnapshotState>,
    ) -> Result<(), TurnError> {
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        Ok(())
    }

    async fn refresh_snapshot_cache_after_stale_mutation_error(
        &self,
        guard: &mut Option<RowSnapshotState>,
        error: &TurnError,
    ) -> Result<bool, TurnError> {
        if !matches!(error, TurnError::ThreadBusy(_)) {
            return Ok(false);
        }
        let head_seq = self.delta_log_head_seq().await?;
        if guard
            .as_ref()
            .is_none_or(|state| state.journal_seq < head_seq)
        {
            *guard = Some(self.load_snapshot_from_rows().await?);
            return Ok(true);
        }
        Ok(false)
    }

    /// Acquire the long-lived embedded authority with the runner-lease overlay
    /// applied, without rebuilding it from a full-snapshot clone.
    ///
    /// For [`RunnerLeaseOverlay::None`] and [`RunnerLeaseOverlay::Run`] this
    /// reuses the cached authority in place (the `Run` case overlays a single
    /// run record's live lease heartbeat onto it) — an O(1) reuse rather than a
    /// per-op O(store-size) `from_persistence_snapshot` rebuild. Only
    /// [`RunnerLeaseOverlay::All`] (expired-lease recovery) still rebuilds from
    /// an overlaid snapshot clone, because it must overlay every run's lease.
    /// Capture the runner-lease overlay inputs from the current cached state
    /// for one `apply` iteration: an `All` overlay needs the whole snapshot
    /// re-overlaid (baseline `Some`), a `Run` overlay needs just that run's
    /// record patched onto the shared cached engine, and `None` needs neither.
    /// Shared by both the whole-snapshot and targeted-delta apply paths.
    fn overlay_inputs(
        state: &RowSnapshotState,
        overlay: RunnerLeaseOverlay,
    ) -> (Option<TurnPersistenceSnapshot>, Option<TurnRunRecord>) {
        let overlay_baseline =
            matches!(overlay, RunnerLeaseOverlay::All).then(|| state.snapshot.clone());
        let overlay_run = match overlay {
            RunnerLeaseOverlay::Run(run_id) => state.run_record_by_id(run_id),
            RunnerLeaseOverlay::None | RunnerLeaseOverlay::All => None,
        };
        (overlay_baseline, overlay_run)
    }

    async fn acquire_overlaid_store(
        &self,
        cached_store: Arc<TurnStateEngine>,
        overlay_baseline: Option<TurnPersistenceSnapshot>,
        overlay_run: Option<TurnRunRecord>,
        overlay: RunnerLeaseOverlay,
    ) -> Result<Arc<TurnStateEngine>, TurnError> {
        if let Some(baseline) = overlay_baseline {
            let (overlaid_snapshot, _) = self
                .runner_lease_store()
                .overlay((baseline, None), overlay)
                .await?;
            Ok(Arc::new(self.build_in_memory_store(overlaid_snapshot)?))
        } else {
            if let Some(run) = overlay_run {
                let overlaid = self.runner_lease_store().overlay_run_record(run).await?;
                cached_store.overlay_runner_lease_record(overlaid)?;
            }
            Ok(cached_store)
        }
    }

    pub(super) async fn apply<T, A, Fut>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        T: Send,
    {
        self.ensure_not_degraded().await?;
        let critical = async {
            let mut guard = self.snapshot_state.lock().await;
            let mut refreshed_after_stale_error = false;
            loop {
                self.ensure_snapshot_cache_for_mutation(&mut guard).await?;
                let (baseline, current_journal_seq, cached_store, overlay_baseline, overlay_run) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let (overlay_baseline, overlay_run) = Self::overlay_inputs(state, overlay);
                    (
                        state.snapshot.clone(),
                        state.journal_seq,
                        Arc::clone(&state.store),
                        overlay_baseline,
                        overlay_run,
                    )
                };
                let store = self
                    .acquire_overlaid_store(cached_store, overlay_baseline, overlay_run, overlay)
                    .await?;
                let outcome = apply(Arc::clone(&store)).await;
                let mut new_snapshot = store.persistence_snapshot();
                preserve_loop_checkpoints(&baseline, &mut new_snapshot);
                let value = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        if !refreshed_after_stale_error
                            && self
                                .refresh_snapshot_cache_after_stale_mutation_error(
                                    &mut guard, &error,
                                )
                                .await?
                        {
                            refreshed_after_stale_error = true;
                            continue;
                        }
                        self.reset_cache_after_rejected_mutation(&mut guard)?;
                        return Err(error);
                    }
                };
                if new_snapshot == baseline {
                    return Ok(RowApplyOutcome::Ready(value));
                }

                let delta = match snapshot_delta(&baseline, &new_snapshot) {
                    Ok(delta) => delta,
                    Err(error) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                let persist_delta = row_store_durable_delta(delta.clone());
                // A durable-empty delta (the snapshot differs only in categories
                // the durable delta strips — run/turn tombstones, admission
                // scaffolding, retention floor — all rebuilt on load) must NOT
                // consume a reservation sequence: `enqueue_delta` skips an empty
                // delta so no backend append happens. Advancing the hot-cache
                // journal seq without a matching append desyncs it from the
                // backend append log, and a later mutation's rows/reservations
                // land one seq ahead of the real append — a subsequent active-lock
                // DELETE then collides with a stale reserved row and is dropped on
                // crash recovery (#6263). Keep the hot cache current at the SAME
                // seq and return.
                if persist_delta.is_empty() {
                    let next_state =
                        RowSnapshotState::new(new_snapshot, store, current_journal_seq)?;
                    *guard = Some(next_state);
                    return Ok(RowApplyOutcome::Ready(value));
                }
                let delta_critical = delta_is_recoverability_critical(&baseline, &persist_delta);
                let reservation_seq = current_journal_seq.next();
                let next_state = RowSnapshotState::new(new_snapshot, store, reservation_seq)?;
                // Bound the pending window BEFORE enqueue (#6263 Step 3): a
                // non-critical write-behind op reserves a slot here, under
                // `snapshot_state`, so concurrent callers can never grow the
                // journal channel past the cap while the flusher is stalled.
                // A degraded reservation → clear the hot cache (next read
                // reloads from durable) and fail fast.
                if !delta_critical && let Err(error) = self.reserve_write_behind_slot().await {
                    *guard = None;
                    return Err(error);
                }
                let ack = match self.enqueue_delta(persist_delta) {
                    Ok(ack) => ack,
                    Err(error) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                *guard = Some(next_state);
                let ack = self
                    .track_write_behind_ack_if_async(delta_critical, ack)
                    .await;
                return Ok(RowApplyOutcome::Pending(PendingRowCommit { value, ack }));
            }
        };

        let outcome = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store apply timed out".to_string(),
                });
            }
        };
        match outcome {
            RowApplyOutcome::Ready(value) => Ok(value),
            RowApplyOutcome::Pending(pending) => {
                self.commit_pending(pending, "turn state row-store append ack timed out")
                    .await
            }
        }
    }

    pub(super) async fn apply_with_targeted_delta<T, A, Fut, D>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
        build_delta: D,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &TurnStateEngine,
                &T,
            ) -> Result<SnapshotDelta, TurnError>
            + Send,
        T: Send,
    {
        self.ensure_not_degraded().await?;
        let critical = async {
            let mut guard = self.snapshot_state.lock().await;
            let mut build_delta = Some(build_delta);
            let mut refreshed_after_stale_error = false;
            loop {
                self.ensure_snapshot_cache_for_mutation(&mut guard).await?;
                let (latest_event_cursor, cached_store, overlay_baseline, overlay_run) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let (overlay_baseline, overlay_run) = Self::overlay_inputs(state, overlay);
                    (
                        state.latest_event_cursor(),
                        Arc::clone(&state.store),
                        overlay_baseline,
                        overlay_run,
                    )
                };
                let store = self
                    .acquire_overlaid_store(cached_store, overlay_baseline, overlay_run, overlay)
                    .await?;
                let outcome = apply(Arc::clone(&store)).await;
                let value = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        if !refreshed_after_stale_error
                            && self
                                .refresh_snapshot_cache_after_stale_mutation_error(
                                    &mut guard, &error,
                                )
                                .await?
                        {
                            refreshed_after_stale_error = true;
                            continue;
                        }
                        self.reset_cache_after_rejected_mutation(&mut guard)?;
                        return Err(error);
                    }
                };
                let build_delta = build_delta.take().ok_or_else(|| TurnError::Unavailable {
                    reason: "turn state row-store targeted delta builder was reused".to_string(),
                })?;
                let (delta, persist_delta, delta_critical, reservation_seq) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let delta =
                        build_delta(&state.snapshot, latest_event_cursor, store.as_ref(), &value)?;
                    let persist_delta = row_store_durable_delta(delta.clone());
                    let delta_critical =
                        delta_is_recoverability_critical(&state.snapshot, &persist_delta);
                    (
                        delta,
                        persist_delta,
                        delta_critical,
                        state.journal_seq.next(),
                    )
                };
                // A no-op or in-memory-only mutation whose DURABLE delta is empty
                // must NOT consume a reservation sequence. `enqueue_delta` skips an
                // empty delta (no backend append happens), so advancing the
                // hot-cache journal seq here would desync it from the backend
                // append log: the next mutation's rows would be written one
                // sequence ahead of the real append, and a later active-lock
                // DELETE (materialized at the real seq) would then collide with
                // the stale row and be dropped, leaking the lock across a crash
                // (#6263). Apply the hot-cache delta at the CURRENT seq (no
                // advance), enqueue nothing, and return.
                if persist_delta.is_empty() {
                    if let Some(state) = guard.as_mut() {
                        let current_seq = state.journal_seq;
                        if let Err(error) = state.apply_delta(delta, current_seq) {
                            *guard = None;
                            return Err(error);
                        }
                        state.store = store;
                    }
                    return Ok(PendingRowCommit { value, ack: None });
                }
                if let Some(state) = guard.as_mut() {
                    if let Err(error) = state.apply_delta(delta, reservation_seq) {
                        *guard = None;
                        return Err(error);
                    }
                    state.store = store;
                } else {
                    let mut snapshot = store.persistence_snapshot();
                    snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
                    let next_state = match RowSnapshotState::new(snapshot, store, reservation_seq) {
                        Ok(state) => state,
                        Err(error) => {
                            *guard = None;
                            return Err(error);
                        }
                    };
                    *guard = Some(next_state);
                }
                // Bound the pending window BEFORE enqueue (#6263 Step 3): see the
                // twin reservation in the whole-snapshot apply path above.
                if !delta_critical && let Err(error) = self.reserve_write_behind_slot().await {
                    *guard = None;
                    return Err(error);
                }
                let ack = match self.enqueue_delta(persist_delta) {
                    Ok(ack) => ack,
                    Err(error) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                let ack = self
                    .track_write_behind_ack_if_async(delta_critical, ack)
                    .await;
                return Ok(PendingRowCommit { value, ack });
            }
        };

        let pending = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store targeted apply timed out".to_string(),
                });
            }
        };
        self.commit_pending(
            pending,
            "turn state row-store targeted append ack timed out",
        )
        .await
    }

    pub(super) async fn apply_run_state_transition<A, Fut>(
        &self,
        operation: &'static str,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
        apply: A,
    ) -> Result<TurnRunState, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<TurnRunState, TurnError>> + Send,
    {
        let span = turn_state_write_span(operation, None, Some(&run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(run_id, runner_id, lease_token, retired_status)
                .await?;
            let result = self.apply(RunnerLeaseOverlay::Run(run_id), apply).await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, retired_status)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    pub(super) async fn apply_run_state_transition_with_targeted_delta<A, Fut, D>(
        &self,
        operation: &'static str,
        target: RunStateTransitionTarget,
        apply: A,
        build_delta: D,
    ) -> Result<TurnRunState, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<TurnRunState, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &TurnStateEngine,
                &TurnRunState,
            ) -> Result<SnapshotDelta, TurnError>
            + Send,
    {
        let RunStateTransitionTarget {
            run_id,
            runner_id,
            lease_token,
            retired_status,
        } = target;
        let span = turn_state_write_span(operation, None, Some(&run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(run_id, runner_id, lease_token, retired_status)
                .await?;
            let result = self
                .apply_with_targeted_delta(RunnerLeaseOverlay::Run(run_id), apply, build_delta)
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, retired_status)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    pub(super) async fn compensate_failed_claim(&self, claimed: &ClaimedTurnRun) {
        let run_id = claimed.state.run_id;
        let result = self
            .apply(RunnerLeaseOverlay::Run(run_id), |store| async move {
                let outcome = store
                    .relinquish_run(RelinquishRunRequest {
                        run_id,
                        runner_id: claimed.runner_id,
                        lease_token: claimed.lease_token,
                    })
                    .await;
                outcome.map(|_| ())
            })
            .instrument(turn_state_write_span(
                "compensate_failed_claim",
                Some(&claimed.state.scope),
                Some(&run_id),
            ))
            .await;
        if let Err(error) = result {
            tracing::debug!(
                run_id = %run_id,
                error = %error,
                "failed to compensate turn claim after memory runner lease seed failed"
            );
        }
    }
}
