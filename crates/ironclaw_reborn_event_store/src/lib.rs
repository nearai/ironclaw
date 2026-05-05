//! Reborn-owned durable event and audit store backends.
//!
//! This crate is the production-composition side of the Reborn event
//! substrate. `ironclaw_events` owns the durable log traits and redacted record
//! vocabulary; this crate owns backend selection, fail-closed profile
//! validation, and concrete storage adapters that should not live in the
//! substrate crate.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, InMemoryDurableAuditLog, InMemoryDurableEventLog, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::{AgentId, AuditEnvelope};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::sync::Mutex;

#[cfg(feature = "libsql")]
mod libsql_store;
#[cfg(feature = "postgres")]
mod postgres_store;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod sql_common;

/// Backend configuration for Reborn durable event/audit stores.
#[derive(Debug)]
pub enum RebornEventStoreConfig {
    /// In-memory reference backend. Valid only for explicit local/test
    /// profiles; production rejects it before returning a service graph.
    InMemory,
    /// Single-node durable JSONL backend rooted outside V1 migrations and DB
    /// traits. Production must explicitly accept this single-node durability
    /// mode so it cannot become an implicit memory-style fallback.
    Jsonl {
        root: PathBuf,
        accept_single_node_durable: bool,
    },
    /// PostgreSQL backend configuration. Schema files and the concrete adapter
    /// are owned by this crate rather than V1 DB/AppBuilder paths.
    Postgres { url: SecretString },
    /// libSQL backend configuration. Local paths and remote libSQL URLs are
    /// opened directly by this crate rather than V1 DB/AppBuilder paths.
    Libsql {
        path_or_url: String,
        auth_token: Option<SecretString>,
    },
}

/// Reborn composition profile controlling which fallbacks are legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornProfile {
    LocalDev,
    Test,
    Production,
}

/// Durable event and audit log handles consumed by Reborn composition.
#[derive(Clone)]
pub struct RebornEventStores {
    pub events: Arc<dyn DurableEventLog>,
    pub audit: Arc<dyn DurableAuditLog>,
}

/// Redacted factory/configuration errors.
#[derive(Debug, Error)]
pub enum RebornEventStoreError {
    #[error("production Reborn event store cannot use in-memory storage")]
    ProductionInMemoryDisabled,
    #[error("production JSONL event store requires explicit single-node durable acceptance")]
    ProductionJsonlRequiresAcceptance,
    #[error("production Reborn event store cannot use cleartext http:// libSQL URL")]
    ProductionLibsqlClearTextDisabled,
    #[error(
        "remote Reborn Postgres event store requires sslmode=require (sslmode=disable rejected)"
    )]
    RemotePostgresClearTextDisabled,
    #[error("{backend} Reborn event store backend is not enabled in this build")]
    BackendUnavailable { backend: &'static str },
    #[error("{backend} Reborn event store failed during {operation}")]
    BackendOperation {
        backend: &'static str,
        operation: &'static str,
    },
    #[error("Reborn event store I/O failed during {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl RebornEventStoreError {
    fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn backend<E>(backend: &'static str, operation: &'static str, _source: E) -> Self {
        Self::BackendOperation { backend, operation }
    }
}

