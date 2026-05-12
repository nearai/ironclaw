use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::AuditEnvelope;

use crate::cursor::{EventCursor, EventLogEntry, EventReplay, EventStreamKey, ReadScope};
use crate::error::EventError;
use crate::runtime_event::RuntimeEvent;
use crate::sink::{AuditSink, DurableAuditLog, DurableEventLog, EventSink};

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
        let start_index = self
            .entries
            .partition_point(|entry| entry.cursor.as_u64() <= after.as_u64());
        let mut entries = Vec::new();
        let mut last_scanned = after;
        for entry in &self.entries[start_index..] {
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

fn lock_or_recover<T>(mutex: &Arc<Mutex<T>>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
