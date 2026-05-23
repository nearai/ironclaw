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
//! - In-memory backends are provided for tests and reference loops. Production
//!   backend selection and durable adapters live outside this substrate crate
//!   in `ironclaw_reborn_event_store`, which depends on these traits rather
//!   than pulling storage drivers back into `ironclaw_events`.
//!
//! # Redaction invariants
//!
//! Events and audit envelopes must not leak raw secrets, raw host paths,
//! private auth tokens, raw request/response payloads, approval reasons,
//! invocation fingerprints, lease IDs, or lease contents. Runtime
//! `error_kind` strings are constrained to short classification tokens; any
//! unsafe value is collapsed to `Unclassified`.
#![warn(unreachable_pub)]

mod cursor;
mod error;
mod in_memory;
mod jsonl;
mod runtime_event;
mod security_audit;
mod sink;

pub use cursor::{EventCursor, EventLogEntry, EventReplay, EventStreamKey, ReadScope};
pub use error::EventError;
pub use in_memory::{
    InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink,
};
pub use jsonl::{parse_jsonl, replay_jsonl};
pub use runtime_event::{
    RuntimeEvent, RuntimeEventId, RuntimeEventKind, UNCLASSIFIED_ERROR_KIND, sanitize_error_kind,
};
pub use security_audit::{
    InMemorySecurityAuditSink, NoopSecurityAuditSink, SecurityAuditEvent, SecurityAuditSink,
    SecurityBoundary, SecurityDecision, TracingSecurityAuditSink,
};
pub use sink::{
    AuditSink, DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventSink,
};
