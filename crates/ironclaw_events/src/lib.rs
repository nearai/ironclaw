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
    AgentId, AuditEnvelope, CapabilityId, ExtensionId, MissionId, ProcessId, ProjectId,
    ResourceScope, RuntimeKind, TenantId, ThreadId, Timestamp, UserId,
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
/// `error_kind` is constrained by [`sanitize_error_kind`] on every wire
/// crossing:
///
/// - the typed `dispatch_failed` / `process_failed` constructors apply
///   sanitization at construction time;
/// - the custom [`Deserialize`] impl re-runs the sanitizer on any inbound
///   JSONL/wire payload;
/// - the custom [`Serialize`] impl re-runs the sanitizer before emitting the
///   wire payload, so an in-process caller that builds the struct directly
///   (`RuntimeEvent { error_kind: Some(raw), .. }`) still cannot smuggle raw
///   error text, paths, or token-shaped secrets through any
///   `serde_json::to_*` / durable-log `append` path.
///
/// The struct's fields remain `pub` for ergonomic in-memory inspection, but
/// the redaction invariant is enforced wherever the value crosses an I/O
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize)]
struct RuntimeEventWire {
    event_id: RuntimeEventId,
    timestamp: Timestamp,
    kind: RuntimeEventKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<RuntimeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_id: Option<ProcessId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
}

impl Serialize for RuntimeEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Re-run the redaction guard on the way out. This is the symmetric
        // partner to the Deserialize hook below; together they enforce that
        // `error_kind` is sanitized on every wire crossing regardless of
        // which constructor or direct field assignment produced the value.
        let wire = RuntimeEventWire {
            event_id: self.event_id,
            timestamp: self.timestamp,
            kind: self.kind,
            scope: self.scope.clone(),
            capability_id: self.capability_id.clone(),
            provider: self.provider.clone(),
            runtime: self.runtime,
            process_id: self.process_id,
            output_bytes: self.output_bytes,
            error_kind: self.error_kind.clone().map(sanitize_error_kind),
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RuntimeEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RuntimeEventWire::deserialize(deserializer)?;
        Ok(Self {
            event_id: wire.event_id,
            timestamp: wire.timestamp,
            kind: wire.kind,
            scope: wire.scope,
            capability_id: wire.capability_id,
            provider: wire.provider,
            runtime: wire.runtime,
            process_id: wire.process_id,
            output_bytes: wire.output_bytes,
            error_kind: wire.error_kind.map(sanitize_error_kind),
        })
    }
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

/// Stable token written to `RuntimeEvent.error_kind` whenever a caller-supplied
/// value fails redaction.
pub const UNCLASSIFIED_ERROR_KIND: &str = "Unclassified";

const MAX_ERROR_KIND_LEN: usize = 64;
const MAX_ERROR_KIND_SEGMENT_LEN: usize = 24;

/// Collapse any error_kind value that does not match the stable classification
/// shape into the single `Unclassified` token. This is the redaction guard
/// that keeps raw error messages, paths, and stringified secrets out of
/// durable runtime events.
///
/// Accepts only `lower_snake_case` identifiers with optional `.` or `:`
/// separators (e.g. `missing_runtime_backend`, `wasm.host_http_denied`,
/// `dispatch:timeout`). Rejects anything that resembles a path, free-form
/// error text, JWT, base64 token, or API key:
///
/// - empty string;
/// - longer than 64 bytes overall, or any dot/colon-separated segment longer
///   than 24 bytes (defeats long random tokens);
/// - characters outside `[a-z0-9_]` for body content, or `[._:]` separators;
/// - leading character that is not a lowercase ASCII letter (defeats
///   numeric-prefixed tokens, leading underscores, leading separators).
pub fn sanitize_error_kind(error_kind: impl Into<String>) -> String {
    let value = error_kind.into();
    if is_safe_error_kind(&value) {
        value
    } else {
        UNCLASSIFIED_ERROR_KIND.to_string()
    }
}

fn is_safe_error_kind(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ERROR_KIND_LEN {
        return false;
    }
    let first = value.as_bytes()[0];
    if !first.is_ascii_lowercase() {
        return false;
    }
    if value
        .bytes()
        .any(|byte| !is_error_kind_char(byte) && !matches!(byte, b'.' | b':'))
    {
        return false;
    }
    for segment in value.split(['.', ':']) {
        if segment.is_empty() || segment.len() > MAX_ERROR_KIND_SEGMENT_LEN {
            return false;
        }
        let segment_first = segment.as_bytes()[0];
        if !segment_first.is_ascii_lowercase() {
            return false;
        }
    }
    true
}