/// Build durable event and audit logs for a standalone Reborn composition path.
pub async fn build_reborn_event_stores(
    profile: RebornProfile,
    config: RebornEventStoreConfig,
) -> Result<RebornEventStores, RebornEventStoreError> {
    match config {
        RebornEventStoreConfig::InMemory => {
            if profile == RebornProfile::Production {
                return Err(RebornEventStoreError::ProductionInMemoryDisabled);
            }
            Ok(RebornEventStores {
                events: Arc::new(InMemoryDurableEventLog::new()),
                audit: Arc::new(InMemoryDurableAuditLog::new()),
            })
        }
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable,
        } => {
            if profile == RebornProfile::Production && !accept_single_node_durable {
                return Err(RebornEventStoreError::ProductionJsonlRequiresAcceptance);
            }
            tokio::fs::create_dir_all(&root)
                .await
                .map_err(|source| RebornEventStoreError::io("initialize jsonl root", source))?;
            let store = JsonlStore::new(root);
            Ok(RebornEventStores {
                events: Arc::new(JsonlDurableEventLog::from_store(store.clone())),
                audit: Arc::new(JsonlDurableAuditLog::from_store(store)),
            })
        }
        RebornEventStoreConfig::Postgres { url } => {
            #[cfg(feature = "postgres")]
            {
                postgres_store::build_postgres_event_stores(url).await
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = url;
                Err(RebornEventStoreError::BackendUnavailable {
                    backend: "postgres",
                })
            }
        }
        RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        } => {
            if profile == RebornProfile::Production && path_or_url.starts_with("http://") {
                return Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled);
            }
            #[cfg(feature = "libsql")]
            {
                libsql_store::build_libsql_event_stores(path_or_url, auth_token).await
            }
            #[cfg(not(feature = "libsql"))]
            {
                let _ = (path_or_url, auth_token);
                Err(RebornEventStoreError::BackendUnavailable { backend: "libsql" })
            }
        }
    }
}

/// JSONL-backed durable runtime event log.
#[derive(Clone)]
pub struct JsonlDurableEventLog {
    store: JsonlStore,
}

impl JsonlDurableEventLog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            store: JsonlStore::new(root.into()),
        }
    }

    fn from_store(store: JsonlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for JsonlDurableEventLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JsonlDurableEventLog")
            .field("root", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl DurableEventLog for JsonlDurableEventLog {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        let stream = EventStreamKey::from_scope(&event.scope);
        self.store.append(StreamKind::Runtime, &stream, event).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        let owned_filter = filter.clone();
        self.store
            .read_after(
                StreamKind::Runtime,
                stream,
                filter,
                after,
                limit,
                move |event| owned_filter.matches_event(event),
            )
            .await
    }
}

/// JSONL-backed durable audit log.
#[derive(Clone)]
pub struct JsonlDurableAuditLog {
    store: JsonlStore,
}

impl JsonlDurableAuditLog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            store: JsonlStore::new(root.into()),
        }
    }

    fn from_store(store: JsonlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for JsonlDurableAuditLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JsonlDurableAuditLog")
            .field("root", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl DurableAuditLog for JsonlDurableAuditLog {
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        let stream = EventStreamKey::new(
            record.tenant_id.clone(),
            record.user_id.clone(),
            record.agent_id.clone(),
        );
        self.store.append(StreamKind::Audit, &stream, record).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        let owned_filter = filter.clone();
        self.store
            .read_after(
                StreamKind::Audit,
                stream,
                filter,
                after,
                limit,
                move |record| owned_filter.matches_audit(record),
            )
            .await
    }
}

#[derive(Debug, Clone)]
struct JsonlStore {
    root: PathBuf,
    locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
}

impl JsonlStore {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn append<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        record: T,
    ) -> Result<EventLogEntry<T>, EventError>
    where
        T: Clone + Serialize + DeserializeOwned + Send + 'static,
    {
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let path = self.stream_path(kind, stream);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| durable_error("jsonl event store failed to prepare stream"))?;
        }
        // Serialise the record outside the blocking section.
        let record_for_envelope = record.clone();
        let assigned_cursor = tokio::task::spawn_blocking(move || -> Result<u64, EventError> {
            // The OS-level exclusive lock spans both reading the prior tail
            // cursor and appending the new record so two processes cannot
            // both observe the same last cursor and emit duplicates.
            append_with_cursor_assignment(&path, |next_cursor| {
                let envelope = JsonlEntry {
                    cursor: EventCursor::new(next_cursor),
                    record: record_for_envelope,
                };
                serde_json::to_string(&envelope).map_err(|error| EventError::Serialize {
                    reason: error.to_string(),
                })
            })
        })
        .await
        .map_err(|_| durable_error("jsonl event store failed to append record"))??;
        Ok(EventLogEntry {
            cursor: EventCursor::new(assigned_cursor),
            record,
        })
    }

    async fn read_after<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        _filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
        is_match: impl Fn(&T) -> bool + Send + 'static,
    ) -> Result<EventReplay<T>, EventError>
    where
        T: Clone + DeserializeOwned + Send + 'static,
    {
        if limit == 0 {
            return Err(EventError::InvalidReplayRequest {
                reason: "limit must be greater than zero".to_string(),
            });
        }
        let after = after.unwrap_or_default();
        // We hold the in-process stream lock while we *read* purely so that
        // a concurrent in-process append cannot interleave a partial line
        // mid-read. Cross-process safety is provided by the OS-level
        // exclusive file lock taken by `append_envelope`; readers do not
        // need the OS lock.
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let path = self.stream_path(kind, stream);
        tokio::task::spawn_blocking(move || {
            stream_read_after::<T, _>(&path, after, limit, is_match)
        })
        .await
        .map_err(|_| durable_error("jsonl event store failed to read stream"))?
    }

    async fn stream_lock(&self, kind: StreamKind, stream: &EventStreamKey) -> Arc<Mutex<()>> {
        let key = stream_lock_key(kind, stream);
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn stream_path(&self, kind: StreamKind, stream: &EventStreamKey) -> PathBuf {
        let mut path = self
            .root
            .join(kind.directory())
            .join(component("tenant", stream.tenant_id.as_str()))
            .join(component("user", stream.user_id.as_str()));
        path.push(format!(
            "{}.jsonl",
            agent_component(stream.agent_id.as_ref())
        ));
        path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StreamKind {
    Runtime,
    Audit,
}

impl StreamKind {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Audit => "audit",
        }
    }

    fn directory(self) -> &'static Path {
        match self {
            Self::Runtime => Path::new("events"),
            Self::Audit => Path::new("audit"),
        }
    }

    fn lock_prefix(self) -> &'static str {
        match self {
            Self::Runtime => "events",
            Self::Audit => "audit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonlEntry<T> {
    cursor: EventCursor,
    record: T,
}

