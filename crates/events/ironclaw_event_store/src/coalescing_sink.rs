//! Write-behind coalescing runtime [`EventSink`].
//!
//! The durable runtime event log is append-only and *derived* (projections
//! fold it; durable cursors persisted elsewhere can outlive it). Its appends
//! are observability writes whose returned cursor is discarded at the sink
//! (`DurableEventSink::emit` → `.map(|_| ())`), so nothing on the hot path
//! gates a side effect on synchronous durability of an individual event.
//!
//! [`CoalescingEventSink`] exploits that: `emit` buffers the event in memory
//! and returns immediately; a single background drain task flushes the buffer
//! as **one multi-row INSERT per stream per drain window** (via
//! [`DurableEventLog::append_batch`]). A per-turn burst of ~21–57 single-row
//! INSERTs collapses to one round-trip, while crash-loss is bounded to the
//! unflushed sub-second tail.
//!
//! **This is a runtime-event-only optimization.** The compliance audit log
//! (`DurableAuditSink`) is a separate sink and stays synchronous.
//!
//! ## Ordering & durability
//!
//! - All flushes are awaited sequentially in the single drain task, so the
//!   global append order is preserved; within a stream the multi-row INSERT
//!   assigns contiguous, monotonic seqs in buffer order.
//! - A flush is one atomic multi-row INSERT — there is no torn batch. On a
//!   crash, every flushed batch is durable and only the in-memory tail
//!   (bounded by `flush_interval` or `max_batch`) is lost.
//! - The [`EventSink::flush`] override drains everything queued before the
//!   call, for graceful shutdown and deterministic tests.
//! - The internal channel is bounded by [`EventBatchConfig::channel_capacity`].
//!   [`EventSink::emit`] and [`NonBlockingEventSink::try_emit`] drop best-effort
//!   events while the channel is full; neither applies backpressure to
//!   producers.
//!
//! ## Loss semantics — best-effort by design, NOT at-least-once
//!
//! This sink is **lossy under stress on purpose**. It carries best-effort
//! observability events whose returned cursor is discarded at the sink, so
//! none of these losses can alter a runtime/control-plane outcome. There are
//! three stress-path drop modes (beyond the crash-tail loss noted under
//! Ordering & durability above), all bounded and observable:
//!
//! 1. **Overload drop** — sustained back-pressure fills the bounded channel;
//!    best-effort emission drops (it must never block the caller) and increments
//!    `dropped_count`.
//! 2. **Per-event reject** — a backend `append_batch` rejects some rows. The
//!    successful rows are still durably committed (`append_batch` preserves the
//!    successful prefix and returns per-event results); only the rejected rows
//!    are lost. They are NOT requeued: retrying in this single serialized drain
//!    loop would head-of-line-block every other stream and, under a flapping
//!    backend, grow memory unboundedly — the same failure the channel bound
//!    avoids.
//! 3. **Drain panic** — a backend driver panics inside the spawned append; the
//!    whole window for that flush is lost and `error!`-logged. Retrying a
//!    panicking append is a panic loop, so the next window simply proceeds.
//!
//! This sink does not retry backend rejects or survive process crashes.
//! At-least-once durability requires a different substrate.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_event_log::{
    DurableEventLog, EventError, EventSink, NonBlockingEventSink, RuntimeEvent,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

/// Default maximum number of [`DrainMessage`]s queued in the write-behind
/// channel. ~8 k events is generous burst headroom for normal traffic while
/// still bounding memory when the durable backend stalls.
const DEFAULT_CHANNEL_CAPACITY: usize = 8192;

/// Tuning for the write-behind coalescing event sink.
#[derive(Debug, Clone, Copy)]
pub struct EventBatchConfig {
    /// Maximum events folded into a single multi-row INSERT. A burst larger
    /// than this flushes early (in `max_batch`-sized statements) so memory and
    /// per-statement parameter counts stay bounded.
    pub max_batch: usize,
    /// Upper bound on how long the first event of a window waits before its
    /// batch is flushed. Bounds both visibility lag and crash-loss tail.
    pub flush_interval: Duration,
    /// Maximum queued messages while the durable backend is stalled.
    pub channel_capacity: usize,
}

