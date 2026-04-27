//! Runtime event, audit envelope, and durable append-log substrate for
//! IronClaw Reborn.
//!
//! `ironclaw_events` defines the small redacted vocabulary every
//! Reborn system-service crate uses to record observable runtime/process
//! transitions and control-plane audit, plus the durable append-log substrate
//! the host runtime, dispatcher, process manager, and approval resolver use
//! to expose replayable scoped streams.
//!
//! # Layering
//!
//! - [`RuntimeEvent`] / [`RuntimeEventKind`] are the metadata-only event
//!   shapes. Constructors collapse unsafe error detail into `Unclassified`.
//! - [`EventSink`] / [`AuditSink`] are best-effort delivery traits. Failures
//!   are recorded but must not alter runtime or control-plane outcomes.
//! - [`DurableEventLog`] / [`DurableAuditLog`] are explicit-error append-log
//!   traits with a monotonic per-stream [`EventCursor`] and replay-after
//!   semantics. Append failures are propagated; replay against a cursor older
//!   than the earliest retained entry returns [`EventError::ReplayGap`] so
//!   transports can request a snapshot/rebase rather than silently lose data.
//! - In-memory backends are provided for tests and reference loops.
//!   Filesystem-backed JSONL backends and PostgreSQL/libSQL backends are
//!   deliberately deferred to later grouped Reborn PRs that depend on
//!   `ironclaw_filesystem` and the database substrates. The byte-level
//!   [`parse_jsonl`] and [`replay_jsonl`] helpers are exposed so those later
//!   backends can build on the same redaction and replay invariants.
//!
//! # Redaction invariants
//!
//! Events and audit envelopes must not leak raw secrets, raw host paths,
//! private auth tokens, raw request/response payloads, approval reasons,
//! invocation fingerprints, lease IDs, or lease contents. Runtime
//! `error_kind` strings are constrained to short classification tokens; any
//! unsafe value is collapsed to `Unclassified`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    AgentId, AuditEnvelope, CapabilityId, ExtensionId, ProcessId, ResourceScope, RuntimeKind,
    TenantId, Timestamp, UserId,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use uuid::Uuid;

// -----------------------------------------------------------------------------
// Runtime event vocabulary
// -----------------------------------------------------------------------------

/// Runtime event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeEventId(Uuid);

impl RuntimeEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for RuntimeEventId {
    fn default() -> Self {
        Self::new()
    }
}

/// Event kinds emitted by the composition/runtime path.
///
/// Approval-specific event kinds are deliberately absent. Approval resolution
/// is a control-plane concern and is recorded as
/// [`AuditEnvelope`] with `AuditStage::ApprovalResolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
}

/// Redacted runtime event payload.
///
/// All optional fields are absent unless meaningful for the event kind.
/// `error_kind` is constrained by [`sanitize_error_kind`] so detail-like
/// values (raw error text, paths, secrets) cannot leak through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: RuntimeEventKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
}

impl RuntimeEvent {
    pub fn dispatch_requested(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchRequested,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn runtime_selected(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::RuntimeSelected,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn dispatch_succeeded(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchSucceeded,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: Some(output_bytes),
            error_kind: None,
        })
    }

    pub fn dispatch_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchFailed,
            scope,
            capability_id,
            provider,
            runtime,
            process_id: None,
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn process_started(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessStarted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_completed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessCompleted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessFailed,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn process_killed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessKilled,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    fn new(payload: RuntimeEventPayload) -> Self {
        Self {
            event_id: RuntimeEventId::new(),
            timestamp: Utc::now(),
            kind: payload.kind,
            scope: payload.scope,
            capability_id: payload.capability_id,
            provider: payload.provider,
            runtime: payload.runtime,
            process_id: payload.process_id,
            output_bytes: payload.output_bytes,
            error_kind: payload.error_kind,
        }
    }
}

struct RuntimeEventPayload {
    kind: RuntimeEventKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    provider: Option<ExtensionId>,
    runtime: Option<RuntimeKind>,
    process_id: Option<ProcessId>,
    output_bytes: Option<u64>,
    error_kind: Option<String>,
}

/// Collapse any error_kind value that does not match the stable classification
/// shape into the single `Unclassified` token. This is the redaction guard
/// that keeps raw error messages, paths, and stringified secrets out of
/// durable runtime events.
pub fn sanitize_error_kind(error_kind: impl Into<String>) -> String {
    let error_kind = error_kind.into();
    let is_safe = !error_kind.is_empty()
        && error_kind.len() <= 128
        && error_kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));
    if is_safe {
        error_kind
    } else {
        "Unclassified".to_string()
    }
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