fn is_error_kind_char(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventStreamKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// Authorized read filter applied to durable replay.
///
/// `EventStreamKey` partitions cursors by `(tenant, user, agent)` per the
/// durable-log path contract. Within a single stream, multiple
/// projects/missions/threads/processes can co-exist; a project-scoped
/// consumer must still see only its own project's events. `ReadScope`
/// carries the deeper-scope dimensions and is enforced by the durable-log
/// implementation, not by the caller.
///
/// `ReadScope::any()` disables filtering and is intended for tests or
/// admin/aggregate paths that already hold authority for the whole stream.
/// Production callers must construct a tightened filter.
///
/// Filter semantics: a `Some(want)` field in the filter matches only
/// records whose corresponding scope field is `Some(want)`. A record with
/// `None` in that field does **not** match a filter that asks for
/// `Some(...)` — the filter is a tightening, never a permissive default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<MissionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<ProcessId>,
}

impl ReadScope {
    /// Filter that matches every record in the stream. Use only when the
    /// caller already holds authority for the whole stream (tests,
    /// admin/aggregate paths).
    pub fn any() -> Self {
        Self::default()
    }

    /// True iff every `Some` field in the filter has a matching value in the
    /// supplied [`ResourceScope`]. `process_id` is checked against the
    /// caller-supplied `process_id` because runtime events carry it on the
    /// record rather than inside the scope.
    pub fn matches_event(&self, event: &RuntimeEvent) -> bool {
        matches_optional(self.project_id.as_ref(), event.scope.project_id.as_ref())
            && matches_optional(self.mission_id.as_ref(), event.scope.mission_id.as_ref())
            && matches_optional(self.thread_id.as_ref(), event.scope.thread_id.as_ref())
            && matches_optional(self.process_id.as_ref(), event.process_id.as_ref())
    }

    /// True iff every `Some` field in the filter matches the corresponding
    /// top-level field on the audit envelope.
    pub fn matches_audit(&self, record: &AuditEnvelope) -> bool {
        matches_optional(self.project_id.as_ref(), record.project_id.as_ref())
            && matches_optional(self.mission_id.as_ref(), record.mission_id.as_ref())
            && matches_optional(self.thread_id.as_ref(), record.thread_id.as_ref())
            && matches_optional(self.process_id.as_ref(), record.process_id.as_ref())
    }
}

fn matches_optional<T: PartialEq>(want: Option<&T>, have: Option<&T>) -> bool {
    match want {
        None => true,
        Some(want) => have == Some(want),
    }
}

/// One replayed record and its durable cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry<T> {
    pub cursor: EventCursor,
    pub record: T,
}

/// Bounded replay response from a durable event/audit log.
///
/// `next_cursor` is suitable for the next `read_after_cursor` call. When
/// `entries` is empty, `next_cursor` echoes the requested cursor so the
/// consumer can resume cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReplay<T> {
    pub entries: Vec<EventLogEntry<T>>,
    pub next_cursor: EventCursor,
}