#[derive(Debug, Deserialize)]
struct JsonlCursor {
    cursor: EventCursor,
}

fn read_last_jsonl_cursor(path: &Path) -> Result<Option<u64>, EventError> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(durable_error("jsonl event store failed to read stream")),
    };
    let mut position = file
        .metadata()
        .map_err(|_| durable_error("jsonl event store failed to read stream"))?
        .len();
    if position == 0 {
        return Ok(None);
    }

    const CHUNK_SIZE: u64 = 8192;
    let mut reversed_line = Vec::new();
    let mut saw_non_newline = false;
    while position > 0 {
        let read_len = position.min(CHUNK_SIZE) as usize;
        position -= read_len as u64;
        file.seek(SeekFrom::Start(position))
            .map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        let mut chunk = vec![0; read_len];
        file.read_exact(&mut chunk)
            .map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        for byte in chunk.into_iter().rev() {
            if byte == b'\n' || byte == b'\r' {
                if saw_non_newline {
                    reversed_line.reverse();
                    return parse_jsonl_cursor(&reversed_line);
                }
                continue;
            }
            saw_non_newline = true;
            reversed_line.push(byte);
        }
    }

    if !saw_non_newline {
        return Ok(None);
    }
    reversed_line.reverse();
    parse_jsonl_cursor(&reversed_line)
}

fn parse_jsonl_cursor(line: &[u8]) -> Result<Option<u64>, EventError> {
    let envelope =
        serde_json::from_slice::<JsonlCursor>(line).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
    Ok(Some(envelope.cursor.as_u64()))
}

