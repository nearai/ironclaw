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
    sync::Arc,
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
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

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
        self.store
            .read_after(StreamKind::Runtime, stream, filter, after, limit, |event| {
                filter.matches_event(event)
            })
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
        self.store
            .read_after(StreamKind::Audit, stream, filter, after, limit, |record| {
                filter.matches_audit(record)
            })
            .await
    }
}

#[derive(Debug, Clone)]
struct JsonlStore {
    root: PathBuf,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
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
        T: Clone + Serialize + DeserializeOwned,
    {
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let entries = self.read_entries::<T>(kind, stream).await?;
        let next_cursor = entries
            .last()
            .map(|entry| entry.cursor.as_u64())
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
        let entry = EventLogEntry {
            cursor: EventCursor::new(next_cursor),
            record,
        };
        let envelope = JsonlEntry {
            cursor: entry.cursor,
            record: entry.record.clone(),
        };
        self.append_envelope(kind, stream, &envelope).await?;
        Ok(entry)
    }

    async fn read_after<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        _filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
        is_match: impl Fn(&T) -> bool,
    ) -> Result<EventReplay<T>, EventError>
    where
        T: Clone + DeserializeOwned,
    {
        if limit == 0 {
            return Err(EventError::InvalidReplayRequest {
                reason: "limit must be greater than zero".to_string(),
            });
        }
        let after = after.unwrap_or_default();
        let lock = self.stream_lock(kind, stream).await;
        let _guard = lock.lock().await;
        let entries = self.read_entries::<T>(kind, stream).await?;
        let head = entries
            .last()
            .map(|entry| entry.cursor)
            .unwrap_or_else(EventCursor::origin);
        if after.as_u64() > head.as_u64() {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: head,
            });
        }

        let mut replay_entries = Vec::new();
        let mut last_scanned = after;
        for entry in entries {
            if entry.cursor.as_u64() <= after.as_u64() {
                continue;
            }
            last_scanned = entry.cursor;
            if !is_match(&entry.record) {
                continue;
            }
            replay_entries.push(entry);
            if replay_entries.len() >= limit {
                break;
            }
        }
        let next_cursor = replay_entries
            .last()
            .map(|entry| entry.cursor)
            .unwrap_or(last_scanned);
        Ok(EventReplay {
            entries: replay_entries,
            next_cursor,
        })
    }

    async fn append_envelope<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        envelope: &JsonlEntry<T>,
    ) -> Result<(), EventError>
    where
        T: Serialize,
    {
        let path = self.stream_path(kind, stream);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| durable_error("jsonl event store failed to prepare stream"))?;
        }
        let line = serde_json::to_string(envelope).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|_| durable_error("jsonl event store failed to open stream"))?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|_| durable_error("jsonl event store failed to append record"))?;
        file.write_all(b"\n")
            .await
            .map_err(|_| durable_error("jsonl event store failed to append record"))?;
        file.flush()
            .await
            .map_err(|_| durable_error("jsonl event store failed to flush record"))?;
        file.sync_data()
            .await
            .map_err(|_| durable_error("jsonl event store failed to sync record"))?;
        Ok(())
    }

    async fn read_entries<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
    ) -> Result<Vec<EventLogEntry<T>>, EventError>
    where
        T: DeserializeOwned,
    {
        let path = self.stream_path(kind, stream);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(durable_error("jsonl event store failed to read stream")),
        };
        parse_jsonl_entries(&bytes)
    }

    async fn stream_lock(&self, kind: StreamKind, stream: &EventStreamKey) -> Arc<Mutex<()>> {
        let key = stream_lock_key(kind, stream);
        let mut locks = self.locks.lock().await;
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
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

fn parse_jsonl_entries<T>(bytes: &[u8]) -> Result<Vec<EventLogEntry<T>>, EventError>
where
    T: DeserializeOwned,
{
    let text = std::str::from_utf8(bytes).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    let mut entries = Vec::new();
    let mut expected_cursor = 1u64;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let envelope =
            serde_json::from_str::<JsonlEntry<T>>(line).map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })?;
        if envelope.cursor.as_u64() != expected_cursor {
            return Err(durable_error(
                "jsonl event stream cursor sequence is invalid",
            ));
        }
        entries.push(EventLogEntry {
            cursor: envelope.cursor,
            record: envelope.record,
        });
        expected_cursor = expected_cursor
            .checked_add(1)
            .ok_or_else(|| durable_error("jsonl event cursor overflowed u64"))?;
    }
    Ok(entries)
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
