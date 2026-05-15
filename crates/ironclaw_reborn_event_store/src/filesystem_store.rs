//! Filesystem-backed durable event/audit log.
//!
//! `FilesystemDurableEventLog` and `FilesystemDurableAuditLog` route the
//! durable log through a [`ScopedFilesystem`]'s unified `append`/`tail`
//! plane instead of speaking SQL directly. This is the migration target
//! for the kernel-storage rework — once the per-backend `LibSqlStore` /
//! `PostgresStore` implementations in `libsql_store.rs` and
//! `postgres_store.rs` are removed (task #17), the only backend dispatch
//! left in this crate is whichever `RootFilesystem` got mounted under the
//! log's path.
//!
//! Path layout — one event log path per stream:
//!
//! ```text
//! /events/<kind>/<tenant>/<user>/<agent>
//! ```
//!
//! Where `<kind>` is `runtime` / `audit`, and `<agent>` falls back to
//! `_none` when the stream key carries no agent id. Path components come
//! straight from the validated `EventStreamKey` ids — they are already
//! constrained to a safe alphabet by `ironclaw_host_api`.
//!
//! Cursor semantics:
//!
//! - `append` returns a cursor whose `u64` is the underlying mount's
//!   monotonic `SeqNo` for that path.
//! - `read_after_cursor` calls `tail(path, SeqNo::from_backend(after))`
//!   and applies `ReadScope` filtering in Rust over the returned records.
//!   `next_cursor` advances past any trailing filtered records so a
//!   `(matched, filtered, filtered)` window resumes past the last
//!   filtered record on the next call rather than re-scanning them.
//! - If `after > 0` and `tail` returns empty, we surface
//!   [`EventError::ReplayGap`] from origin — same shape as the in-memory
//!   durable log, and the only sane behaviour given that the unified
//!   `tail` op cannot distinguish "after exceeds head" from "no records
//!   yet" without a separate head probe.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, ReadScope, RuntimeEvent,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{AuditEnvelope, ScopedPath};

use crate::{StreamKind, durable_error};

/// Filesystem-backed durable runtime event log.
pub struct FilesystemDurableEventLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    fs: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemDurableEventLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    pub fn new(fs: Arc<ScopedFilesystem<F>>) -> Self {
        Self { fs }
    }
}

impl<F> std::fmt::Debug for FilesystemDurableEventLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemDurableEventLog")
            .field("fs", &"<scoped_root_filesystem>")
            .finish()
    }
}

#[async_trait]
impl<F> DurableEventLog for FilesystemDurableEventLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        let stream = EventStreamKey::from_scope(&event.scope);
        let path = stream_path(StreamKind::Runtime, &stream)?;
        let payload = serde_json::to_vec(&event).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
        let seq = self
            .fs
            .append(&path, payload)
            .await
            .map_err(map_filesystem_append_error)?;
        Ok(EventLogEntry {
            cursor: EventCursor::new(seq.get()),
            record: event,
        })
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
        let path = stream_path(StreamKind::Runtime, stream)?;
        let records = self
            .fs
            .tail(&path, SeqNo::from_backend(after.as_u64()))
            .await
            .map_err(map_filesystem_tail_error)?;

        if records.is_empty() && after.as_u64() > 0 {
            // Tail returned nothing past `after`. Distinguish "consumer is
            // caught up to head" (after == head) from "consumer asked for
            // a foreign future cursor" (after > head) by probing the head.
            // `tail(path, 0)` is on the cold path here — only reached when
            // tail-after-cursor already came back empty — so the extra
            // round trip is acceptable.
            // PR #3679 review fix: bounded probe instead of `tail(0)` (which
            // loaded the entire log to read its last seq — O(N) per cold
            // call). `tail(after - 1)` returns events with seq > after - 1,
            // i.e. seq >= after. Combined with the prior empty
            // `tail(after)`, a non-empty probe means head == after exactly,
            // so the consumer is caught up. An empty probe means head <
            // after, i.e. a foreign-future cursor.
            let probe = self
                .fs
                .tail(&path, SeqNo::from_backend(after.as_u64().saturating_sub(1)))
                .await
                .map_err(map_filesystem_tail_error)?;
            if probe.is_empty() {
                return Err(EventError::ReplayGap {
                    requested: after,
                    earliest: EventCursor::origin(),
                });
            }
            return Ok(EventReplay {
                entries: Vec::new(),
                next_cursor: after,
            });
        }

        let mut entries = Vec::new();
        let mut last_scanned = after;
        for record in records {
            let event: RuntimeEvent =
                serde_json::from_slice(&record.payload).map_err(|error| EventError::Serialize {
                    reason: error.to_string(),
                })?;
            last_scanned = EventCursor::new(record.seq.get());
            if !filter.matches_event(&event) {
                continue;
            }
            entries.push(EventLogEntry {
                cursor: last_scanned,
                record: event,
            });
            if entries.len() >= limit {
                break;
            }
        }

        let next_cursor = match entries.last() {
            // If a trailing filtered record bumped `last_scanned` past the
            // last matched cursor, the consumer's resume cursor must
            // advance to `last_scanned` so the next replay does not
            // re-scan the filtered tail.
            Some(entry) if last_scanned.as_u64() > entry.cursor.as_u64() => last_scanned,
            Some(entry) => entry.cursor,
            None => last_scanned,
        };
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }
}

