//! Contract tests for the write-behind [`CoalescingEventSink`].
//!
//! Proves the round-trip reduction (N emits → one batched append call, zero
//! single appends), order/content preservation, and the bounded crash-loss
//! tail (buffered-but-unflushed events are not yet durable; everything flushed
//! survives).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_events::{
    DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay, EventSink,
    EventStreamKey, InMemoryDurableEventLog, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ProjectId, ResourceScope, TenantId, UserId,
};
use ironclaw_reborn_event_store::{CoalescingEventSink, EventBatchConfig};

fn capability_id() -> CapabilityId {
    CapabilityId::new("demo.echo").expect("capability id")
}

fn scope_for(user: &str, project: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("default").expect("agent id")),
        project_id: Some(ProjectId::new(project).expect("project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Wraps an inner [`DurableEventLog`], counting how many times each entry
/// point is invoked so a test can prove the sink coalesces single `emit`s into
/// one `append_batch` call rather than N `append`s.
///
/// When `inject_partial_failure` is set to `true`, `append_batch` returns a
/// partial-failure result vector: the first event succeeds (committed to the
/// inner log) and the remaining events fail with an injected error. This lets
/// tests verify that `flush()` propagates a per-event rejection as `Err`.
struct CountingEventLog {
    inner: InMemoryDurableEventLog,
    append_calls: AtomicUsize,
    append_batch_calls: AtomicUsize,
    batched_events: AtomicUsize,
    /// Size of each individual `append_batch` call, in call order.
    batch_sizes: std::sync::Mutex<Vec<usize>>,
    /// When `true`, `append_batch` injects an error on every event after the
    /// first, simulating a partial backend rejection.
    inject_partial_failure: std::sync::atomic::AtomicBool,
}

impl CountingEventLog {
    fn new() -> Self {
        Self {
            inner: InMemoryDurableEventLog::new(),
            append_calls: AtomicUsize::new(0),
            append_batch_calls: AtomicUsize::new(0),
            batched_events: AtomicUsize::new(0),
            batch_sizes: std::sync::Mutex::new(Vec::new()),
            inject_partial_failure: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl DurableEventLog for CountingEventLog {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.append(event).await
    }

    async fn append_batch(
        &self,
        events: Vec<RuntimeEvent>,
    ) -> Vec<Result<EventLogEntry<RuntimeEvent>, EventError>> {
        self.append_batch_calls.fetch_add(1, Ordering::SeqCst);
        self.batched_events
            .fetch_add(events.len(), Ordering::SeqCst);
        self.batch_sizes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(events.len());
        if self
            .inject_partial_failure
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            // Partial rejection: commit only the first event to the inner log;
            // return an injected error for every subsequent event.
            let mut results: Vec<Result<EventLogEntry<RuntimeEvent>, EventError>> =
                Vec::with_capacity(events.len());
            let mut iter = events.into_iter();
            if let Some(first) = iter.next() {
                results.push(self.inner.append(first).await);
            }
            for _ in iter {
                results.push(Err(EventError::Sink {
                    reason: "injected test failure".to_string(),
                }));
            }
            results
        } else {
            self.inner.append_batch(events).await
        }
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        self.inner
            .read_after_cursor(stream, filter, after, limit)
            .await
    }

    async fn head_cursor(
        &self,
        stream: &EventStreamKey,
        after: EventCursor,
    ) -> Result<EventCursor, EventError> {
        self.inner.head_cursor(stream, after).await
    }
}

#[tokio::test]
async fn emits_coalesce_into_a_single_batched_append_preserving_order() {
    let log = Arc::new(CountingEventLog::new());
    let sink = CoalescingEventSink::new(
        Arc::clone(&log) as Arc<dyn DurableEventLog>,
        EventBatchConfig {
            max_batch: 256,
            flush_interval: Duration::from_millis(50),
        },
    );

    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    const N: usize = 21;
    for _ in 0..N {
        sink.emit(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("emit buffers");
    }

    sink.flush().await.expect("flush drains the buffer");

    // The 21 single emits collapsed into ONE batched append carrying all 21
    // events — zero per-event single appends.
    assert_eq!(
        log.append_calls.load(Ordering::SeqCst),
        0,
        "no single-row appends on the coalesced path"
    );
    assert_eq!(
        log.append_batch_calls.load(Ordering::SeqCst),
        1,
        "exactly one batched append round-trip"
    );
    assert_eq!(log.batched_events.load(Ordering::SeqCst), N);

    // Order + content preserved: 21 events, monotonic cursors, in emit order.
    let replay = log
        .read_after_cursor(&stream, &ReadScope::default(), None, 1000)
        .await
        .expect("replay");
    assert_eq!(replay.entries.len(), N);
    for window in replay.entries.windows(2) {
        assert!(
            window[0].cursor.as_u64() < window[1].cursor.as_u64(),
            "cursors are monotonic in emit order"
        );
    }
}

#[tokio::test]
async fn crash_before_flush_loses_only_the_unflushed_tail() {
    let log = Arc::new(CountingEventLog::new());
    // Long interval so a buffered batch stays unflushed until we call flush():
    // this is the window a crash would lose, and nothing more.
    let sink = CoalescingEventSink::new(
        Arc::clone(&log) as Arc<dyn DurableEventLog>,
        EventBatchConfig {
            max_batch: 1000,
            flush_interval: Duration::from_secs(10),
        },
    );

    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    // First batch: emit then explicitly flush → durable.
    for _ in 0..5 {
        sink.emit(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .unwrap();
    }
    sink.flush().await.expect("flush batch 1");

    let after_first = log
        .read_after_cursor(&stream, &ReadScope::default(), None, 1000)
        .await
        .unwrap();
    assert_eq!(after_first.entries.len(), 5, "flushed batch is durable");

    // Second batch: emit but DO NOT flush. With the long interval these stay
    // buffered. Yield so the drain task has a chance to (not) write them.
    for _ in 0..3 {
        sink.emit(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // A crash here loses ONLY the 3 unflushed events; the 5 flushed survive.
    let mid_crash = log
        .read_after_cursor(&stream, &ReadScope::default(), None, 1000)
        .await
        .unwrap();
    assert_eq!(
        mid_crash.entries.len(),
        5,
        "unflushed tail is not yet durable — bounded crash loss"
    );

    // A proper flush then commits the tail with no loss or reordering.
    sink.flush().await.expect("flush batch 2");
    let after_second = log
        .read_after_cursor(&stream, &ReadScope::default(), None, 1000)
        .await
        .unwrap();
    assert_eq!(after_second.entries.len(), 8);
    for window in after_second.entries.windows(2) {
        assert!(window[0].cursor.as_u64() < window[1].cursor.as_u64());
    }
}

#[tokio::test]
async fn coalescing_sink_flush_splits_burst_at_max_batch() {
    // A burst larger than `max_batch` must be split into multiple
    // `append_batch` calls, each carrying at most `max_batch` events, so
    // memory and per-statement parameter counts stay bounded.  The sum of all
    // flushed events must equal the burst and the durable log must contain
    // them in emit order.
    const MAX_BATCH: usize = 4;
    const BURST: usize = 10; // 10 > MAX_BATCH → expected splits: [4, 4, 2]

    let log = Arc::new(CountingEventLog::new());
    let sink = CoalescingEventSink::new(
        Arc::clone(&log) as Arc<dyn DurableEventLog>,
        EventBatchConfig {
            max_batch: MAX_BATCH,
            // Large interval so the drain loop never fires on the timer;
            // the final batch is released only when flush() sends its ack.
            flush_interval: Duration::from_secs(10),
        },
    );

    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    // Give each event a distinct capability_id so that a batch-boundary
    // permutation would produce wrong content, not just pass a monotonic-cursor
    // check with identical records.
    let emitted_ids: Vec<CapabilityId> = (0..BURST)
        .map(|i| CapabilityId::new(format!("demo.event{i}")).expect("valid capability id"))
        .collect();
    for cap_id in &emitted_ids {
        sink.emit(RuntimeEvent::dispatch_requested(
            scope.clone(),
            cap_id.clone(),
        ))
        .await
        .expect("emit buffers without blocking");
    }

    sink.flush().await.expect("flush drains entire buffer");

    // The burst must have been split into ceil(10/4) = 3 separate batches.
    let sizes: Vec<usize> = log
        .batch_sizes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        sizes,
        vec![4, 4, 2],
        "burst of {BURST} with max_batch={MAX_BATCH} must split into [4,4,2]"
    );
    assert_eq!(
        log.append_batch_calls.load(Ordering::SeqCst),
        3,
        "exactly three batched append round-trips"
    );
    assert_eq!(
        log.batched_events.load(Ordering::SeqCst),
        BURST,
        "all {BURST} events must be durably written"
    );
    assert_eq!(
        log.append_calls.load(Ordering::SeqCst),
        0,
        "no single-row appends on the coalesced path"
    );

    // Order + content preservation: each replayed event's capability_id must
    // match the emitted order exactly. Monotonic cursor check alone would pass
    // even if records were permuted within or across batch boundaries — checking
    // content catches that class of bug.
    let replay = log
        .read_after_cursor(&stream, &ReadScope::default(), None, 100)
        .await
        .expect("replay after burst flush");
    assert_eq!(replay.entries.len(), BURST);
    for (i, entry) in replay.entries.iter().enumerate() {
        assert_eq!(
            entry.record.capability_id, emitted_ids[i],
            "event {i} must appear in emit order"
        );
    }
    for window in replay.entries.windows(2) {
        assert!(
            window[0].cursor.as_u64() < window[1].cursor.as_u64(),
            "cursors must be monotonic in emit order across batch boundaries"
        );
    }
}

#[tokio::test]
async fn flush_propagates_partial_append_batch_failure() {
    // Verify that a per-event rejection inside `append_batch` propagates all
    // the way through `flush()` as an `Err`. The sink's `flush()` contract
    // guarantees `Ok(())` only when every queued event landed durably; callers
    // rely on that guarantee for graceful-shutdown sequencing.
    let log = Arc::new(CountingEventLog::new());
    log.inject_partial_failure
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let sink = CoalescingEventSink::new(
        Arc::clone(&log) as Arc<dyn DurableEventLog>,
        EventBatchConfig {
            max_batch: 256,
            flush_interval: Duration::from_millis(50),
        },
    );

    let scope = scope_for("alice", "project-a");
    // Emit 3 events so `append_batch` has at least 2 items — the mock commits
    // the first and rejects the rest, making this a partial failure.
    for _ in 0..3 {
        sink.emit(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("emit must buffer without blocking");
    }

    let result = sink.flush().await;
    assert!(
        result.is_err(),
        "flush must return Err when append_batch reports a per-event rejection"
    );
}