impl Default for EventBatchConfig {
    fn default() -> Self {
        Self {
            max_batch: 256,
            flush_interval: Duration::from_millis(50),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

enum DrainMessage {
    // Boxed: `RuntimeEvent` is much larger than the flush ack, so boxing keeps
    // the channel's per-message footprint small.
    Event(Box<RuntimeEvent>),
    Flush(oneshot::Sender<Result<(), EventError>>),
}

/// Write-behind [`EventSink`] that coalesces same-window runtime appends into
/// batched multi-row INSERTs. Cheap to [`Clone`]: all clones feed the one
/// shared drain task.
#[derive(Clone)]
pub struct CoalescingEventSink {
    tx: mpsc::Sender<DrainMessage>,
    dropped: Arc<AtomicU64>,
    channel_capacity: usize,
}

impl std::fmt::Debug for CoalescingEventSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoalescingEventSink")
            .field("tx", &"<drain_sender>")
            .finish()
    }
}

impl CoalescingEventSink {
    /// Spawn the drain task and return a sink that buffers appends to `log`.
    pub fn new(log: Arc<dyn DurableEventLog>, config: EventBatchConfig) -> Self {
        let channel_capacity = config.channel_capacity.max(1);
        let (tx, rx) = mpsc::channel(channel_capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        tokio::spawn(drain_loop(log, config, rx));
        Self {
            tx,
            dropped,
            channel_capacity,
        }
    }

    /// Number of events dropped because the internal channel was full.
    ///
    /// Useful for metrics and tests. A non-zero value means the durable backend
    /// is stalling and best-effort events are being silently discarded.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn try_emit_event(&self, event: RuntimeEvent) -> Result<(), EventError> {
        match self.tx.try_send(DrainMessage::Event(Box::new(event))) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Rate-limited: log on the first drop (0→1 transition) and
                // every 1000th drop thereafter. The `dropped` counter is the
                // real signal; logging every single drop would amplify a
                // backend outage with unbounded I/O and corrupt the REPL/TUI.
                let new_dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if new_dropped == 1 || new_dropped.is_multiple_of(1000) {
                    tracing::debug!(
                        target: "ironclaw::reborn::event_store::coalescing",
                        dropped = new_dropped,
                        capacity = self.channel_capacity,
                        "event dropped: coalescing channel at capacity; drain may be stalled"
                    );
                }
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(sink_closed()),
        }
    }
}

fn sink_closed() -> EventError {
    EventError::Sink {
        reason: "coalescing event sink drain task is no longer running".to_string(),
    }
}

#[async_trait]
impl EventSink for CoalescingEventSink {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        // Best-effort: buffer and return immediately. The channel is bounded;
        // if it is full (drain stalled) we drop the event rather than block
        // the caller.
        self.try_emit_event(event)
    }

    /// Flush every event queued before this call, awaiting durable write. Used
    /// for graceful shutdown and deterministic tests. Returns `Ok(())` only
    /// when every queued event has been written durably. Returns `Err` if the
    /// drain task is no longer running **or** if the durable append itself
    /// failed — callers can rely on `Ok(())` as a true durability guarantee.
    ///
    /// Unlike [`EventSink::emit`], flush awaits channel capacity rather than
    /// dropping on full — it is on the slow/shutdown path and must not be lost.
    async fn flush(&self) -> Result<(), EventError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(DrainMessage::Flush(ack_tx))
            .await
            .map_err(|_| sink_closed())?;
        // The ack now carries a Result; flatten Result<Result<…>, RecvError>.
        ack_rx.await.map_err(|_| sink_closed())?
    }
}

impl NonBlockingEventSink for CoalescingEventSink {
    fn try_emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        self.try_emit_event(event)
    }
}

