use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::AuditEnvelope;
use secrecy::{ExposeSecret, SecretString};
use tokio_postgres::{Client, NoTls, types::ToSql};

use crate::{
    RebornEventStoreError, RebornEventStores, StreamKind, durable_error,
    sql_common::{
        SqlRecordMetadata, agent_db_key, audit_metadata, decode_record, empty_or_foreign_stream,
        filter_audit, filter_runtime, runtime_metadata, stream_from_audit, stream_from_runtime,
        validate_replay_request,
    },
};

const POSTGRES_EVENT_STORE_SCHEMA: &str =
    include_str!("../migrations/postgres/001_initial_event_store.sql");

pub(crate) async fn build_postgres_event_stores(
    url: SecretString,
) -> Result<RebornEventStores, RebornEventStoreError> {
    let (client, connection) = tokio_postgres::connect(url.expose_secret(), NoTls)
        .await
        .map_err(|source| RebornEventStoreError::backend("postgres", "connect", source))?;
    tokio::spawn(async move {
        if connection.await.is_err() {
            tracing::debug!("postgres event-store connection task exited with an error");
        }
    });

    let store = PostgresStore::new(client);
    store
        .run_migrations()
        .await
        .map_err(|source| RebornEventStoreError::backend("postgres", "run migrations", source))?;
    Ok(RebornEventStores {
        events: Arc::new(PostgresDurableEventLog::from_store(store.clone())),
        audit: Arc::new(PostgresDurableAuditLog::from_store(store)),
    })
}

#[derive(Clone)]
struct PostgresStore {
    client: Arc<Client>,
}