/// Stream a JSONL stream line-by-line, applying the cursor `after` filter,
/// the predicate, and the `limit`. Stops as soon as `limit` matches are
/// collected, so a `limit = 1` request on a multi-gigabyte JSONL never reads
/// or parses the whole file.
fn stream_read_after<T, F>(
    path: &Path,
    after: EventCursor,
    limit: usize,
    is_match: F,
) -> Result<EventReplay<T>, EventError>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    use std::io::{BufRead, BufReader};

    let file = match std::fs::File::open(path) {
        Ok(file) => {
            // Take a shared advisory lock so we never observe a partially
            // written line from a concurrent appender in another process.
            file.lock_shared()
                .map_err(|_| durable_error("jsonl event store failed to acquire read lock"))?;
            file
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // No file means no entries yet; treat the head as the origin.
            if after.as_u64() > 0 {
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
        Err(_) => return Err(durable_error("jsonl event store failed to read stream")),
    };
    let reader = BufReader::new(file);

    let mut replay_entries = Vec::new();
    let mut last_scanned = after;
    let mut head_cursor = EventCursor::origin();
    let mut expected_cursor = 1u64;
    let mut after_validated = after.as_u64() == 0;

    for line in reader.lines() {
        let line = line.map_err(|_| durable_error("jsonl event store failed to read stream"))?;
        if line.trim().is_empty() {
            continue;
        }
        // Decode just the cursor first to validate sequencing cheaply and to
        // skip records we do not need to materialise into `T`.
        let envelope_cursor = serde_json::from_str::<JsonlCursor>(&line)
            .map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })?
            .cursor;
        if envelope_cursor.as_u64() != expected_cursor {
            return Err(durable_error(
                "jsonl event stream cursor sequence is invalid",
            ));
        }
        head_cursor = envelope_cursor;
        expected_cursor = expected_cursor
            .checked_add(1)
            .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
        if envelope_cursor.as_u64() <= after.as_u64() {
            // We have proven the stream contains at least one record at or
            // beyond `after`, so a future-cursor `ReplayGap` cannot apply.
            if envelope_cursor.as_u64() == after.as_u64() {
                after_validated = true;
            }
            continue;
        }
        // Crossing past `after` also validates it, since the head is now
        // strictly greater than `after`.
        after_validated = true;
        last_scanned = envelope_cursor;
        let envelope = serde_json::from_str::<JsonlEntry<T>>(&line).map_err(|error| {
            EventError::Serialize {
                reason: error.to_string(),
            }
        })?;
        if !is_match(&envelope.record) {
            continue;
        }
        replay_entries.push(EventLogEntry {
            cursor: envelope.cursor,
            record: envelope.record,
        });
        if replay_entries.len() >= limit {
            // Stop streaming as soon as we have `limit` matches so that a
            // small `limit` against a large file does not pay full-stream
            // parse latency. `next_cursor` correctly equals the last match
            // here; the caller can detect any future-cursor gap on the
            // subsequent call.
            break;
        }
    }

    if !after_validated && after.as_u64() > head_cursor.as_u64() {
        return Err(EventError::ReplayGap {
            requested: after,
            earliest: head_cursor,
        });
    }

    let last_matched = replay_entries.last().map(|entry| entry.cursor);
    let next_cursor = match last_matched {
        Some(matched) if matched.as_u64() >= last_scanned.as_u64() => matched,
        Some(_) => last_scanned,
        None => last_scanned,
    };
    Ok(EventReplay {
        entries: replay_entries,
        next_cursor,
    })
}

