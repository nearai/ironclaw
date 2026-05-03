use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::AuditEnvelope;
use libsql::{Connection, Database, Transaction, params, params_from_iter};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    RebornEventStoreError, RebornEventStores, StreamKind, durable_error,
    sql_common::{
        SqlRecordMetadata, agent_db_key, audit_metadata, decode_record, empty_or_foreign_stream,
        filter_audit, filter_runtime, runtime_metadata, stream_from_audit, stream_from_runtime,
        validate_replay_request,
    },
};

const LIBSQL_EVENT_STORE_SCHEMA: &str =
    include_str!("../migrations/libsql/001_initial_event_store.sql");

pub(crate) async fn build_libsql_event_stores(
    path_or_url: String,
    auth_token: Option<SecretString>,
) -> Result<RebornEventStores, RebornEventStoreError> {
    let db = build_database(&path_or_url, auth_token).await?;
    let store = LibSqlStore::new(db);
    store
        .run_migrations()
        .await
        .map_err(|source| RebornEventStoreError::backend("libsql", "run migrations", source))?;
    Ok(RebornEventStores {
        events: Arc::new(LibSqlDurableEventLog::from_store(store.clone())),
        audit: Arc::new(LibSqlDurableAuditLog::from_store(store)),
    })
}

async fn build_database(
    path_or_url: &str,
    auth_token: Option<SecretString>,
) -> Result<Arc<Database>, RebornEventStoreError> {
    let db = if is_remote_libsql(path_or_url) {
        libsql::Builder::new_remote(
            path_or_url.to_string(),
            auth_token
                .as_ref()
                .map(|token| token.expose_secret().to_string())
                .unwrap_or_default(),
        )
        .build()
        .await
    } else {
        if path_or_url != ":memory:" {
            let path = Path::new(path_or_url);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|source| {
                    RebornEventStoreError::io("initialize libsql parent", source)
                })?;
            }
        }
        libsql::Builder::new_local(path_or_url).build().await
    };
    db.map(Arc::new)
        .map_err(|source| RebornEventStoreError::backend("libsql", "connect", source))
}

fn is_remote_libsql(path_or_url: &str) -> bool {
    path_or_url.starts_with("libsql://")
        || path_or_url.starts_with("https://")
        || path_or_url.starts_with("http://")
}

#[derive(Clone)]
struct LibSqlStore {
    db: Arc<Database>,
}

impl LibSqlStore {
    fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<Connection, EventError> {
        let conn = self
            .db
            .connect()
            .map_err(|_| durable_error("libsql event store failed to connect"))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|_| durable_error("libsql event store failed to configure busy timeout"))?;
        conn.query("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(|_| durable_error("libsql event store failed to enable foreign keys"))?;
        Ok(conn)
    }

