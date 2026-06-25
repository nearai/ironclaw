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
struct CountingEventLog {
    inner: InMemoryDurableEventLog,
    append_calls: AtomicUsize,
    append_batch_calls: AtomicUsize,
    batched_events: AtomicUsize,
}

impl CountingEventLog {
    fn new() -> Self {
        Self {
            inner: InMemoryDurableEventLog::new(),
            append_calls: AtomicUsize::new(0),
            append_batch_calls: AtomicUsize::new(0),
            batched_events: AtomicUsize::new(0),
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
        self.inner.append_batch(events).await
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
