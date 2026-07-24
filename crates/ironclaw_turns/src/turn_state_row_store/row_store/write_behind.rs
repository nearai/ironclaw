//! Write-behind window machinery for [`TurnStateRowStore`]: the
//! bounded pending-ack backpressure window, the critical-vs-async commit
//! barrier, degradation guards, and the journal enqueue/await plumbing. Moved
//! verbatim from the module root during the #6263 decomposition; behavior is
//! unchanged.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;

use crate::TurnError;

use super::{
    PendingRowCommit, TurnStateRowStore,
    delta::{RowSnapshotState, SnapshotDelta},
    journal::{DeltaAck, DeltaJournal},
};

impl<F> TurnStateRowStore<F>
where
    F: RootFilesystem,
{
    /// Flush the write-behind tail: await every enqueued-but-un-acked
    /// non-critical delta ack so nothing un-durable remains when the process
    /// exits.
    ///
    /// Awaits the oldest-first backpressure window populated by
    /// [`track_write_behind_ack_if_async`](Self::track_write_behind_ack_if_async),
    /// giving a planned/graceful restart a clean durable tail. A hard crash
    /// (SIGKILL/OOM) still loses only the un-acked non-critical (non-critical =
    /// not gate-park/terminal/brand-new-run) tail; gate-park, terminal, and
    /// brand-new-run transitions are synchronously durable and never wait on
    /// this drain.
    ///
    /// Idempotent. On an append failure the flusher has halted and latched the
    /// store degraded; the error is surfaced and the hot cache is rolled back to
    /// the last consistent durable point (mirroring the backpressure path).
    pub async fn drain(&self) -> Result<(), TurnError> {
        let mut window = self.pending_write_behind.lock().await;
        while let Some(ack) = window.pop_front() {
            if let Err(error) = DeltaJournal::await_ack(Some(ack)).await {
                drop(window);
                self.clear_snapshot_cache().await;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Whether reads must serve from the process-local hot snapshot to honor
    /// read-your-writes. A non-critical mutation returns `Ok` after updating
    /// the hot cache but before its durable append, so a durable-row read
    /// could miss it. The hot cache is the single-writer authority. Once the
    /// journal has degraded the cache is cleared and reads must fall back to
    /// the last consistent durable point.
    pub(super) fn is_write_behind_healthy(&self) -> bool {
        !self.delta_journal.is_degraded()
    }

    /// Reset the hot cache after a mutation the embedded engine REJECTED (a
    /// domain error such as `ThreadBusy` / `InvalidTransition`).
    ///
    /// The op ran against the shared cached engine, so it may have left a
    /// partial mutation; that must be discarded. Durable LAGS the cache
    /// (un-acked non-critical ops), so a reload-from-durable would silently
    /// drop acked-but-unflushed ops the caller was told succeeded. Instead,
    /// rebuild the embedded engine from the cached snapshot (which still
    /// includes those un-flushed ops), discarding only the failed op's
    /// partial mutation.
    pub(super) fn reset_cache_after_rejected_mutation(
        &self,
        guard: &mut Option<RowSnapshotState>,
    ) -> Result<(), TurnError> {
        if let Some(state) = guard.as_mut() {
            state.store = Arc::new(self.build_in_memory_store(state.snapshot.clone())?);
        }
        Ok(())
    }

    /// Fail a mutation fast when the store has degraded after a write-behind
    /// append failure. Clears the diverged hot cache (reads then reload from the
    /// last consistent durable point) and returns a retryable error.
    pub(super) async fn ensure_not_degraded(&self) -> Result<(), TurnError> {
        if self.delta_journal.is_degraded() {
            self.clear_snapshot_cache().await;
            return Err(TurnError::Unavailable {
                reason: "turn-state row store degraded after a write-behind durable append failure"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Commit a prepared [`PendingRowCommit`].
    ///
    /// * Critical — await the ack (a barrier: awaiting a critical op's ack
    ///   implies the whole preceding async tail is durable).
    /// * Non-critical — already enqueued and tracked in the bounded pending
    ///   window by `apply` (returns `ack: None`), so nothing is awaited here.
    pub(super) async fn commit_pending<T>(
        &self,
        pending: PendingRowCommit<T>,
        timeout_reason: &'static str,
    ) -> Result<T, TurnError> {
        // `ack` is `None` for a no-op/empty commit AND for a non-critical
        // write-behind commit (enqueued + tracked in `apply` under the
        // `snapshot_state` lock): either way there is nothing to flush here.
        if pending.ack.is_none() {
            return Ok(pending.value);
        }
        // A durable ack is present ⇒ a critical write-behind barrier: await
        // it. A non-critical write-behind commit never reaches here —
        // `track_write_behind_ack_if_async` returned `ack: None` above, so the
        // `ack.is_none()` early return already handled it.
        self.await_pending_commit(pending, timeout_reason).await
    }

    /// Reserve a slot in the bounded write-behind pending window BEFORE the
    /// journal enqueue, so a stalled flusher cannot let concurrent callers grow
    /// the unbounded journal channel without bound (#6263 Step 3). Called under
    /// the `snapshot_state` lock, which serializes enqueue; the flusher drains
    /// the journal independently of that lock, so awaiting the oldest pending
    /// ack here is backpressure, not deadlock. At the cap, await (and drop) the
    /// OLDEST pending ack first, bounding both memory and the crash-loss window.
    /// A degraded (append-failure) ack propagates; the caller clears the hot
    /// cache and fails fast.
    pub(super) async fn reserve_write_behind_slot(&self) -> Result<(), TurnError> {
        let cap = self.limits.max_pending_write_behind_deltas.max(1);
        let mut window = self.pending_write_behind.lock().await;
        while window.len() >= cap {
            // Await the OLDEST ack IN PLACE — peek, don't pop first. This runs
            // under `apply`'s outer timeout, and a cancelled await MUST leave the
            // ack tracked in the window; popping first would drop it on
            // cancellation, letting later writes see an empty window and enqueue
            // behind a stalled append (unbounded again), and letting a read-side
            // drain falsely succeed while the acknowledged write is still in
            // flight (#6298 IronLoop f7). Remove it only once it has resolved.
            let Some(front) = window.front_mut() else {
                break;
            };
            let result = DeltaJournal::await_ack_ref(front).await;
            window.pop_front();
            result?;
        }
        Ok(())
    }

    /// For a non-critical write-behind commit, move the durable ack into the
    /// bounded pending window (its slot was reserved by
    /// [`reserve_write_behind_slot`](Self::reserve_write_behind_slot) before the
    /// enqueue) and return `None` so [`commit_pending`](Self::commit_pending)
    /// returns without awaiting. Otherwise return the ack unchanged for the
    /// caller to await (a critical barrier). Runs under
    /// `snapshot_state`, so the reserve→enqueue→track sequence is serialized and
    /// the window can never exceed the cap.
    pub(super) async fn track_write_behind_ack_if_async(
        &self,
        critical: bool,
        ack: Option<DeltaAck>,
    ) -> Option<DeltaAck> {
        if !critical {
            if let Some(ack) = ack {
                self.pending_write_behind.lock().await.push_back(ack);
            }
            return None;
        }
        ack
    }

    /// Read-side write-behind barrier (#6298).
    ///
    /// The durable-read query methods (`get_run_state`, `read_turn_events_after`,
    /// `get_loop_checkpoint`) read materialized durable rows so they preserve the
    /// contracts a bounded hot cache cannot: cross-writer freshness and
    /// queryability of terminal runs / events the cache has EVICTED but the
    /// durable store retains. However, a non-critical mutation returns `Ok`
    /// after merely ENQUEUEing its delta — before the flusher appends it and
    /// the materializer writes its rows. A durable read issued in that window
    /// would miss the just-acked write (`get_run_state` → `Ok(None)` →
    /// `ScopeNotFound`, an empty event page, a missing checkpoint), failing
    /// every runtime `submit_turn` → `get_run_state`.
    ///
    /// Draining the enqueued-but-un-acked non-critical window here — bounded by
    /// [`TurnStateStoreLimits::max_pending_write_behind_deltas`], typically
    /// just the caller's own trailing submit — awaits those durable appends, so
    /// the subsequent durable read (which force-materializes the journal tail)
    /// observes them. This is a read-side barrier symmetric to a critical
    /// transition's write-side barrier, and keeps the durable read's exact
    /// semantics stable (only WHEN it reads changes, never WHAT it reads). On a
    /// drained ack failure the flusher has halted and latched the store
    /// degraded; clear the diverged hot cache and surface the retryable error,
    /// mirroring the backpressure path.
    pub(super) async fn flush_pending_write_behind_for_read(&self) -> Result<(), TurnError> {
        // Await each pending ack IN PLACE under the window lock (peek-await-pop),
        // removing it only once it resolves. Holding the lock across the awaits
        // bounds the set to the writes present when the flush began — no later
        // write can register mid-flush, so this still only covers read-your-writes
        // writes — AND keeps un-awaited acks tracked if this read is cancelled. A
        // drain-into-`Vec` would drop the un-awaited acks on cancellation, losing
        // acknowledged-but-unflushed writes and letting this flush falsely report
        // success (#6298 IronLoop f7). The journal flusher does not take this
        // lock, so awaiting under it cannot deadlock; the set is bounded by the
        // pending cap.
        let mut window = self.pending_write_behind.lock().await;
        while let Some(front) = window.front_mut() {
            let result = DeltaJournal::await_ack_ref(front).await;
            window.pop_front();
            if let Err(error) = result {
                drop(window);
                self.clear_snapshot_cache().await;
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn enqueue_delta(
        &self,
        delta: SnapshotDelta,
    ) -> Result<Option<DeltaAck>, TurnError> {
        self.delta_journal.enqueue(delta)
    }

    pub(super) async fn await_delta_ack(&self, ack: Option<DeltaAck>) -> Result<(), TurnError> {
        DeltaJournal::await_ack(ack).await
    }

    async fn await_pending_commit<T>(
        &self,
        pending: PendingRowCommit<T>,
        timeout_reason: &'static str,
    ) -> Result<T, TurnError> {
        match tokio::time::timeout(self.apply_timeout, self.await_delta_ack(pending.ack)).await {
            Ok(Ok(())) => Ok(pending.value),
            Ok(Err(error)) => {
                self.clear_snapshot_cache().await;
                Err(error)
            }
            Err(_) => {
                self.clear_snapshot_cache().await;
                Err(TurnError::Unavailable {
                    reason: timeout_reason.to_string(),
                })
            }
        }
    }
}
