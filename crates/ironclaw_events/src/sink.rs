use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::AuditEnvelope;

use crate::cursor::{EventCursor, EventLogEntry, EventReplay, EventStreamKey, ReadScope};
use crate::error::EventError;
use crate::runtime_event::RuntimeEvent;

/// Async event sink used by runtime/composition services.
///
/// **Best-effort observability.** The contract requires that a sink failure
/// **must not** change runtime outcomes. The trait returns `Result` so
/// implementations can surface diagnostics to a separate observer/log,
/// **never** so callers can `?`-propagate the error and short-circuit the
/// surrounding workflow.
///
/// Callers (dispatcher, process manager, host runtime) must:
///
/// 1. invoke `emit(...).await`;
/// 2. record any returned error to a diagnostics channel of their choice;
/// 3. continue with their original success/failure result.
///
/// A type-level enforcement of this contract (no-fail emit + separate
/// fallible diagnostics surface) is a deliberate follow-up; see the
/// "best-effort sink contract" follow-up issue.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError>;

    /// Flush any buffered events to durable storage. Synchronous sinks are
    /// already durable on `emit` return, so the default is a no-op. Write-behind
    /// sinks override this to drain their buffer (graceful shutdown, tests).
    async fn flush(&self) -> Result<(), EventError> {
        Ok(())
    }
}

/// Async audit sink used by control-plane services.
///
/// **Best-effort observability.** Same contract as [`EventSink`]: a sink
/// failure must not change approval resolution outcomes. The trait returns
/// `Result` so implementations can surface diagnostics, never so callers can
/// short-circuit on a sink error.
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError>;
}

// -----------------------------------------------------------------------------
// Explicit-error durable log traits
// -----------------------------------------------------------------------------

/// Durable runtime/process event log with explicit-error append and scoped
/// replay-after semantics.
///
/// `append` failures must be propagated. `read_after_cursor` is gated on
/// two-tier authority:
///
/// 1. The caller must validate that the requested [`EventStreamKey`] matches
///    the consumer's authorized stream before serving the result.
/// 2. The supplied [`ReadScope`] is enforced **by the implementation**, not
///    by the caller, so a project-scoped or thread-scoped consumer cannot
///    receive records from another project/thread within the same stream.
///
/// The implementation rejects cursors that predate the earliest retained
/// entry, or that exceed the current stream head, with
/// [`EventError::ReplayGap`].
#[async_trait]
pub trait DurableEventLog: Send + Sync {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError>;

    /// Append a batch of events, returning one result per event in input
    /// order. Implementations that can coalesce same-stream appends into a
    /// single backend round-trip should override this; the default impl is a
    /// correctness-preserving fallback that appends one at a time so a
    /// non-overriding log still works (it just pays one round-trip per event).
    ///
    /// Per-event `Result`s are returned (rather than a single `Result<Vec<_>>`)
    /// so a partial failure surfaces precisely without discarding the
    /// successfully-appended prefix. Ordering of the returned vec matches the
    /// input.
    async fn append_batch(
        &self,
        events: Vec<RuntimeEvent>,
    ) -> Vec<Result<EventLogEntry<RuntimeEvent>, EventError>> {
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            out.push(self.append(event).await);
        }
        out
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError>;

    /// Snapshot the stream's current head cursor (the cursor of the most
    /// recently appended record at the instant of the call), considering the
    /// whole `(tenant, user, agent)` stream regardless of deeper-scope
    /// filtering.
    ///
    /// `after` is a known-valid resume cursor for the caller (typically the
    /// subscription's `start_cursor`); implementations must treat it as the
    /// floor of the probe so the call never trips the earliest-retained
    /// `ReplayGap` guard for a still-valid resume position. The returned head
    /// is `>= after`. A cursor strictly beyond the current head is a foreign /
    /// future cursor and must be rejected with [`EventError::ReplayGap`] —
    /// mirroring the `read_after_cursor` contract.
    ///
    /// # Atomicity contract (REQUIRED — no default impl)
    ///
    /// The head must be read **atomically at the instant of the call** from a
    /// single authoritative tail observation (a tail counter, the stream's
    /// `next_cursor`, or the backend's last-assigned sequence number). It must
    /// NOT be derived by draining the stream page-by-page until an empty read:
    /// a record appended concurrently *during* such a drain would fold into the
    /// observed head and be mis-classified as replay. Each backend knows how to
    /// read its own tail cheaply, so this is a required operation rather than a
    /// default-implemented unbounded scan hidden behind a method whose docs
    /// promise atomicity.
    ///
    /// # Why this exists (PR #3931, Hole 1)
    ///
    /// Event-triggered subscriptions must distinguish *replay* (records that
    /// existed in the gap from `start_cursor` to head-at-startup, which may
    /// have already fired their side effects on a prior run) from *live*
    /// records (appended after the subscription started). The previous
    /// implementation treated "the first poll that returns no entries" as the
    /// replay/live boundary, which races: a live record appended before the
    /// first empty poll — or while a continuous backlog drains past the true
    /// head — was mis-marked as replay and could be wrongly deduped/skipped.
    /// Snapshotting the head **once, atomically, at subscription start** fixes
    /// the boundary: `cursor <= startup_head` is replay, everything else is
    /// live.
    async fn head_cursor(
        &self,
        stream: &EventStreamKey,
        after: EventCursor,
    ) -> Result<EventCursor, EventError>;
}

/// Durable control-plane audit log with explicit-error append and scoped
/// replay-after semantics. See [`DurableEventLog`] for cursor and replay
/// semantics.
#[async_trait]
pub trait DurableAuditLog: Send + Sync {
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError>;

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError>;
}

/// [`EventSink`] adapter that appends each emitted runtime event to a durable log.
#[derive(Clone)]
pub struct DurableEventSink {
    log: Arc<dyn DurableEventLog>,
}

impl DurableEventSink {
    pub fn new(log: Arc<dyn DurableEventLog>) -> Self {
        Self { log }
    }

    pub fn log(&self) -> Arc<dyn DurableEventLog> {
        Arc::clone(&self.log)
    }
}

impl std::fmt::Debug for DurableEventSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableEventSink")
            .field("log", &"<durable_event_log>")
            .finish()
    }
}

#[async_trait]
impl EventSink for DurableEventSink {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        self.log.append(event).await.map(|_| ())
    }
}

/// [`AuditSink`] adapter that appends each emitted audit envelope to a durable log.
#[derive(Clone)]
pub struct DurableAuditSink {
    log: Arc<dyn DurableAuditLog>,
}

impl DurableAuditSink {
    pub fn new(log: Arc<dyn DurableAuditLog>) -> Self {
        Self { log }
    }

    pub fn log(&self) -> Arc<dyn DurableAuditLog> {
        Arc::clone(&self.log)
    }
}

impl std::fmt::Debug for DurableAuditSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableAuditSink")
            .field("log", &"<durable_audit_log>")
            .finish()
    }
}

#[async_trait]
impl AuditSink for DurableAuditSink {
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError> {
        self.log.append(record).await.map(|_| ())
    }
}