impl PostgresStore {
    fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
        }
    }

    async fn run_migrations(&self) -> Result<(), EventError> {
        self.client
            .batch_execute(POSTGRES_EVENT_STORE_SCHEMA)
            .await
            .map_err(|_| durable_error("postgres event store failed to run migrations"))
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
        let kind = kind.as_db_str();
        let agent_id = agent_db_key(stream.agent_id.as_ref());
        let record_id = uuid::Uuid::parse_str(&metadata.record_id)
            .map_err(|_| durable_error("postgres event record id is invalid"))?;
        let project_id = metadata.project_id.as_deref();
        let mission_id = metadata.mission_id.as_deref();
        let thread_id = metadata.thread_id.as_deref();
        let process_id = metadata
            .process_id
            .as_deref()
            .map(uuid::Uuid::parse_str)
            .transpose()
            .map_err(|_| durable_error("postgres event process id is invalid"))?;
        let occurred_at = metadata
            .occurred_at
            .parse::<ironclaw_host_api::Timestamp>()
            .map_err(|_| durable_error("postgres event timestamp is invalid"))?;
        let row = self
            .client
            .query_one(
                r#"
                WITH next_stream AS (
                    INSERT INTO reborn_event_streams (
                        stream_kind, tenant_id, user_id, agent_id, next_cursor, earliest_retained
                    )
                    VALUES ($1, $2, $3, $4, 1, 0)
                    ON CONFLICT (stream_kind, tenant_id, user_id, agent_id) DO UPDATE SET
                        next_cursor = reborn_event_streams.next_cursor + 1,
                        updated_at = NOW()
                    RETURNING next_cursor
                )
                INSERT INTO reborn_event_entries (
                    stream_kind, tenant_id, user_id, agent_id, cursor, record_id,
                    record_kind, project_id, mission_id, thread_id, process_id,
                    occurred_at, record_json
                )
                SELECT
                    $1, $2, $3, $4, next_cursor, $5,
                    $6, $7, $8, $9, $10,
                    $11, $12
                FROM next_stream
                RETURNING cursor
                "#,
                &[
                    &kind,
                    &stream.tenant_id.as_str(),
                    &stream.user_id.as_str(),
                    &agent_id,
                    &record_id,
                    &metadata.record_kind.as_str(),
                    &project_id,
                    &mission_id,
                    &thread_id,
                    &process_id,
                    &occurred_at,
                    &metadata.record_json,
                ],
            )
            .await
            .map_err(|_| durable_error("postgres event store failed to append record"))?;
        let cursor: i64 = row.get("cursor");
        u64::try_from(cursor).map_err(|_| durable_error("postgres event cursor is negative"))
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
        let kind = kind.as_db_str();
        let agent_id = agent_db_key(stream.agent_id.as_ref());
        let stream_row = self
            .client
            .query_opt(
                r#"
                SELECT next_cursor, earliest_retained
                FROM reborn_event_streams
                WHERE stream_kind = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id = $4
                "#,
                &[
                    &kind,
                    &stream.tenant_id.as_str(),
                    &stream.user_id.as_str(),
                    &agent_id,
                ],
            )
            .await
            .map_err(|_| durable_error("postgres event store failed to read stream"))?;
        let Some(row) = stream_row else {
            return empty_or_foreign_stream(after, limit);
        };
        let next_cursor = u64::try_from(row.get::<_, i64>("next_cursor"))
            .map_err(|_| durable_error("postgres stream cursor is negative"))?;
        let earliest_retained = u64::try_from(row.get::<_, i64>("earliest_retained"))
            .map_err(|_| durable_error("postgres stream retention cursor is negative"))?;
        validate_replay_request(next_cursor, earliest_retained, after, limit)?;

        let after_i64 = i64::try_from(after.as_u64())
            .map_err(|_| durable_error("postgres replay cursor exceeds i64"))?;
        let tenant_id = stream.tenant_id.as_str().to_string();
        let user_id = stream.user_id.as_str().to_string();
        let agent_id = agent_id.to_string();
        let project_filter = filter.project_id.as_ref().map(|id| id.as_str().to_string());
        let mission_filter = filter.mission_id.as_ref().map(|id| id.as_str().to_string());
        let thread_filter = filter.thread_id.as_ref().map(|id| id.as_str().to_string());
        let process_filter = filter.process_id.as_ref().map(|id| id.as_uuid());
        let limit_i64 =
            i64::try_from(limit).map_err(|_| durable_error("postgres replay limit exceeds i64"))?;
        let mut query = r#"
                SELECT cursor, record_json
                FROM reborn_event_entries
                WHERE stream_kind = $1
                    AND tenant_id = $2
                    AND user_id = $3
                    AND agent_id = $4
                    AND cursor > $5
                "#
        .to_string();
        let mut params: Vec<&(dyn ToSql + Sync)> =
            vec![&kind, &tenant_id, &user_id, &agent_id, &after_i64];
        if let Some(project_filter) = &project_filter {
            query.push_str(&format!(" AND project_id = ${}", params.len() + 1));
            params.push(project_filter);
        }
        if let Some(mission_filter) = &mission_filter {
            query.push_str(&format!(" AND mission_id = ${}", params.len() + 1));
            params.push(mission_filter);
        }
        if let Some(thread_filter) = &thread_filter {
            query.push_str(&format!(" AND thread_id = ${}", params.len() + 1));
            params.push(thread_filter);
        }
        if let Some(process_filter) = &process_filter {
            query.push_str(&format!(" AND process_id = ${}", params.len() + 1));
            params.push(process_filter);
        }
        query.push_str(&format!(" ORDER BY cursor ASC LIMIT ${}", params.len() + 1));
        params.push(&limit_i64);
        let rows = self
            .client
            .query(&query, &params)
            .await
            .map_err(|_| durable_error("postgres event store failed to read entries"))?;
        let mut entries = Vec::new();
        for row in rows {
            let cursor = u64::try_from(row.get::<_, i64>("cursor"))
                .map_err(|_| durable_error("postgres entry cursor is negative"))?;
            let value: serde_json::Value = row.get("record_json");
            let (record, matches) = decode_and_match(value)?;
            let cursor = EventCursor::new(cursor);
            if !matches {
                continue;
            }
            entries.push(EventLogEntry { cursor, record });
            if entries.len() >= limit {
                break;
            }
        }
        let next_cursor = entries
            .last()
            .map(|entry| entry.cursor)
            .unwrap_or_else(|| EventCursor::new(next_cursor));
        Ok(EventReplay {
            entries,
            next_cursor,
        })
    }
}

#[derive(Clone)]
pub struct PostgresDurableEventLog {
    store: PostgresStore,
}

impl PostgresDurableEventLog {
    fn from_store(store: PostgresStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for PostgresDurableEventLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresDurableEventLog")
            .field("client", &"<postgres_event_store>")
            .finish()
    }
}

#[async_trait]
impl DurableEventLog for PostgresDurableEventLog {
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
pub struct PostgresDurableAuditLog {
    store: PostgresStore,
}

impl PostgresDurableAuditLog {
    fn from_store(store: PostgresStore) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for PostgresDurableAuditLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresDurableAuditLog")
            .field("client", &"<postgres_event_store>")
            .finish()
    }
}

#[async_trait]
impl DurableAuditLog for PostgresDurableAuditLog {
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