/// Filesystem-backed durable audit log.
pub struct FilesystemDurableAuditLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    fs: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemDurableAuditLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    pub fn new(fs: Arc<ScopedFilesystem<F>>) -> Self {
        Self { fs }
    }
}

impl<F> std::fmt::Debug for FilesystemDurableAuditLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemDurableAuditLog")
            .field("fs", &"<scoped_root_filesystem>")
            .finish()
    }
}

#[async_trait]
impl<F> DurableAuditLog for FilesystemDurableAuditLog<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        let stream = EventStreamKey::new(
            record.tenant_id.clone(),
            record.user_id.clone(),
            record.agent_id.clone(),
        );
        let path = stream_path(StreamKind::Audit, &stream)?;
        let payload = serde_json::to_vec(&record).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
        let seq = self
            .fs
            .append(&path, payload)
            .await
            .map_err(map_filesystem_append_error)?;
        Ok(EventLogEntry {
            cursor: EventCursor::new(seq.get()),
            record,
        })
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
        let path = stream_path(StreamKind::Audit, stream)?;
        let records = self
            .fs
            .tail(&path, SeqNo::from_backend(after.as_u64()))
            .await
            .map_err(map_filesystem_tail_error)?;

        if records.is_empty() && after.as_u64() > 0 {
            // Same head-probe pattern as the runtime log: distinguish
            // caught-up-to-head from foreign-future-cursor.
            // PR #3679 review fix: bounded probe instead of `tail(0)` (which
            // loaded the entire log to read its last seq — O(N) per cold
            // call). `tail(after - 1)` returns events with seq > after - 1,
            // i.e. seq >= after. Combined with the prior empty
            // `tail(after)`, a non-empty probe means head == after exactly,
            // so the consumer is caught up. An empty probe means head <
            // after, i.e. a foreign-future cursor.
            let probe = self
                .fs
                .tail(&path, SeqNo::from_backend(after.as_u64().saturating_sub(1)))
                .await
                .map_err(map_filesystem_tail_error)?;
            if probe.is_empty() {
                return Err(EventError::ReplayGap {
                    requested: after,
                    earliest: EventCursor::origin(),
                });
            }
            return Ok(EventReplay {
                entries: Vec::new(),
                next_cursor: after,
            });
        }

        let mut entries = Vec::new();
        let mut last_scanned = after;
        for record in records {
            let envelope: AuditEnvelope =
                serde_json::from_slice(&record.payload).map_err(|error| EventError::Serialize {
                    reason: error.to_string(),
                })?;
            last_scanned = EventCursor::new(record.seq.get());
            if !filter.matches_audit(&envelope) {
                continue;
            }
            entries.push(EventLogEntry {
                cursor: last_scanned,
                record: envelope,
            });
            if entries.len() >= limit {
                break;
            }
        }

        let next_cursor = match entries.last() {
            // If a trailing filtered record bumped `last_scanned` past the
            // last matched cursor, the consumer's resume cursor must
            // advance to `last_scanned` so the next replay does not
            // re-scan the filtered tail.
            Some(entry) if last_scanned.as_u64() > entry.cursor.as_u64() => last_scanned,
            Some(entry) => entry.cursor,
            None => last_scanned,
        };
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }
}

fn stream_path(kind: StreamKind, stream: &EventStreamKey) -> Result<ScopedPath, EventError> {
    let kind_segment = match kind {
        StreamKind::Runtime => "runtime",
        StreamKind::Audit => "audit",
    };
    let agent_segment = stream
        .agent_id
        .as_ref()
        .map(|id| id.as_str())
        .unwrap_or("_none");
    let raw = format!(
        "/events/{kind}/{tenant}/{user}/{agent}",
        kind = kind_segment,
        tenant = stream.tenant_id.as_str(),
        user = stream.user_id.as_str(),
        agent = agent_segment,
    );
    ScopedPath::new(raw).map_err(|_| durable_error("filesystem event store stream path is invalid"))
}

fn map_filesystem_append_error(error: FilesystemError) -> EventError {
    // Don't leak backend details into the durable error surface; the
    // crate-level error policy keeps backend specifics behind a redacted
    // boundary. Categorise the few cases callers can act on, then fall
    // back to the generic DurableLog error.
    match error {
        FilesystemError::PermissionDenied { .. } => {
            durable_error("filesystem event store rejected append: permission denied")
        }
        FilesystemError::MountNotFound { .. } => {
            durable_error("filesystem event store has no mount for the stream path")
        }
        FilesystemError::Unsupported { .. } => {
            durable_error("filesystem event store mount does not advertise the events plane")
        }
        _ => durable_error("filesystem event store failed to append record"),
    }
}

fn map_filesystem_tail_error(error: FilesystemError) -> EventError {
    match error {
        FilesystemError::PermissionDenied { .. } => {
            durable_error("filesystem event store rejected tail: permission denied")
        }
        FilesystemError::MountNotFound { .. } => {
            durable_error("filesystem event store has no mount for the stream path")
        }
        FilesystemError::Unsupported { .. } => {
            durable_error("filesystem event store mount does not advertise the events plane")
        }
        _ => durable_error("filesystem event store failed to read stream"),
    }
}
