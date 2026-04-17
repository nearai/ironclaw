//! Per-thread fanout for the agent main loop.
//!
//! Routes each [`IncomingMessage`] to a per-conversation-thread `mpsc` queue,
//! with one worker task per active bucket. Same-thread traffic stays strictly
//! serial (preserving [`Session`]/`Thread` invariants); cross-thread traffic
//! runs concurrently, bounded by a global semaphore.
//!
//! The bucket key is computed by [`ThreadFanout::bucket_key`]; see
//! [`docs/plans/2026-04-16-013-refactor-ironclaw-per-thread-fanout-agent-loop-plan.md`]
//! for the full decision matrix.
//!
//! This module does not know about [`Agent`]; all message handling is dispatched
//! through a [`FanoutHandler`] trait object, which lets tests stub the handler
//! and production wire it to `Agent::process_one`.
//!
//! [`IncomingMessage`]: crate::channels::IncomingMessage
//! [`Session`]: crate::agent::session::Session
//! [`Agent`]: crate::agent::Agent

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore, mpsc};

use crate::agent::submission::{Submission, SubmissionParser};
use crate::channels::IncomingMessage;

/// Configuration knobs for the thread-fanout layer.
#[derive(Debug, Clone)]
pub struct FanoutConfig {
    /// Per-bucket data-channel capacity. When a bucket's queue is full, new
    /// dispatches to that bucket are rejected with
    /// [`DispatchError::BucketFull`]. Default 16.
    pub bucket_queue_capacity: usize,
    /// Per-bucket control-channel capacity. Control messages are small and
    /// priority; 8 is generous. Default 8.
    pub control_queue_capacity: usize,
    /// Bucket idle timeout before reap. A bucket with no data or control
    /// activity for this long is removed from the registry. Default 300 s.
    pub idle_timeout: Duration,
    /// Global cap on concurrently executing turns. Each bucket worker
    /// acquires one semaphore permit before invoking the handler. Default 32.
    pub max_concurrent_turns: usize,
}

impl Default for FanoutConfig {
    fn default() -> Self {
        Self {
            bucket_queue_capacity: 16,
            control_queue_capacity: 8,
            idle_timeout: Duration::from_secs(300),
            max_concurrent_turns: 32,
        }
    }
}

/// Priority signals sent to a bucket worker on the control lane.
///
/// Control messages bypass the data FIFO — the worker's select is biased on
/// the control channel so a [`ControlMsg::Interrupt`] can short-circuit a
/// long-running turn without waiting for prior data to drain.
#[derive(Debug, Clone)]
pub enum ControlMsg {
    /// Request the worker to abandon the in-flight turn and continue.
    /// (Wired in Unit 4; accepted but no-op in earlier units.)
    Interrupt,
    /// Graceful stop: finish any in-flight work, then exit.
    Quit,
    /// Global shutdown: broadcast during fanout-wide drain.
    Shutdown,
}

/// Errors returned by [`ThreadFanout::dispatch`].
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// The target bucket's data queue is full; caller should surface a
    /// "busy" status to the user and/or drop.
    #[error("bucket '{0}' full (backpressure)")]
    BucketFull(String),
    /// The fanout has been marked for shutdown and will not accept new work.
    #[error("fanout is shutting down")]
    ShuttingDown,
}

/// Handler invoked by a bucket worker for each message it receives.
///
/// In production this is wired to `Agent::process_one`. Tests pass a stub
/// that records messages for later assertion.
#[async_trait]
pub trait FanoutHandler: Send + Sync + 'static {
    /// Handle a single message. The worker has already acquired a global
    /// semaphore permit, so the handler may take as long as the unbounded
    /// turn requires without blocking other buckets beyond the permit cap.
    async fn handle(&self, msg: IncomingMessage);
}

/// Per-bucket registry entry.
struct BucketHandle {
    data_tx: mpsc::Sender<IncomingMessage>,
    control_tx: mpsc::Sender<ControlMsg>,
    /// Worker JoinHandle retained so [`ThreadFanout::shutdown`] can await
    /// cooperative drain and abort laggards on timeout.
    worker: tokio::task::JoinHandle<()>,
}