// -----------------------------------------------------------------------------
// Best-effort sink traits
// -----------------------------------------------------------------------------

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
    fn append(&mut self, record: T) -> Result<EventLogEntry<T>, EventError> {
        let next = self
            .next_cursor
            .checked_add(1)
            .ok_or_else(|| EventError::DurableLog {
                reason: "event cursor overflowed u64; durable log exhausted".to_string(),
            })?;
        self.next_cursor = next;
        let entry = EventLogEntry {
            cursor: EventCursor::new(next),
            record,
        };
        self.entries.push(entry.clone());
        Ok(entry)
    }

    fn read_after(
        &self,
        after: EventCursor,
        limit: usize,
        is_match: impl Fn(&T) -> bool,
    ) -> Result<EventReplay<T>, EventError> {
        // A cursor that points beyond the current head is a contract
        // violation, not a benign no-op: returning empty would silently lose
        // every event 1..=head once it lands. Surface as ReplayGap so the
        // caller is forced to request a snapshot/rebase and re-derive a
        // cursor that belongs to this stream.
        if after.as_u64() > self.next_cursor {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: EventCursor::new(self.next_cursor),
            });
        }
        if self.earliest_retained > 0 && after.as_u64() < self.earliest_retained.saturating_sub(1) {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: EventCursor::new(self.earliest_retained),
            });
        }
        // Walk every entry past the cursor; advance the scanned-cursor
        // marker even when a record is filtered out so the consumer's
        // resume cursor moves forward and they don't see filtered records
        // again on the next call.
        let mut entries = Vec::new();
        let mut last_scanned = after;
        for entry in &self.entries {
            if entry.cursor.as_u64() <= after.as_u64() {
                continue;
            }
            last_scanned = entry.cursor;
            if !is_match(&entry.record) {
                continue;
            }
            entries.push(entry.clone());
            if entries.len() >= limit {
                break;
            }
        }
        let next_cursor = entries
            .last()
            .map(|entry| entry.cursor)
            .unwrap_or(last_scanned);
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }

    /// Discard entries whose cursor is `<=` the supplied cursor and advance
    /// `earliest_retained` so subsequent reads with stale cursors return
    /// [`EventError::ReplayGap`]. Used by retention policies in production
    /// backends and by tests that exercise the gap path.
    ///
    /// Rejects cursors beyond the current stream head with
    /// [`EventError::InvalidReplayRequest`]. Without that guard a misuse
    /// (e.g. a calendar-time retention policy on a quiet stream) could push
    /// `earliest_retained` past `next_cursor` and brick the stream until
    /// enough appends caught up — every replay in the meantime would return
    /// a `ReplayGap` whose `earliest` value points at a cursor the stream
    /// has never issued.
    fn truncate_before_or_at(&mut self, cursor: EventCursor) -> Result<(), EventError> {
        let bound = cursor.as_u64();
        if bound == 0 {
            return Ok(());
        }
        if bound > self.next_cursor {
            return Err(EventError::InvalidReplayRequest {
                reason: format!(
                    "truncation cursor {bound} exceeds stream head {head}",
                    head = self.next_cursor,
                ),
            });
        }
        self.entries.retain(|entry| entry.cursor.as_u64() > bound);
        if bound >= self.earliest_retained {
            self.earliest_retained = bound + 1;
        }
        Ok(())
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

    /// Drop entries whose cursor is `<=` the supplied cursor for the given
    /// stream and advance the stream's earliest-retained marker so subsequent
    /// reads against older cursors return [`EventError::ReplayGap`].
    ///
    /// Returns [`EventError::InvalidReplayRequest`] when the supplied cursor
    /// exceeds the stream's current head; without that guard a misuse could
    /// permanently brick the stream.
    ///
    /// Production backends apply this from a retention policy. Tests use it
    /// to exercise the gap path without coupling to a specific policy.
    pub fn truncate_before_or_at(
        &self,
        stream: &EventStreamKey,
        cursor: EventCursor,
    ) -> Result<(), EventError> {
        let mut streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable event log lock poisoned".to_string(),
        })?;
        match streams.get_mut(stream) {
            Some(state) => state.truncate_before_or_at(cursor),
            None => Ok(()),
        }
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
        stream.append(event)
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
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
            Some(state) => state.read_after(after, limit, |event| filter.matches_event(event)),
            None => {
                // An absent stream is at head-zero. Any cursor beyond origin
                // is a foreign cursor that this stream has never issued, so
                // surface a gap rather than silently echoing the cursor and
                // hiding events 1..after if/when the stream starts.
                if after.as_u64() > 0 {
                    Err(EventError::ReplayGap {
                        requested: after,
                        earliest: EventCursor::origin(),
                    })
                } else {
                    Ok(EventReplay {
                        entries: Vec::new(),
                        next_cursor: after,
                    })
                }
            }
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

    /// See [`InMemoryDurableEventLog::truncate_before_or_at`].
    pub fn truncate_before_or_at(
        &self,
        stream: &EventStreamKey,
        cursor: EventCursor,
    ) -> Result<(), EventError> {
        let mut streams = self.streams.lock().map_err(|_| EventError::DurableLog {
            reason: "in-memory durable audit log lock poisoned".to_string(),
        })?;
        match streams.get_mut(stream) {
            Some(state) => state.truncate_before_or_at(cursor),
            None => Ok(()),
        }
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
        stream.append(record)
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
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
            Some(state) => state.read_after(after, limit, |record| filter.matches_audit(record)),
            None => {
                if after.as_u64() > 0 {
                    Err(EventError::ReplayGap {
                        requested: after,
                        earliest: EventCursor::origin(),
                    })
                } else {
                    Ok(EventReplay {
                        entries: Vec::new(),
                        next_cursor: after,
                    })
                }
            }
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
///
/// **Assumes uncompacted JSONL.** Backends that compact entries (drop old
/// records to reclaim disk) must not use this helper directly: line index
/// will desynchronize from the logical cursor and the helper will return
/// `ReplayGap` with a meaningless `earliest` value. Compacting backends
/// should either store the cursor inline in each record and use a different
/// parser, or maintain an out-of-band file-offset → cursor map.
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
    // A cursor beyond the JSONL head is a foreign or stale cursor; the
    // contract requires explicit ReplayGap signaling rather than silently
    // echoing it, mirroring InMemoryDurableEventLog. Without this guard a
    // future filesystem JSONL backend would accept cursors this stream
    // never issued and hide records once new lines are appended.
    if after > current_cursor {
        return Err(EventError::ReplayGap {
            requested: EventCursor::new(after),
            earliest: EventCursor::new(current_cursor),
        });
    }
    let next_cursor = entries
        .last()
        .map(|entry| entry.cursor)
        .unwrap_or_else(|| EventCursor::new(after));
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
