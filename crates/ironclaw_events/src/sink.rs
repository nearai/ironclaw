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

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError>;
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