/// The per-thread fanout layer.
///
/// Cloneable internal state — the struct itself is not `Clone` because
/// `handler: Arc<dyn FanoutHandler>` already provides shared ownership and
/// the registry is behind an `Arc<RwLock<_>>`.
pub struct ThreadFanout {
    buckets: Arc<RwLock<HashMap<String, BucketHandle>>>,
    semaphore: Arc<Semaphore>,
    config: FanoutConfig,
    handler: Arc<dyn FanoutHandler>,
    shutdown: Arc<AtomicBool>,
    /// Observability counters — cheap atomics to avoid per-dispatch locks.
    stats: Arc<FanoutStats>,
}

/// Lightweight counters exposed for observability.
///
/// These atomics are the authoritative source for `agent_fanout_*`
/// metrics. The `log` observability backend logs structured fields at
/// key transitions; a future Prometheus backend can expose these
/// directly. Keep all label cardinality unbounded at O(1) — no per-bucket
/// labeled metrics that could blow up with millions of threads.
#[derive(Debug, Default)]
pub struct FanoutStats {
    /// Total buckets created (monotonically increasing).
    pub buckets_created: AtomicU64,
    /// Total buckets reaped on idle timeout.
    pub buckets_reaped: AtomicU64,
    /// Total dispatch calls received (including drops).
    pub dispatches_total: AtomicU64,
    /// Drops caused by bucket queue full (backpressure).
    pub drops_bucket_full: AtomicU64,
    /// Drops caused by the fanout being in shutdown state.
    pub drops_shutdown: AtomicU64,
    /// Total control-channel messages sent (Interrupt/Quit/Shutdown).
    pub control_sent: AtomicU64,
    /// Total messages that completed their handler (non-interrupted).
    pub turns_completed: AtomicU64,
    /// Total messages whose handler future was dropped by an interrupt.
    pub turns_interrupted: AtomicU64,
    /// Accumulated handler wall time in microseconds. Average per turn =
    /// `handle_latency_us_total / turns_completed` (Prometheus can derive
    /// histogram from this by combining with the count).
    pub handle_latency_us_total: AtomicU64,
    /// Accumulated enqueue→worker-pickup latency in microseconds.
    pub enqueue_latency_us_total: AtomicU64,
}

/// A shallow snapshot of [`FanoutStats`] for logging / tests.
#[derive(Debug, Clone)]
pub struct FanoutSnapshot {
    pub buckets_created: u64,
    pub buckets_reaped: u64,
    pub active_buckets: usize,
    pub dispatches_total: u64,
    pub drops_bucket_full: u64,
    pub drops_shutdown: u64,
    pub control_sent: u64,
    pub turns_completed: u64,
    pub turns_interrupted: u64,
    pub handle_latency_us_total: u64,
    pub enqueue_latency_us_total: u64,
}