async fn drain_loop(
    log: Arc<dyn DurableEventLog>,
    config: EventBatchConfig,
    mut rx: mpsc::Receiver<DrainMessage>,
) {
    let max_batch = config.max_batch.max(1);
    let mut pending_flush_error: Option<EventError> = None;
    loop {
        // Block until a window opens (or exit once every sender is dropped).
        let first = match rx.recv().await {
            Some(message) => message,
            None => return,
        };

        let mut batch: Vec<RuntimeEvent> = Vec::new();
        let mut acks: Vec<oneshot::Sender<Result<(), EventError>>> = Vec::new();
        let mut closed = false;

        match first {
            DrainMessage::Event(event) => batch.push(*event),
            DrainMessage::Flush(ack) => acks.push(ack),
        }

        // Accumulate the window: stop at the size cap, the interval deadline, a
        // flush request, or channel close. A flush-only first message skips the
        // wait entirely (nothing queued before it survives the FIFO ordering).
        if !batch.is_empty() {
            let deadline = Instant::now() + config.flush_interval;
            while batch.len() < max_batch {
                match timeout_at(deadline, rx.recv()).await {
                    Ok(Some(DrainMessage::Event(event))) => batch.push(*event),
                    Ok(Some(DrainMessage::Flush(ack))) => {
                        acks.push(ack);
                        break;
                    }
                    Ok(None) => {
                        closed = true;
                        break;
                    }
                    Err(_elapsed) => break,
                }
            }
        }

        let flush_result = flush_batch(&log, std::mem::take(&mut batch)).await;
        if pending_flush_error.is_none() {
            pending_flush_error = flush_result.err();
        }
        if !acks.is_empty() {
            // A graceful flush must surface the original typed failure from
            // earlier drain windows, not only the window that happened to
            // carry the Flush message.
            let flush_error = pending_flush_error.take();
            for ack in acks {
                let payload = match &flush_error {
                    None => Ok(()),
                    Some(error) => Err(error.clone()),
                };
                // Receiver may have given up; ignore.
                let _ = ack.send(payload);
            }
        }

        if closed {
            return;
        }
    }
}

async fn flush_batch(
    log: &Arc<dyn DurableEventLog>,
    batch: Vec<RuntimeEvent>,
) -> Result<(), EventError> {
    if batch.is_empty() {
        return Ok(());
    }
    // Isolate the flush in its own task so a panic inside a backend
    // `append_batch` (driver bug, serialization edge) cannot tear down the
    // long-lived drain loop and silently stop ALL runtime event logging for
    // the process. A panic here is logged and the next window still drains.
    // Awaiting the handle keeps flushes serialized → global append order is
    // preserved.
    //
    // No retry/requeue on failure — deliberate, see the module-level "Loss
    // semantics". `append_batch` durably commits the successful prefix and
    // returns per-event results, so only backend-rejected rows are lost (not
    // the window). Requeuing them in this single serialized loop would
    // head-of-line-block every other stream and grow memory unboundedly under
    // a flapping backend. These are best-effort observability events; the
    // first error is surfaced so `flush()` can fail loud, nothing is replayed.
    let log = Arc::clone(log);
    match tokio::spawn(async move { log.append_batch(batch).await }).await {
        Ok(results) => {
            // Collect the first error (if any) so callers can surface durable
            // failures. All per-event errors are logged; the first is returned
            // so `flush()` can fail loud on a stalled backend.
            let mut first_error: Option<EventError> = None;
            for result in results {
                if let Err(error) = result {
                    tracing::debug!(
                        target: "ironclaw::reborn::event_store::coalescing",
                        %error,
                        "durable event append failed during coalescing flush"
                    );
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        Err(join_error) => {
            tracing::error!(
                target: "ironclaw::reborn::event_store::coalescing",
                %join_error,
                "coalescing event flush panicked; dropping this batch and continuing"
            );
            Err(EventError::Sink {
                reason: join_error.to_string(),
            })
        }
    }
}