    async fn run_migrations(&self) -> Result<(), EventError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_EVENT_STORE_SCHEMA)
            .await
            .map(|_| ())
            .map_err(|_| durable_error("libsql event store failed to run migrations"))
    }

    async fn append_runtime(
        &self,
        event: RuntimeEvent,
    ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        let stream = stream_from_runtime(&event);
        let metadata = runtime_metadata(&event)?;
        let cursor = self
            .append_record(StreamKind::Runtime, &stream, &metadata)
            .await?;
        Ok(EventLogEntry {
            cursor: EventCursor::new(cursor),
            record: event,
        })
    }

    async fn append_audit(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        let stream = stream_from_audit(&record);
        let metadata = audit_metadata(&record)?;
        let cursor = self
            .append_record(StreamKind::Audit, &stream, &metadata)
            .await?;
        Ok(EventLogEntry {
            cursor: EventCursor::new(cursor),
            record,
        })
    }

    async fn append_record(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        metadata: &SqlRecordMetadata,
    ) -> Result<u64, EventError> {
        let conn = self.connect().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|_| durable_error("libsql event store failed to begin append"))?;
        let result = self.append_record_in_tx(&tx, kind, stream, metadata).await;
        match result {
            Ok(cursor) => {
                tx.commit()
                    .await
                    .map_err(|_| durable_error("libsql event store failed to commit append"))?;
                Ok(cursor)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn append_record_in_tx(
        &self,
        tx: &Transaction,
        kind: StreamKind,
        stream: &EventStreamKey,
        metadata: &SqlRecordMetadata,
    ) -> Result<u64, EventError> {
        let now = metadata.occurred_at.as_str();
        let kind = kind.as_db_str();
        let agent_id = agent_db_key(stream.agent_id.as_ref());
        tx.execute(
            r#"
            INSERT INTO reborn_event_streams (
                stream_kind, tenant_id, user_id, agent_id, next_cursor,
                earliest_retained, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?5)
            ON CONFLICT (stream_kind, tenant_id, user_id, agent_id) DO NOTHING
            "#,
            params![
                kind,
                stream.tenant_id.as_str(),
                stream.user_id.as_str(),
                agent_id,
                now,
            ],
        )
        .await
        .map_err(|_| durable_error("libsql event store failed to initialize stream"))?;
        tx.execute(
            r#"
            UPDATE reborn_event_streams
            SET next_cursor = next_cursor + 1, updated_at = ?5
            WHERE stream_kind = ?1 AND tenant_id = ?2 AND user_id = ?3 AND agent_id = ?4
            "#,
            params![
                kind,
                stream.tenant_id.as_str(),
                stream.user_id.as_str(),
                agent_id,
                now,
            ],
        )
        .await
        .map_err(|_| durable_error("libsql event store failed to advance cursor"))?;
        let mut rows = tx
            .query(
                r#"
                SELECT next_cursor
                FROM reborn_event_streams
                WHERE stream_kind = ?1 AND tenant_id = ?2 AND user_id = ?3 AND agent_id = ?4
                "#,
                params![
                    kind,
                    stream.tenant_id.as_str(),
                    stream.user_id.as_str(),
                    agent_id,
                ],
            )
            .await
            .map_err(|_| durable_error("libsql event store failed to read cursor"))?;
        let row = rows
            .next()
            .await
            .map_err(|_| durable_error("libsql event store failed to read cursor"))?
            .ok_or_else(|| durable_error("libsql event stream cursor missing after update"))?;
        let cursor = row
            .get::<i64>(0)
            .map_err(|_| durable_error("libsql event stream cursor has invalid type"))?;
        let cursor =
            u64::try_from(cursor).map_err(|_| durable_error("libsql event cursor is negative"))?;
        let record_json = serde_json::to_string(&metadata.record_json).map_err(|error| {
            EventError::Serialize {
                reason: error.to_string(),
            }
        })?;
        tx.execute(
            r#"
            INSERT INTO reborn_event_entries (
                stream_kind, tenant_id, user_id, agent_id, cursor, record_id,
                record_kind, project_id, mission_id, thread_id, process_id,
                occurred_at, record_json, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?12)
            "#,
            params![
                kind,
                stream.tenant_id.as_str(),
                stream.user_id.as_str(),
                agent_id,
                i64::try_from(cursor)
                    .map_err(|_| durable_error("libsql event cursor exceeds i64"))?,
                metadata.record_id.as_str(),
                metadata.record_kind.as_str(),
                opt_text(metadata.project_id.as_deref()),
                opt_text(metadata.mission_id.as_deref()),
                opt_text(metadata.thread_id.as_deref()),
                opt_text(metadata.process_id.as_deref()),
                metadata.occurred_at.as_str(),
                record_json.as_str(),
            ],
        )
        .await
        .map_err(|_| durable_error("libsql event store failed to append record"))?;
        Ok(cursor)
    }

    async fn read_runtime(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        self.read_after(StreamKind::Runtime, stream, filter, after, limit, |value| {
            let event = decode_record::<RuntimeEvent>(value)?;
            let matches = filter_runtime(filter, &event);
            Ok((event, matches))
        })
        .await
    }

    async fn read_audit(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        self.read_after(StreamKind::Audit, stream, filter, after, limit, |value| {
            let record = decode_record::<AuditEnvelope>(value)?;
            let matches = filter_audit(filter, &record);
            Ok((record, matches))
        })
        .await
    }

    async fn read_after<T>(
        &self,
        kind: StreamKind,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
        decode_and_match: impl Fn(serde_json::Value) -> Result<(T, bool), EventError>,
    ) -> Result<EventReplay<T>, EventError>
    where
        T: Clone,
    {
        let after = after.unwrap_or_default();
        let conn = self.connect().await?;
        let kind = kind.as_db_str();
        let agent_id = agent_db_key(stream.agent_id.as_ref());
        let mut stream_rows = conn
            .query(
                r#"
                SELECT next_cursor, earliest_retained
                FROM reborn_event_streams
                WHERE stream_kind = ?1 AND tenant_id = ?2 AND user_id = ?3 AND agent_id = ?4
                "#,
                params![
                    kind,
                    stream.tenant_id.as_str(),
                    stream.user_id.as_str(),
                    agent_id,
                ],
            )
            .await
            .map_err(|_| durable_error("libsql event store failed to read stream"))?;
        let Some(row) = stream_rows
            .next()
            .await
            .map_err(|_| durable_error("libsql event store failed to read stream"))?
        else {
            return empty_or_foreign_stream(after, limit);
        };
        let next_cursor = u64::try_from(
            row.get::<i64>(0)
                .map_err(|_| durable_error("libsql stream cursor has invalid type"))?,
        )
        .map_err(|_| durable_error("libsql stream cursor is negative"))?;
        let earliest_retained = u64::try_from(
            row.get::<i64>(1)
                .map_err(|_| durable_error("libsql stream retention cursor has invalid type"))?,
        )
        .map_err(|_| durable_error("libsql stream retention cursor is negative"))?;
        validate_replay_request(next_cursor, earliest_retained, after, limit)?;

        let after_i64 = i64::try_from(after.as_u64())
            .map_err(|_| durable_error("libsql replay cursor exceeds i64"))?;
        let limit_i64 =
            i64::try_from(limit).map_err(|_| durable_error("libsql replay limit exceeds i64"))?;
        let mut query = r#"
                SELECT cursor, record_json
                FROM reborn_event_entries
                WHERE stream_kind = ?1
                    AND tenant_id = ?2
                    AND user_id = ?3
                    AND agent_id = ?4
                    AND cursor > ?5
                "#
        .to_string();
        let mut query_params = vec![
            libsql::Value::Text(kind.to_string()),
            libsql::Value::Text(stream.tenant_id.as_str().to_string()),
            libsql::Value::Text(stream.user_id.as_str().to_string()),
            libsql::Value::Text(agent_id.to_string()),
            libsql::Value::Integer(after_i64),
        ];
        push_text_filter(
            &mut query,
            &mut query_params,
            "project_id",
            filter.project_id.as_ref().map(|id| id.as_str()),
        );
        push_text_filter(
            &mut query,
            &mut query_params,
            "mission_id",
            filter.mission_id.as_ref().map(|id| id.as_str()),
        );
        push_text_filter(
            &mut query,
            &mut query_params,
            "thread_id",
            filter.thread_id.as_ref().map(|id| id.as_str()),
        );
        let process_filter = filter.process_id.as_ref().map(|id| id.to_string());
        push_text_filter(
            &mut query,
            &mut query_params,
            "process_id",
            process_filter.as_deref(),
        );
        query.push_str(&format!(
            " ORDER BY cursor ASC LIMIT ?{}",
            query_params.len() + 1
        ));
        query_params.push(libsql::Value::Integer(limit_i64));
        let mut rows = conn
            .query(&query, params_from_iter(query_params))
            .await
            .map_err(|_| durable_error("libsql event store failed to read entries"))?;
        let mut entries = Vec::new();
        let mut last_scanned: Option<EventCursor> = None;
        while let Some(row) = rows
            .next()
            .await
            .map_err(|_| durable_error("libsql event store failed to read entries"))?
        {
            let cursor = u64::try_from(
                row.get::<i64>(0)
                    .map_err(|_| durable_error("libsql entry cursor has invalid type"))?,
            )
            .map_err(|_| durable_error("libsql entry cursor is negative"))?;
            let record_json = row
                .get::<String>(1)
                .map_err(|_| durable_error("libsql entry JSON has invalid type"))?;
            let value =
                serde_json::from_str::<serde_json::Value>(&record_json).map_err(|error| {
                    EventError::Serialize {
                        reason: error.to_string(),
                    }
                })?;
            let (record, matches) = decode_and_match(value)?;
            let cursor = EventCursor::new(cursor);
            last_scanned = Some(cursor);
            if !matches {
                continue;
            }
            entries.push(EventLogEntry { cursor, record });
            if entries.len() >= limit {
                break;
            }
        }
        // Track the highest cursor we scanned so callers can advance past
        // records that were filtered out at the application layer. Without
        // this, a filtered-out record at the head of the stream would be
        // rescanned indefinitely on every replay.
        let last_matched = entries.last().map(|entry| entry.cursor);
        let next_cursor = match (last_matched, last_scanned) {
            (Some(matched), Some(scanned)) if scanned.as_u64() > matched.as_u64() => scanned,
            (Some(matched), _) => matched,
            (None, Some(scanned)) => scanned,
            (None, None) => EventCursor::new(next_cursor),
        };
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }
}

#[derive(Clone)]
pub struct LibSqlDurableEventLog {
    store: LibSqlStore,
}

impl LibSqlDurableEventLog {
    fn from_store(store: LibSqlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for LibSqlDurableEventLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LibSqlDurableEventLog")
            .field("db", &"<libsql_event_store>")
            .finish()
    }
}

#[async_trait]
impl DurableEventLog for LibSqlDurableEventLog {
    async fn append(&self, event: RuntimeEvent) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        self.store.append_runtime(event).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        self.store.read_runtime(stream, filter, after, limit).await
    }
}

#[derive(Clone)]
pub struct LibSqlDurableAuditLog {
    store: LibSqlStore,
}

impl LibSqlDurableAuditLog {
    fn from_store(store: LibSqlStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for LibSqlDurableAuditLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LibSqlDurableAuditLog")
            .field("db", &"<libsql_event_store>")
            .finish()
    }
}

#[async_trait]
impl DurableAuditLog for LibSqlDurableAuditLog {
    async fn append(
        &self,
        record: AuditEnvelope,
    ) -> Result<EventLogEntry<AuditEnvelope>, EventError> {
        self.store.append_audit(record).await
    }

    async fn read_after_cursor(
        &self,
        stream: &EventStreamKey,
        filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        self.store.read_audit(stream, filter, after, limit).await
    }
}

fn opt_text(value: Option<&str>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}

fn push_text_filter(
    query: &mut String,
    params: &mut Vec<libsql::Value>,
    column: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        query.push_str(&format!(" AND {column} = ?{}", params.len() + 1));
        params.push(libsql::Value::Text(value.to_string()));
    }
}