impl ThreadFanout {
    /// Create a new fanout with the given config and handler.
    pub fn new(config: FanoutConfig, handler: Arc<dyn FanoutHandler>) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_turns));
        Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            semaphore,
            config,
            handler,
            shutdown: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(FanoutStats::default()),
        }
    }

    /// Snapshot of internal counters. See [`FanoutStats`].
    pub fn stats(&self) -> Arc<FanoutStats> {
        Arc::clone(&self.stats)
    }

    /// Current number of active buckets. O(1) read under shared lock.
    pub async fn active_buckets(&self) -> usize {
        self.buckets.read().await.len()
    }

    /// Snapshot all counters plus active bucket count in one call.
    /// Intended for periodic exporters and debug logging.
    pub async fn snapshot(&self) -> FanoutSnapshot {
        FanoutSnapshot {
            buckets_created: self.stats.buckets_created.load(Ordering::Relaxed),
            buckets_reaped: self.stats.buckets_reaped.load(Ordering::Relaxed),
            active_buckets: self.active_buckets().await,
            dispatches_total: self.stats.dispatches_total.load(Ordering::Relaxed),
            drops_bucket_full: self.stats.drops_bucket_full.load(Ordering::Relaxed),
            drops_shutdown: self.stats.drops_shutdown.load(Ordering::Relaxed),
            control_sent: self.stats.control_sent.load(Ordering::Relaxed),
            turns_completed: self.stats.turns_completed.load(Ordering::Relaxed),
            turns_interrupted: self.stats.turns_interrupted.load(Ordering::Relaxed),
            handle_latency_us_total: self.stats.handle_latency_us_total.load(Ordering::Relaxed),
            enqueue_latency_us_total: self.stats.enqueue_latency_us_total.load(Ordering::Relaxed),
        }
    }

    /// Compute the bucket key for a message. Pure function — unit-testable.
    ///
    /// See the decision matrix in the plan; in summary:
    /// - If `thread_id` is present and non-empty: `"{channel}:{thread_id}"`.
    /// - Otherwise if `is_internal`: `"internal:{channel}:user:{user_id}"`.
    /// - Otherwise: `"{channel}:user:{user_id}"`.
    ///
    /// The `channel` prefix prevents cross-channel key collisions (e.g.
    /// Slack and DingTalk both returning `cid-xxx` as a native conversation
    /// id); the `thread_id` is taken as-computed by the adapter so that the
    /// channel's own `GroupSessionScope` policy is preserved.
    pub fn bucket_key(msg: &IncomingMessage) -> String {
        let has_thread = msg
            .thread_id
            .as_deref()
            .map(|t| !t.is_empty())
            .unwrap_or(false);
        match (msg.is_internal, has_thread) {
            (_, true) => {
                // Safe: has_thread guarantees Some and non-empty.
                format!("{}:{}", msg.channel, msg.thread_id.as_deref().unwrap_or(""))
            }
            (false, false) => format!("{}:user:{}", msg.channel, msg.user_id),
            (true, false) => format!("internal:{}:user:{}", msg.channel, msg.user_id),
        }
    }

    /// Dispatch a message to its per-thread bucket.
    ///
    /// Fast path (existing bucket): read-lock, `try_send` on the Sender.
    /// Slow path (new or closed bucket): write-lock, check-insert-spawn.
    ///
    /// When the message is a recognized control submission
    /// (`/interrupt`, `/stop`, `/quit`, `/exit`, `/shutdown`), it is
    /// routed to the bucket's control channel instead of the data queue.
    /// This bypasses any pending data-queue depth, so an interrupt lands
    /// within milliseconds even behind a 60 s-running turn. Bucketed
    /// control messages only apply to an existing bucket — if the target
    /// bucket does not exist, the control is a no-op (nothing to
    /// interrupt), which matches the historical semantics.
    pub async fn dispatch(&self, msg: IncomingMessage) -> Result<(), DispatchError> {
        if self.shutdown.load(Ordering::SeqCst) {
            self.stats.drops_shutdown.fetch_add(1, Ordering::Relaxed);
            return Err(DispatchError::ShuttingDown);
        }
        self.stats.dispatches_total.fetch_add(1, Ordering::Relaxed);
        let key = Self::bucket_key(&msg);

        // Priority peek: interrupt/quit bypass the data FIFO.
        if let Some(ctrl) = classify_control(&msg.content) {
            match self.send_control(&key, ctrl).await {
                Ok(_) => return Ok(()),
                Err(e) => return Err(e),
            }
        }

        // Fast path: existing bucket. On Closed we recover the msg and fall
        // through to the slow path; on None we pass the msg through.
        let msg = {
            let guard = self.buckets.read().await;
            match guard.get(&key) {
                Some(handle) => match handle.data_tx.try_send(msg) {
                    Ok(()) => return Ok(()),
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.stats.drops_bucket_full.fetch_add(1, Ordering::Relaxed);
                        return Err(DispatchError::BucketFull(key));
                    }
                    Err(mpsc::error::TrySendError::Closed(returned)) => returned,
                },
                None => msg,
            }
        };

        self.dispatch_slow(key, msg).await
    }

    /// Sibling of [`Self::dispatch`] for the slow path. Takes ownership of
    /// the message so we never lose it across lock transitions.
    async fn dispatch_slow(&self, key: String, msg: IncomingMessage) -> Result<(), DispatchError> {
        let mut guard = self.buckets.write().await;
        // Double-check under write lock (TOCTOU guard, mirrors
        // `Scheduler.schedule`'s check-insert pattern).
        if let Some(handle) = guard.get(&key) {
            match handle.data_tx.try_send(msg) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.stats.drops_bucket_full.fetch_add(1, Ordering::Relaxed);
                    return Err(DispatchError::BucketFull(key));
                }
                Err(mpsc::error::TrySendError::Closed(returned)) => {
                    // Stale entry — worker already exited. Remove and recreate.
                    guard.remove(&key);
                    return self.create_bucket_and_send(key, returned, guard);
                }
            }
        }
        self.create_bucket_and_send(key, msg, guard)
    }

    fn create_bucket_and_send(
        &self,
        key: String,
        msg: IncomingMessage,
        mut guard: tokio::sync::RwLockWriteGuard<'_, HashMap<String, BucketHandle>>,
    ) -> Result<(), DispatchError> {
        let (data_tx, data_rx) =
            mpsc::channel::<IncomingMessage>(self.config.bucket_queue_capacity);
        let (control_tx, control_rx) =
            mpsc::channel::<ControlMsg>(self.config.control_queue_capacity);

        // Send the first message to the buffer *before* spawning so we never
        // lose it to a spawn-ordering race.
        if let Err(err) = data_tx.try_send(msg) {
            return Err(match err {
                mpsc::error::TrySendError::Full(_) => DispatchError::BucketFull(key),
                mpsc::error::TrySendError::Closed(_) => DispatchError::ShuttingDown,
            });
        }

        let key_for_worker = key.clone();
        let handler = Arc::clone(&self.handler);
        let semaphore = Arc::clone(&self.semaphore);
        let buckets = Arc::clone(&self.buckets);
        let stats = Arc::clone(&self.stats);
        let idle_timeout = self.config.idle_timeout;

        let worker = tokio::spawn(async move {
            bucket_worker(
                key_for_worker,
                data_rx,
                control_rx,
                handler,
                semaphore,
                buckets,
                stats,
                idle_timeout,
            )
            .await;
        });

        guard.insert(
            key,
            BucketHandle {
                data_tx,
                control_tx,
                worker,
            },
        );
        self.stats.buckets_created.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Send a control-channel message to a specific bucket, if present.
    ///
    /// Returns `Ok(true)` if the bucket was found and the control message
    /// was sent (or queued), `Ok(false)` if no such bucket exists. Never
    /// blocks; uses `try_send`.
    pub async fn send_control(&self, key: &str, msg: ControlMsg) -> Result<bool, DispatchError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(DispatchError::ShuttingDown);
        }
        let guard = self.buckets.read().await;
        if let Some(handle) = guard.get(key) {
            match handle.control_tx.try_send(msg) {
                Ok(()) => {
                    self.stats.control_sent.fetch_add(1, Ordering::Relaxed);
                    Ok(true)
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Control channel is small and priority; being full is a
                    // real signal of runaway control traffic, not normal.
                    Err(DispatchError::BucketFull(format!("{key}:control")))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    /// Broadcast a control message to every bucket. Used by graceful shutdown.
    pub async fn broadcast_control(&self, msg: ControlMsg) -> usize {
        let guard = self.buckets.read().await;
        let mut sent = 0usize;
        for (_, handle) in guard.iter() {
            if handle.control_tx.try_send(msg.clone()).is_ok() {
                sent += 1;
            }
        }
        self.stats
            .control_sent
            .fetch_add(sent as u64, Ordering::Relaxed);
        sent
    }

    /// Initiate a cooperative drain and shutdown.
    ///
    /// 1. Mark the fanout as shutting down so subsequent dispatches fail fast.
    /// 2. Broadcast [`ControlMsg::Shutdown`] to every bucket.
    /// 3. Await every worker's JoinHandle with a bounded timeout.
    /// 4. Abort laggards and log how many were still in flight.
    ///
    /// Safe to call more than once; subsequent calls return quickly.
    pub async fn shutdown(&self, grace: Duration) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return; // Already shutting down.
        }

        let _ = self.broadcast_control(ControlMsg::Shutdown).await;

        // Collect (key, handle) pairs under write lock. Drain drops the
        // BucketHandle's senders, closing the channels — workers' selects
        // exit cooperatively once their current turn finishes (or
        // immediately, if idle).
        let workers: Vec<(String, tokio::task::JoinHandle<()>)> = {
            let mut guard = self.buckets.write().await;
            guard.drain().map(|(k, h)| (k, h.worker)).collect()
        };

        let count = workers.len();
        if count == 0 {
            tracing::debug!("fanout shutdown: no active buckets");
            return;
        }

        // Move each handle into its own `Arc<Mutex<Option<_>>>` slot so
        // the join future and the post-timeout abort loop can cooperate
        // without unsafe ownership tricks.
        let slots: Vec<Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>> = workers
            .into_iter()
            .map(|(_, h)| Arc::new(tokio::sync::Mutex::new(Some(h))))
            .collect();

        let join_futs = slots.iter().map(|slot| {
            let slot = Arc::clone(slot);
            async move {
                let handle_opt = {
                    let mut g = slot.lock().await;
                    g.take()
                };
                if let Some(handle) = handle_opt {
                    let _ = handle.await;
                }
            }
        });
        let join_all = futures::future::join_all(join_futs);

        match tokio::time::timeout(grace, join_all).await {
            Ok(_) => {
                tracing::debug!(workers = count, "fanout shutdown drained cleanly");
            }
            Err(_) => {
                let mut aborted = 0usize;
                for slot in &slots {
                    let handle_opt = {
                        let mut g = slot.lock().await;
                        g.take()
                    };
                    if let Some(handle) = handle_opt {
                        handle.abort();
                        aborted += 1;
                    }
                }
                tracing::warn!(
                    workers = count,
                    aborted,
                    grace_ms = grace.as_millis() as u64,
                    "fanout shutdown timed out; aborted {aborted} still-running worker(s)"
                );
            }
        }
    }
}

