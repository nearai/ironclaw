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
//! - [`CoalescingEventSink::flush`] drains everything queued before the call,
//!   for graceful shutdown and deterministic tests.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_events::{DurableEventLog, EventError, EventSink, RuntimeEvent};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

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
}

impl Default for EventBatchConfig {
    fn default() -> Self {
        Self {
            max_batch: 256,
            flush_interval: Duration::from_millis(50),
        }
    }
}

enum DrainMessage {
    // Boxed: `RuntimeEvent` is much larger than the flush ack, so boxing keeps
    // the channel's per-message footprint small.
    Event(Box<RuntimeEvent>),
    Flush(oneshot::Sender<()>),
}

/// Write-behind [`EventSink`] that coalesces same-window runtime appends into
/// batched multi-row INSERTs. Cheap to [`Clone`]: all clones feed the one
/// shared drain task.
#[derive(Clone)]
pub struct CoalescingEventSink {
    tx: mpsc::UnboundedSender<DrainMessage>,
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
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(drain_loop(log, config, rx));
        Self { tx }
    }

    /// Flush every event queued before this call, awaiting durable write. Used
    /// for graceful shutdown and deterministic tests. Returns an error only if
    /// the drain task is no longer running.
    pub async fn flush(&self) -> Result<(), EventError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(DrainMessage::Flush(ack_tx))
            .map_err(|_| sink_closed())?;
        ack_rx.await.map_err(|_| sink_closed())
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
        // Best-effort: buffer and return immediately. A closed receiver means
        // the drain task stopped; surface a diagnostic the caller may log, but
        // never block or short-circuit the surrounding workflow (sink
        // contract).
        self.tx
            .send(DrainMessage::Event(Box::new(event)))
            .map_err(|_| sink_closed())
    }
}

async fn drain_loop(
    log: Arc<dyn DurableEventLog>,
    config: EventBatchConfig,
    mut rx: mpsc::UnboundedReceiver<DrainMessage>,
) {
    let max_batch = config.max_batch.max(1);
    loop {
        // Block until a window opens (or exit once every sender is dropped).
        let first = match rx.recv().await {
            Some(message) => message,
            None => return,
        };

        let mut batch: Vec<RuntimeEvent> = Vec::new();
        let mut acks: Vec<oneshot::Sender<()>> = Vec::new();
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

        flush_batch(&log, std::mem::take(&mut batch)).await;
        for ack in acks {
            // Receiver may have given up; ignore.
            let _ = ack.send(());
        }

        if closed {
            return;
        }
    }
}

async fn flush_batch(log: &Arc<dyn DurableEventLog>, batch: Vec<RuntimeEvent>) {
    if batch.is_empty() {
        return;
    }
    // Isolate the flush in its own task so a panic inside a backend
    // `append_batch` (driver bug, serialization edge) cannot tear down the
    // long-lived drain loop and silently stop ALL runtime event logging for
    // the process. A panic here is logged and the next window still drains.
    // Awaiting the handle keeps flushes serialized → global append order is
    // preserved.
    let log = Arc::clone(log);
    match tokio::spawn(async move { log.append_batch(batch).await }).await {
        Ok(results) => {
            for result in results {
                if let Err(error) = result {
                    tracing::warn!(
                        target = "ironclaw::reborn::event_store::coalescing",
                        %error,
                        "durable event append failed during coalescing flush"
                    );
                }
            }
        }
        Err(join_error) => {
            tracing::error!(
                target = "ironclaw::reborn::event_store::coalescing",
                %join_error,
                "coalescing event flush panicked; dropping this batch and continuing"
            );
        }
    }
}