/// Acquire an OS-level exclusive advisory lock on `path` (creating the file
/// if needed), determine the current tail cursor by reading the file's last
/// JSONL line under the lock, then invoke `serialise` to produce the next
/// envelope's serialised line and append + fsync it. Releases the lock when
/// the function returns. Cross-process safe: two IronClaw processes that race
/// to append against the same file will block on this lock and emit
/// monotonically-sequenced cursors.
fn append_with_cursor_assignment<F>(path: &Path, serialise: F) -> Result<u64, EventError>
where
    F: FnOnce(u64) -> Result<String, EventError>,
{
    use std::io::Write;

    // Track whether we're about to create the file so we know to fsync the
    // parent directory afterwards. On POSIX, `sync_data()` on the file
    // contents is not enough for crash durability — the parent directory
    // entry that names the new file must also be fsynced, otherwise the
    // first append can disappear after a power loss even though `append()`
    // returned success.
    let is_first_create = !path.exists();

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|_| durable_error("jsonl event store failed to open stream"))?;
    file.lock()
        .map_err(|_| durable_error("jsonl event store failed to acquire append lock"))?;

    // Re-read the prior tail under the lock so we observe writes from any
    // other process that just finished appending.
    let prior_tail = read_last_jsonl_cursor(path)?.unwrap_or(0);
    let next_cursor = prior_tail
        .checked_add(1)
        .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
    let line = serialise(next_cursor)?;

    file.write_all(line.as_bytes())
        .map_err(|_| durable_error("jsonl event store failed to append record"))?;
    file.write_all(b"\n")
        .map_err(|_| durable_error("jsonl event store failed to append record"))?;
    file.flush()
        .map_err(|_| durable_error("jsonl event store failed to flush record"))?;
    file.sync_data()
        .map_err(|_| durable_error("jsonl event store failed to sync record"))?;

    if is_first_create && let Some(parent) = path.parent() {
        // `File::open` on a directory + `sync_all` is the portable way to
        // fsync the directory entry on POSIX. Best-effort on platforms that
        // don't support it (e.g. Windows handles this implicitly).
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    // Lock releases when `file` drops at end of scope.
    Ok(next_cursor)
}

fn durable_error(reason: impl Into<String>) -> EventError {
    EventError::DurableLog {
        reason: reason.into(),
    }
}

fn stream_lock_key(kind: StreamKind, stream: &EventStreamKey) -> String {
    format!(
        "{}/{}/{}/{}",
        kind.lock_prefix(),
        stream.tenant_id.as_str(),
        stream.user_id.as_str(),
        stream
            .agent_id
            .as_ref()
            .map(AgentId::as_str)
            .unwrap_or("<none>")
    )
}

fn component(prefix: &str, value: &str) -> String {
    format!("{prefix}-{}", urlencoding::encode(value))
}

fn agent_component(agent_id: Option<&AgentId>) -> String {
    match agent_id {
        Some(agent_id) => component("agent-id", agent_id.as_str()),
        None => "agent-none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, TenantId, UserId};

    use super::*;

    #[tokio::test]
    async fn jsonl_stream_lock_registry_prunes_released_locks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = JsonlStore::new(temp.path().join("event-store"));
        let stream_a = EventStreamKey::new(
            TenantId::new("tenant-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(AgentId::new("agent-a").unwrap()),
        );
        let stream_b = EventStreamKey::new(
            TenantId::new("tenant-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(AgentId::new("agent-b").unwrap()),
        );

        let lock_a = store.stream_lock(StreamKind::Runtime, &stream_a).await;
        assert_eq!(store.locks.lock().await.len(), 1);
        drop(lock_a);

        let _lock_b = store.stream_lock(StreamKind::Runtime, &stream_b).await;
        assert_eq!(store.locks.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn production_rejects_cleartext_http_libsql_url() {
        let result = build_reborn_event_stores(
            RebornProfile::Production,
            RebornEventStoreConfig::Libsql {
                path_or_url: "http://libsql.example.com:8080".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
        ));
    }

    #[tokio::test]
    async fn local_dev_allows_cleartext_http_libsql_url() {
        // Non-production profiles can still use http:// for local sqld.
        // The build call will fail on the actual connection attempt below
        // for an unreachable address, but it must NOT fail with the
        // cleartext-disabled error.
        let result = build_reborn_event_stores(
            RebornProfile::LocalDev,
            RebornEventStoreConfig::Libsql {
                path_or_url: "http://127.0.0.1:1".to_string(),
                auth_token: None,
            },
        )
        .await;
        assert!(!matches!(
            result,
            Err(RebornEventStoreError::ProductionLibsqlClearTextDisabled)
        ));
    }
}