/// Event sink and durable-log error variants.
#[derive(Debug, Error)]
pub enum EventError {
    #[error("event serialization failed: {reason}")]
    Serialize { reason: String },
    #[error("event sink failed: {reason}")]
    Sink { reason: String },
    #[error("durable event log failed: {reason}")]
    DurableLog { reason: String },
    #[error(
        "replay gap: requested cursor {requested:?} predates earliest retained cursor {earliest:?}; consumer must request a scoped snapshot/rebase"
    )]
    ReplayGap {
        requested: EventCursor,
        earliest: EventCursor,
    },
    #[error("replay request rejected: {reason}")]
    InvalidReplayRequest { reason: String },
}

// -----------------------------------------------------------------------------
// Cursor envelope
// -----------------------------------------------------------------------------

/// Monotonic replay cursor for a scoped durable log.
///
/// Cursors are not global authority. They must be validated against the
/// requesting consumer's [`EventStreamKey`] before any replay is served. A
/// cursor older than the earliest retained record yields
/// [`EventError::ReplayGap`] so transports can fetch a snapshot/rebase rather
/// than silently lose history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventCursor(u64);

impl EventCursor {
    /// The cursor that precedes every record. `read_after_cursor(.., None, ..)`
    /// is equivalent to `read_after_cursor(.., Some(EventCursor::origin()), ..)`.
    pub const fn origin() -> Self {
        Self(0)
    }

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl Default for EventCursor {
    fn default() -> Self {
        Self::origin()
    }
}

/// Stream partition key.
///
/// Reborn durable event/audit streams partition by (tenant, user, agent).
/// Cursors are monotonic within a stream and must be validated against the
/// requesting consumer's stream key. Deeper scope filtering (project,
/// mission, thread, process, invocation) is applied as a read-side filter on
/// the matching stream rather than as a separate cursor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventStreamKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
}

impl EventStreamKey {
    pub fn new(tenant_id: TenantId, user_id: UserId, agent_id: Option<AgentId>) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
        }
    }

    pub fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
        }
    }

    pub fn matches(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
    }
}

/// One replayed record and its durable cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLogEntry<T> {
    pub cursor: EventCursor,
    pub record: T,
}

/// Bounded replay response from a durable event/audit log.
///
/// `next_cursor` is suitable for the next `read_after_cursor` call. When
/// `entries` is empty, `next_cursor` echoes the requested cursor so the
/// consumer can resume cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReplay<T> {
    pub entries: Vec<EventLogEntry<T>>,
    pub next_cursor: EventCursor,
}

// -----------------------------------------------------------------------------
// Best-effort sink traits
// -----------------------------------------------------------------------------

/// Async event sink used by runtime/composition services.
///
/// Best-effort observability. Sink failures are surfaced to the caller, which
/// is expected to log/ignore them rather than alter runtime outcomes; see
/// `events.md` §7.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError>;
}

/// Async audit sink used by control-plane services.
///
/// Best-effort observability. Sink failures must not change approval
/// resolution outcomes; see `events.md` §3.
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
/// stream key authority: callers must validate that the requested
/// [`EventStreamKey`] matches the consumer's authorized scope before serving
/// the result, and the implementation rejects cursors that predate the
/// earliest retained entry with [`EventError::ReplayGap`].
#[async_trait]
pub trait DurableEventLog: Send + Sync {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError>;

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
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
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError>;
}

// -----------------------------------------------------------------------------
// In-memory best-effort sinks
// -----------------------------------------------------------------------------

/// In-memory event sink used by tests and live demos.
#[derive(Debug, Clone, Default)]
pub struct InMemoryEventSink {
    events: Arc<Mutex<Vec<RuntimeEvent>>>,
}

impl InMemoryEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<RuntimeEvent> {
        lock_or_recover(&self.events).clone()
    }
}

#[async_trait]
impl EventSink for InMemoryEventSink {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        lock_or_recover(&self.events).push(event);
        Ok(())
    }
}

/// In-memory audit sink used by tests and live demos.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAuditSink {
    records: Arc<Mutex<Vec<AuditEnvelope>>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<AuditEnvelope> {
        lock_or_recover(&self.records).clone()
    }
}

#[async_trait]
impl AuditSink for InMemoryAuditSink {
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError> {
        lock_or_recover(&self.records).push(record);
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// In-memory durable backends
// -----------------------------------------------------------------------------

#[derive(Debug)]
struct StreamState<T> {
    next_cursor: u64,
    earliest_retained: u64,
    entries: Vec<EventLogEntry<T>>,
}

impl<T> Default for StreamState<T> {
    fn default() -> Self {
        Self {
            next_cursor: 0,
            earliest_retained: 0,
            entries: Vec::new(),
        }
    }
}

impl<T: Clone> StreamState<T> {
    fn append(&mut self, record: T) -> EventLogEntry<T> {
        self.next_cursor += 1;
        let entry = EventLogEntry {
            cursor: EventCursor::new(self.next_cursor),
            record,
        };
        self.entries.push(entry.clone());
        entry
    }

    fn read_after(&self, after: EventCursor, limit: usize) -> Result<EventReplay<T>, EventError> {
        if self.earliest_retained > 0 && after.as_u64() < self.earliest_retained.saturating_sub(1) {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: EventCursor::new(self.earliest_retained),
            });
        }
        let mut entries = Vec::new();
        for entry in &self.entries {
            if entry.cursor.as_u64() <= after.as_u64() {
                continue;
            }
            if entries.len() >= limit {
                break;
            }
            entries.push(entry.clone());
        }
        let next_cursor = entries.last().map(|entry| entry.cursor).unwrap_or(after);
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }
}

