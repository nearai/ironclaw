use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::AuditEnvelope;
use secrecy::{ExposeSecret, SecretString};
use tokio_postgres::{Client, NoTls, types::ToSql};
use tokio_postgres_rustls::MakeRustlsConnect;

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

/// Returns true if the supplied Postgres URL targets a loopback host (or a
/// Unix socket). Anything else is treated as remote and must use TLS.
///
/// Conservative: an unparseable URL is treated as remote (fail closed).
fn is_local_postgres_url(url: &str) -> bool {
    // Unix socket connections (`host=/var/run/...`) have no `://` host.
    if !url.contains("://") {
        return true;
    }
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    match parsed.host_str() {
        // libpq treats an empty host as a Unix-socket connection.
        None | Some("") => true,
        Some(host) => matches!(
            host,
            "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
        ),
    }
}

/// Build a rustls TLS connector for remote Postgres connections.
///
/// Mirrors `src/db/tls.rs`: prefer the platform's native certificate store,
/// fall back to Mozilla's bundled webpki roots when the system store is empty.
fn make_rustls_connector() -> Result<MakeRustlsConnect, RebornEventStoreError> {
    let mut root_store = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for error in &native.errors {
        tracing::warn!("postgres event-store: error loading system root certs: {error}");
    }
    for cert in native.certs {
        if let Err(error) = root_store.add(cert) {
            tracing::warn!("postgres event-store: skipping invalid system root cert: {error}");
        }
    }
    if root_store.is_empty() {
        tracing::info!(
            "postgres event-store: no system root certificates found, using bundled Mozilla roots"
        );
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|source| RebornEventStoreError::backend("postgres", "configure rustls", source))?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

pub(crate) async fn build_postgres_event_stores(
    url: SecretString,
) -> Result<RebornEventStores, RebornEventStoreError> {
    let raw_url = url.expose_secret();
    let client = if is_local_postgres_url(raw_url) {
        let (client, connection) = tokio_postgres::connect(raw_url, NoTls)
            .await
            .map_err(|source| RebornEventStoreError::backend("postgres", "connect", source))?;
        tokio::spawn(async move {
            if connection.await.is_err() {
                tracing::debug!("postgres event-store connection task exited with an error");
            }
        });
        client
    } else {
        let tls = make_rustls_connector()?;
        let (client, connection) = tokio_postgres::connect(raw_url, tls)
            .await
            .map_err(|source| RebornEventStoreError::backend("postgres", "connect-tls", source))?;
        tokio::spawn(async move {
            if connection.await.is_err() {
                tracing::debug!("postgres event-store TLS connection task exited with an error");
            }
        });
        client
    };

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
        let mut last_scanned: Option<EventCursor> = None;
        for row in rows {
            let cursor = u64::try_from(row.get::<_, i64>("cursor"))
                .map_err(|_| durable_error("postgres entry cursor is negative"))?;
            let value: serde_json::Value = row.get("record_json");
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
        // Advance the cursor past records we scanned but filtered out, so the
        // next call does not rescan them forever. When no entries matched at
        // all, fall back to the stream head (`next_cursor` from the streams
        // table is the cursor of the most recent append).
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

#[cfg(test)]
mod tests {
    use super::is_local_postgres_url;

    #[test]
    fn local_postgres_urls_are_recognised() {
        for url in [
            "postgres://user:pass@localhost/db",
            "postgres://user@127.0.0.1:5432/db",
            "postgresql://localhost/db",
            "postgres://[::1]/db",
            "postgres://user@0.0.0.0/db",
            // Unix-socket-style: libpq treats these as local.
            "host=/var/run/postgresql user=ironclaw dbname=ironclaw",
        ] {
            assert!(
                is_local_postgres_url(url),
                "expected `{url}` to be detected as local"
            );
        }
    }

    #[test]
    fn remote_postgres_urls_require_tls() {
        for url in [
            "postgres://user:pass@db.internal/db",
            "postgres://user@10.0.0.5:5432/db",
            "postgresql://user@managed-postgres.example.com/db",
            "postgres://user@2001:db8::1/db",
        ] {
            assert!(
                !is_local_postgres_url(url),
                "expected `{url}` to require TLS"
            );
        }
    }

    #[test]
    fn unparseable_postgres_url_with_scheme_falls_closed_to_remote() {
        // An unparseable URL with a scheme is treated as remote so that
        // production cannot accidentally end up with a NoTls connector
        // because of a typo in the connection string.
        assert!(!is_local_postgres_url("postgres://%%%not-a-host"));
    }
}