/// Classify a message's content into a priority control signal, if any.
///
/// Returns `Some(ControlMsg::Interrupt)` for `/interrupt`/`/stop`,
/// `Some(ControlMsg::Quit)` for `/quit`/`/exit`/`/shutdown`, and `None`
/// for any other content (including `/undo`, `/compact`, and ordinary
/// user input) — those still go through the data FIFO so per-thread
/// ordering is preserved.
///
/// Pure function; does not mutate or consume the message.
pub fn classify_control(content: &str) -> Option<ControlMsg> {
    match SubmissionParser::parse(content) {
        Submission::Interrupt => Some(ControlMsg::Interrupt),
        Submission::Quit => Some(ControlMsg::Quit),
        _ => None,
    }
}

/// Per-bucket worker task body.
///
/// Outer select is biased: control > data > idle-timer. While a turn is in
/// flight, an inner select races the handler future against the control
/// channel — this lets `ControlMsg::Interrupt` drop the in-flight future
/// (cooperative cancel) and `ControlMsg::Quit`/`Shutdown` exit the worker.
#[allow(clippy::too_many_arguments)]
async fn bucket_worker(
    key: String,
    mut data_rx: mpsc::Receiver<IncomingMessage>,
    mut control_rx: mpsc::Receiver<ControlMsg>,
    handler: Arc<dyn FanoutHandler>,
    semaphore: Arc<Semaphore>,
    buckets: Arc<RwLock<HashMap<String, BucketHandle>>>,
    stats: Arc<FanoutStats>,
    idle_timeout: Duration,
) {
    tracing::debug!(bucket = %key, "fanout worker started");
    loop {
        tokio::select! {
            biased;

            ctrl = control_rx.recv() => {
                match ctrl {
                    Some(ControlMsg::Quit) | Some(ControlMsg::Shutdown) => {
                        tracing::debug!(bucket = %key, "fanout worker received Quit/Shutdown");
                        break;
                    }
                    Some(ControlMsg::Interrupt) => {
                        // No in-flight turn to cancel — noop.
                        tracing::debug!(bucket = %key, "interrupt on idle bucket (noop)");
                    }
                    None => {
                        tracing::debug!(bucket = %key, "control channel closed");
                        break;
                    }
                }
            }

            msg = data_rx.recv() => {
                let Some(msg) = msg else {
                    tracing::debug!(bucket = %key, "data channel closed");
                    break;
                };
                // Acquire a permit to bound global concurrency. If the
                // semaphore is closed (global shutdown), exit.
                let Ok(permit) = Arc::clone(&semaphore).acquire_owned().await else {
                    tracing::debug!(bucket = %key, "semaphore closed during permit acquisition");
                    break;
                };

                // Approximate enqueue latency: time since the channel
                // adapter stamped the message.
                let enqueue_us = (chrono::Utc::now() - msg.received_at)
                    .num_microseconds()
                    .unwrap_or(0)
                    .max(0) as u64;
                stats
                    .enqueue_latency_us_total
                    .fetch_add(enqueue_us, Ordering::Relaxed);

                // Race handler completion against control-channel signals.
                // Dropping the handler future on Interrupt cancels it
                // cooperatively — tokio runs destructors on any owned
                // resources, so this is panic-safe.
                let handler_clone = Arc::clone(&handler);
                let handle_start = Instant::now();
                let handle_fut = async move { handler_clone.handle(msg).await };
                tokio::pin!(handle_fut);

                let mut interrupted = false;
                let break_after = tokio::select! {
                    biased;
                    ctrl = control_rx.recv() => {
                        match ctrl {
                            Some(ControlMsg::Interrupt) => {
                                tracing::debug!(
                                    bucket = %key,
                                    "interrupt cancelled in-flight turn"
                                );
                                interrupted = true;
                                // Future dropped here via scope exit.
                                false
                            }
                            Some(ControlMsg::Quit) | Some(ControlMsg::Shutdown) => {
                                tracing::debug!(
                                    bucket = %key,
                                    "quit/shutdown during turn — exiting"
                                );
                                true
                            }
                            None => {
                                tracing::debug!(bucket = %key, "control closed during turn");
                                true
                            }
                        }
                    }
                    _ = &mut handle_fut => { false }
                };
                let handle_us = handle_start.elapsed().as_micros() as u64;
                stats
                    .handle_latency_us_total
                    .fetch_add(handle_us, Ordering::Relaxed);
                if interrupted {
                    stats.turns_interrupted.fetch_add(1, Ordering::Relaxed);
                } else if !break_after {
                    stats.turns_completed.fetch_add(1, Ordering::Relaxed);
                }
                drop(permit);
                if break_after {
                    break;
                }
            }

            _ = tokio::time::sleep(idle_timeout) => {
                // Double-check under write lock. Since write-lock acquisition
                // waits for any in-flight dispatcher's read lock to release,
                // and `try_send` is synchronous, any message enqueued by a
                // dispatcher before we got the write lock is already
                // reflected in `data_rx.len()`.
                let mut guard = buckets.write().await;
                if data_rx.is_empty() && control_rx.is_empty() {
                    if guard.remove(&key).is_some() {
                        stats.buckets_reaped.fetch_add(1, Ordering::Relaxed);
                    }
                    tracing::debug!(bucket = %key, "fanout worker reaped on idle");
                    break;
                }
                // Not empty — continue; the next select tick drains it.
            }
        }
    }
    tracing::debug!(bucket = %key, "fanout worker exiting");
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    struct RecordingHandler {
        seen: Arc<Mutex<Vec<(String, String)>>>,
        sleep: Duration,
    }

    #[async_trait]
    impl FanoutHandler for RecordingHandler {
        async fn handle(&self, msg: IncomingMessage) {
            if !self.sleep.is_zero() {
                tokio::time::sleep(self.sleep).await;
            }
            self.seen
                .lock()
                .await
                .push((Self::key(&msg), msg.content.clone()));
        }
    }

    type RecordedLog = Vec<(String, String)>;

    impl RecordingHandler {
        fn new(sleep: Duration) -> (Arc<Self>, Arc<Mutex<RecordedLog>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            (
                Arc::new(Self {
                    seen: Arc::clone(&seen),
                    sleep,
                }),
                seen,
            )
        }
        fn key(msg: &IncomingMessage) -> String {
            ThreadFanout::bucket_key(msg)
        }
    }

    fn msg(channel: &str, user: &str, thread: Option<&str>, content: &str) -> IncomingMessage {
        let mut m = IncomingMessage::new(channel, user, content);
        m.id = Uuid::new_v4();
        if let Some(t) = thread {
            m = m.with_thread(t);
        }
        m
    }

    fn internal_msg(
        channel: &str,
        user: &str,
        thread: Option<&str>,
        content: &str,
    ) -> IncomingMessage {
        let mut m = msg(channel, user, thread, content);
        m.is_internal = true;
        m
    }

    #[test]
    fn bucket_key_group_message_uses_thread_id() {
        let m = msg("dingtalk", "staff-alice", Some("cid-1:staff-alice"), "hi");
        assert_eq!(ThreadFanout::bucket_key(&m), "dingtalk:cid-1:staff-alice");
    }

    #[test]
    fn bucket_key_dm_uses_thread_id() {
        let m = msg("web", "u-1", Some("dm:u-1"), "hi");
        assert_eq!(ThreadFanout::bucket_key(&m), "web:dm:u-1");
    }

    #[test]
    fn bucket_key_fallback_when_no_thread() {
        let m = msg("dingtalk", "staff-bob", None, "hi");
        assert_eq!(ThreadFanout::bucket_key(&m), "dingtalk:user:staff-bob");
    }

    #[test]
    fn bucket_key_empty_thread_treated_as_none() {
        let m = msg("web", "u-1", Some(""), "hi");
        assert_eq!(ThreadFanout::bucket_key(&m), "web:user:u-1");
    }

    #[test]
    fn bucket_key_internal_without_thread_prefixed() {
        let m = internal_msg("heartbeat", "u-1", None, "tick");
        assert_eq!(ThreadFanout::bucket_key(&m), "internal:heartbeat:user:u-1");
    }

    #[test]
    fn bucket_key_internal_with_thread_uses_normal_prefix() {
        let m = internal_msg("heartbeat", "u-1", Some("synthetic-1"), "tick");
        assert_eq!(ThreadFanout::bucket_key(&m), "heartbeat:synthetic-1");
    }

    #[tokio::test]
    async fn dispatch_routes_to_same_bucket_in_order() {
        let (handler, seen) = RecordingHandler::new(Duration::from_millis(20));
        let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

        for content in ["one", "two", "three"] {
            fanout
                .dispatch(msg("web", "u-1", Some("t-1"), content))
                .await
                .expect("dispatch succeeds");
        }

        // Allow workers to drain.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let seen = seen.lock().await.clone();
        let bucket_msgs: Vec<_> = seen
            .iter()
            .filter(|(k, _)| k == "web:t-1")
            .map(|(_, c)| c.clone())
            .collect();
        assert_eq!(bucket_msgs, vec!["one", "two", "three"]);
    }

    #[tokio::test]
    async fn dispatch_creates_separate_buckets_for_different_threads() {
        let (handler, seen) = RecordingHandler::new(Duration::ZERO);
        let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

        for i in 0..3 {
            fanout
                .dispatch(msg("web", "u-1", Some("t-A"), &format!("a-{i}")))
                .await
                .expect("dispatch a");
        }
        for i in 0..2 {
            fanout
                .dispatch(msg("web", "u-1", Some("t-B"), &format!("b-{i}")))
                .await
                .expect("dispatch b");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(fanout.active_buckets().await, 2);
        let seen = seen.lock().await.clone();
        assert_eq!(seen.len(), 5);
    }

    #[tokio::test]
    async fn idle_timeout_reaps_bucket() {
        let (handler, _seen) = RecordingHandler::new(Duration::ZERO);
        let cfg = FanoutConfig {
            idle_timeout: Duration::from_millis(100),
            ..FanoutConfig::default()
        };
        let fanout = ThreadFanout::new(cfg, handler);

        fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "hi"))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(fanout.active_buckets().await, 1);

        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            fanout.active_buckets().await,
            0,
            "bucket should be reaped after idle window"
        );
        assert_eq!(fanout.stats.buckets_reaped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn reap_race_recreates_bucket_on_new_dispatch() {
        let (handler, seen) = RecordingHandler::new(Duration::ZERO);
        let cfg = FanoutConfig {
            idle_timeout: Duration::from_millis(50),
            ..FanoutConfig::default()
        };
        let fanout = ThreadFanout::new(cfg, handler);

        fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "first"))
            .await
            .unwrap();

        // Let the first bucket reap.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(fanout.active_buckets().await, 0);

        // New dispatch for the same key should create a fresh bucket.
        fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "second"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let seen = seen.lock().await.clone();
        let contents: Vec<_> = seen.iter().map(|(_, c)| c.clone()).collect();
        assert!(contents.contains(&"first".to_string()));
        assert!(contents.contains(&"second".to_string()));
    }

    #[tokio::test]
    async fn backpressure_surfaces_as_bucket_full() {
        // Hold each msg for a long time so the queue saturates; capacity 1.
        let (handler, _seen) = RecordingHandler::new(Duration::from_millis(500));
        let cfg = FanoutConfig {
            bucket_queue_capacity: 1,
            ..FanoutConfig::default()
        };
        let fanout = ThreadFanout::new(cfg, handler);

        // First message accepts and starts the worker.
        fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "1"))
            .await
            .expect("first accept");

        // Spam follow-ups tighter than the handler can drain. At least one
        // must observe BucketFull with key "web:t-1".
        let mut saw_full = false;
        for i in 2..20 {
            match fanout
                .dispatch(msg("web", "u-1", Some("t-1"), &format!("{i}")))
                .await
            {
                Err(DispatchError::BucketFull(k)) => {
                    assert_eq!(k, "web:t-1");
                    saw_full = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_full, "expected BucketFull under tight backpressure");
        assert!(fanout.stats.drops_bucket_full.load(Ordering::Relaxed) >= 1);
    }

    #[tokio::test]
    async fn shutdown_drains_cleanly_within_grace() {
        let (handler, _seen) = RecordingHandler::new(Duration::from_millis(20));
        let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

        for i in 0..3 {
            fanout
                .dispatch(msg("web", "u-1", Some(&format!("t-{i}")), "hi"))
                .await
                .unwrap();
        }

        fanout.shutdown(Duration::from_secs(1)).await;
        assert_eq!(fanout.active_buckets().await, 0);

        // Second call is a no-op.
        fanout.shutdown(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn shutdown_rejects_further_dispatches() {
        let (handler, _seen) = RecordingHandler::new(Duration::ZERO);
        let fanout = ThreadFanout::new(FanoutConfig::default(), handler);
        fanout.shutdown(Duration::from_millis(100)).await;

        let err = fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "late"))
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::ShuttingDown));
    }

    #[tokio::test]
    async fn send_control_reaches_target_bucket() {
        let (handler, _seen) = RecordingHandler::new(Duration::from_millis(50));
        let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

        fanout
            .dispatch(msg("web", "u-1", Some("t-1"), "hi"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let ok = fanout
            .send_control("web:t-1", ControlMsg::Interrupt)
            .await
            .unwrap();
        assert!(ok, "control should reach the existing bucket");

        let missing = fanout
            .send_control("web:nope", ControlMsg::Interrupt)
            .await
            .unwrap();
        assert!(!missing, "unknown bucket returns false");
    }

    #[tokio::test]
    async fn semaphore_caps_concurrent_turns() {
        let (handler, _seen) = RecordingHandler::new(Duration::from_millis(150));
        let cfg = FanoutConfig {
            max_concurrent_turns: 2,
            ..FanoutConfig::default()
        };
        let fanout = ThreadFanout::new(cfg, handler);

        let start = Instant::now();
        for i in 0..4 {
            fanout
                .dispatch(msg("web", "u-1", Some(&format!("t-{i}")), "hi"))
                .await
                .unwrap();
        }
        // With cap=2 and per-handler sleep=150ms, 4 msgs across 4 buckets
        // should take ~300ms (two "waves").
        tokio::time::sleep(Duration::from_millis(400)).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(300),
            "expected at least two waves, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(550),
            "expected fewer than three waves, elapsed={elapsed:?}"
        );
    }
}