/// In-memory durable runtime event log with per-stream monotonic cursors.
#[derive(Debug, Default)]
pub struct InMemoryDurableEventLog {
    streams: Mutex<HashMap<EventStreamKey, StreamState<RuntimeEvent>>>,
}

impl InMemoryDurableEventLog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DurableEventLog for InMemoryDurableEventLog {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        let key = EventStreamKey::from_scope(&event.scope);
        let mut streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable event log lock poisoned".to_string(),
        })?;
        let stream = streams.entry(key).or_default();
        Ok(stream.append(event))
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        if limit == 0 {
            return Err(EventError::InvalidReplayRequest {
                reason: "limit must be greater than zero".to_string(),
            });
        }
        let after = after.unwrap_or_default();
        let streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable event log lock poisoned".to_string(),
        })?;
        match streams.get(stream) {
            Some(state) => state.read_after(after, limit),
            None => Ok(EventReplay {
                entries: Vec::new(),
                next_cursor: after,
            }),
        }
    }
}

/// In-memory durable audit log with per-stream monotonic cursors.
#[derive(Debug, Default)]
pub struct InMemoryDurableAuditLog {
    streams: Mutex<HashMap<EventStreamKey, StreamState<AuditEnvelope>>>,
}

impl InMemoryDurableAuditLog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DurableAuditLog for InMemoryDurableAuditLog {
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        let key = EventStreamKey::new(
            record.tenant_id.clone(),
            record.user_id.clone(),
            record.agent_id.clone(),
        );
        let mut streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable audit log lock poisoned".to_string(),
        })?;
        let stream = streams.entry(key).or_default();
        Ok(stream.append(record))
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        if limit == 0 {
            return Err(EventError::InvalidReplayRequest {
                reason: "limit must be greater than zero".to_string(),
            });
        }
        let after = after.unwrap_or_default();
        let streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable audit log lock poisoned".to_string(),
        })?;
        match streams.get(stream) {
            Some(state) => state.read_after(after, limit),
            None => Ok(EventReplay {
                entries: Vec::new(),
                next_cursor: after,
            }),
        }
    }
}

// -----------------------------------------------------------------------------
// JSONL byte-level helpers (exposed for downstream filesystem-backed sinks)
// -----------------------------------------------------------------------------

/// Parse a JSONL byte slice into a vector of typed records.
///
/// Backend, mount, permission, UTF-8, or malformed JSONL failures are
/// returned as errors; the helper does not silently elide invalid lines.
/// See `events.md` §5.
pub fn parse_jsonl<T>(bytes: &[u8]) -> Result<Vec<T>, EventError>
where
    T: DeserializeOwned,
{
    let text = std::str::from_utf8(bytes).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<T>(line).map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })
        })
        .collect()
}

/// Replay a JSONL byte slice after a cursor with a bounded limit.
///
/// Used by JSONL-backed durable log adapters in later grouped Reborn PRs.
/// The cursor is the 1-based line index of the last consumed record.
pub fn replay_jsonl<T>(
    bytes: &[u8],
    after: Option<EventCursor>,
    limit: usize,
) -> Result<EventReplay<T>, EventError>
where
    T: DeserializeOwned,
{
    if limit == 0 {
        return Err(EventError::InvalidReplayRequest {
            reason: "limit must be greater than zero".to_string(),
        });
    }
    let after = after.unwrap_or_default().as_u64();
    let text = std::str::from_utf8(bytes).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    let mut entries = Vec::new();
    let mut current_cursor = 0u64;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        current_cursor += 1;
        let record = serde_json::from_str::<T>(line).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
        if current_cursor > after && entries.len() < limit {
            entries.push(EventLogEntry {
                cursor: EventCursor::new(current_cursor),
                record,
            });
        }
    }
    let next_cursor = entries
        .last()
        .map(|entry| entry.cursor)
        .unwrap_or_else(|| EventCursor::new(after.max(current_cursor)));
    Ok(EventReplay {
        entries,
        next_cursor,
    })
}

// -----------------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------------

fn lock_or_recover<T>(mutex: &Arc<Mutex<T>>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
