use std::{collections::BTreeMap, error::Error, time::Duration};

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::db::{
    db_error, direct_children, directory_append_error, directory_write_error, escape_like_literal,
    escape_like_with_trailing_wildcard, infrastructure_pg_error, is_not_found, not_found,
    page_offset_to_i64, record_version_from_i64, record_version_to_i64, sql_index_name,
    system_time_from_unix_seconds, virtual_path_prefixes,
};
use crate::vector::{cosine_similarity, decode_embedding_blob};
use crate::{
    BackendCapabilities, Capability, CasExpectation, ContentType, DirEntry, Entry, FileStat,
    FileType, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind, IndexSpec,
    IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, SeqNo, TxnCapability,
    VersionedEntry,
};

#[cfg(feature = "postgres")]
/// PostgreSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct PostgresRootFilesystem {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
const POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV: &str =
    "IRONCLAW_FILESYSTEM_POSTGRES_MIGRATION_CONNECT_MAX_WAIT_SECS";
#[cfg(feature = "postgres")]
const POSTGRES_MIGRATION_CONNECT_DEFAULT_MAX_WAIT: Duration = Duration::from_secs(300);
#[cfg(feature = "postgres")]
const POSTGRES_MIGRATION_CONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
#[cfg(feature = "postgres")]
const POSTGRES_MIGRATION_CONNECT_MAX_BACKOFF: Duration = Duration::from_secs(10);

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self.migration_client_with_retry().await?;
        client
            .batch_execute(POSTGRES_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| infrastructure_pg_error(FilesystemOperation::CreateDirAll, error))
    }

    async fn migration_client_with_retry(
        &self,
    ) -> Result<deadpool_postgres::Object, FilesystemError> {
        let max_wait = postgres_migration_connect_max_wait()?;
        let started_at = tokio::time::Instant::now();
        let mut attempt = 0u32;
        loop {
            attempt = attempt.saturating_add(1);
            match self.client().await {
                Ok(client) => return Ok(client),
                Err(error) => {
                    let elapsed = started_at.elapsed();
                    if elapsed >= max_wait {
                        return Err(error);
                    }
                    let remaining = max_wait - elapsed;
                    let delay = postgres_migration_connect_backoff(attempt - 1).min(remaining);
                    tracing::debug!(
                        attempt,
                        max_wait_ms = max_wait.as_millis(),
                        elapsed_ms = elapsed.as_millis(),
                        retry_after_ms = delay.as_millis(),
                        "postgres root filesystem migration connect failed; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, FilesystemError> {
        self.pool.get().await.map_err(|error| {
            let reason = format!(
                "failed to create PostgreSQL filesystem connection: {}",
                format_error_chain(&error)
            );
            tracing::debug!(
                %reason,
                "postgres root filesystem pool checkout failed"
            );
            FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::Connect,
                reason,
            }
        })
    }
}

#[cfg(feature = "postgres")]
fn postgres_migration_connect_backoff(attempt: u32) -> Duration {
    POSTGRES_MIGRATION_CONNECT_INITIAL_BACKOFF
        .saturating_mul(2u32.saturating_pow(attempt.min(16)))
        .min(POSTGRES_MIGRATION_CONNECT_MAX_BACKOFF)
}

#[cfg(feature = "postgres")]
fn postgres_migration_connect_max_wait() -> Result<Duration, FilesystemError> {
    match std::env::var(POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV) {
        Ok(raw) => {
            let seconds =
                raw.trim()
                    .parse::<u64>()
                    .map_err(|_| FilesystemError::BackendInfrastructure {
                        operation: FilesystemOperation::Connect,
                        reason: format!(
                            "{POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV} must be a positive integer"
                        ),
                    })?;
            if seconds == 0 {
                return Err(FilesystemError::BackendInfrastructure {
                    operation: FilesystemOperation::Connect,
                    reason: format!(
                        "{POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV} must be greater than 0"
                    ),
                });
            }
            Ok(Duration::from_secs(seconds))
        }
        Err(std::env::VarError::NotPresent) => Ok(POSTGRES_MIGRATION_CONNECT_DEFAULT_MAX_WAIT),
        Err(std::env::VarError::NotUnicode(_)) => Err(FilesystemError::BackendInfrastructure {
            operation: FilesystemOperation::Connect,
            reason: format!("{POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV} must be valid Unicode"),
        }),
    }
}

#[cfg(feature = "postgres")]
fn format_error_chain(error: &(dyn Error + 'static)) -> String {
    let mut reason = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        reason.push_str(": ");
        reason.push_str(&error.to_string());
        source = error.source();
    }
    reason
}

#[cfg(feature = "postgres")]
#[async_trait]
impl RootFilesystem for PostgresRootFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        // sql_typical: read/write/append/list/stat/delete/records/query/
        // IndexExact/IndexPrefix/CAS. Events join the set with the V30
        // append/tail backing table. Postgres has native `tsvector` /
        // `plainto_tsquery` so we advertise IndexFts. Vector indexing is
        // currently a brute-force cosine ranker against `indexed->>'key'`
        // values stored as IndexValue::Bytes; we advertise IndexVector but
        // do not require pgvector.
        BackendCapabilities::sql_typical()
            .with(Capability::Events)
            .with(Capability::IndexFts)
            .with(Capability::IndexVector)
            .with_txn(TxnCapability::MultiKey)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let client = self.client().await?;
        postgres_put_with_client(&client, path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let client = self.client().await?;
        postgres_get_with_client(&client, path).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let kind_str = match &spec.kind {
            IndexKind::Exact => "exact".to_string(),
            IndexKind::Prefix => "prefix".to_string(),
            IndexKind::Fts => "fts".to_string(),
            IndexKind::Vector { dim } => format!("vector:{dim}"),
        };
        if spec.keys.is_empty() {
            return Err(FilesystemError::IndexConflict {
                path: path.clone(),
                name: spec.name.clone(),
                reason: crate::IndexConflictReason::EmptyKeys,
            });
        }
        let keys_json = serde_json::to_value(
            spec.keys
                .iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| FilesystemError::SerializeIndexed {
            path: path.clone(),
            operation: FilesystemOperation::EnsureIndex,
        })?;

        let client = self.client().await?;
        // PR #3661 reviewer fix: race-idempotent declaration. Single
        // INSERT ... ON CONFLICT DO NOTHING followed by a read-back +
        // canonical-spec equality check. Two concurrent declarers of the
        // same spec both succeed; declarers of conflicting specs see
        // IndexConflict deterministically.
        cached_execute(
            &client,
            "INSERT INTO root_filesystem_index_specs (prefix, name, keys, kind) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (prefix, name) DO NOTHING",
            &[&path.as_str(), &spec.name.as_str(), &keys_json, &kind_str],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::EnsureIndex, error))?;

        let row = cached_query_opt(
            &client,
            "SELECT keys, kind FROM root_filesystem_index_specs WHERE prefix = $1 AND name = $2",
            &[&path.as_str(), &spec.name.as_str()],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::EnsureIndex, error))?
        .ok_or_else(|| FilesystemError::IndexSpecMissingAfterUpsert {
            path: path.clone(),
            name: spec.name.clone(),
        })?;
        let existing_keys: serde_json::Value = row.get("keys");
        let existing_kind: String = row.get("kind");
        if existing_keys != keys_json || existing_kind != kind_str {
            return Err(FilesystemError::IndexConflict {
                path: path.clone(),
                name: spec.name.clone(),
                reason: crate::IndexConflictReason::SpecMismatch,
            });
        }

        let index_name = sql_index_name(path.as_str(), spec.name.as_str());
        match &spec.kind {
            IndexKind::Exact | IndexKind::Prefix => {
                let expressions: Vec<String> = spec
                    .keys
                    .iter()
                    .map(|k| format!("((indexed->>'{}'))", k.as_str()))
                    .collect();
                let ddl = format!(
                    "CREATE INDEX IF NOT EXISTS {index_name} ON root_filesystem_entries ({})",
                    expressions.join(", ")
                );
                client.batch_execute(&ddl).await.map_err(|error| {
                    db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
            }
            IndexKind::Fts => {
                if spec.keys.len() != 1 {
                    return Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    });
                }
                let fts_key = spec.keys[0].as_str();
                // GIN expression index on a `to_tsvector(...)` over the
                // indexed JSON projection. Postgres `to_tsvector` returns
                // `tsvector`, which GIN indexes natively. The matching
                // predicate at query time uses `@@ plainto_tsquery`.
                //
                // Audit finding F4: libsql FTS5 virtual tables are
                // declared per-mount-prefix (one vtable per
                // `ensure_index(prefix, ...)`), so a query at one prefix
                // can't accidentally match indexed rows under a sibling
                // prefix. The Postgres GIN index, by contrast, used to
                // be global over `root_filesystem_entries` regardless of
                // which prefix declared it — fine for correctness (the
                // query path still scopes by `path LIKE prefix/%`) but
                // it breaks parity in two ways: a search at `/memory/a`
                // would have its planner consider postings from
                // `/memory/b` before filtering, and `DROP INDEX` for a
                // prefix-scoped tear-down would impact other prefixes.
                // Make the index a partial index gated by the prefix to
                // restore parity.
                let prefix_pattern =
                    escape_like_with_trailing_wildcard(&format!("{}/%", path.as_str()));
                let prefix_literal = prefix_pattern.replace('\'', "''");
                let ddl = format!(
                    "CREATE INDEX IF NOT EXISTS {index_name} ON root_filesystem_entries \
                     USING GIN (to_tsvector('english', COALESCE(indexed->>'{fts_key}', ''))) \
                     WHERE path = '{path_literal}' OR path LIKE '{prefix_literal}' ESCAPE '!'",
                    path_literal = path.as_str().replace('\'', "''"),
                );
                client.batch_execute(&ddl).await.map_err(|error| {
                    db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
            }
            IndexKind::Vector { dim } => {
                if *dim == 0 {
                    return Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    });
                }
                // Vector storage = IndexValue::Bytes in the indexed JSON
                // projection. No per-row table or DDL is required; queries
                // brute-force cosine over the candidate set. pgvector
                // support could be layered in later via a dialect probe
                // (`SELECT * FROM pg_extension WHERE extname='vector'`)
                // without changing this trait surface.
            }
        }
        Ok(())
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        // Vector-nearest is evaluated by ranking the candidate set in Rust.
        if let Filter::VectorNearest {
            key,
            embedding,
            limit,
        } = filter
        {
            return self
                .vector_nearest_query(path, key, embedding, *limit)
                .await;
        }
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();
        let path_str = path.as_str().to_string();
        let (prefix_lower, prefix_upper) = descendant_path_range(path);
        params.push(Box::new(path_str));
        params.push(Box::new(prefix_lower));
        params.push(Box::new(prefix_upper));

        let mut conditions = String::new();
        translate_filter(path, filter, &mut conditions, &mut params)?;

        let mut sql = String::from(
            "SELECT path, contents, content_type, kind, indexed, version \
             FROM root_filesystem_entries \
             WHERE is_dir = FALSE AND (path = $1 OR (path >= $2 AND path < $3))",
        );
        if !conditions.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&conditions);
        }
        sql.push_str(&format!(
            " ORDER BY path LIMIT ${} OFFSET ${}",
            params.len() + 1,
            params.len() + 2
        ));
        // `page.limit` is `u32` and clamped to `Page::MAX_LIMIT`, so the
        // `i64::from` is safe by construction. `page.offset` is `u64` and
        // user-supplied — guard with `try_from` so values ≥ 2^63 surface
        // a typed `Backend` error instead of wrapping to a negative
        // OFFSET. (Audit finding F6.)
        params.push(Box::new(i64::from(page.limit.min(Page::MAX_LIMIT))));
        params.push(Box::new(page_offset_to_i64(path, page.offset)?));

        let client = self.client().await?;
        let params_ref: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref() as _).collect();
        let rows = client
            .query(sql.as_str(), &params_ref[..])
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Query, error))?;
        rows.into_iter()
            .map(|row| {
                let row_path: String = row.get("path");
                let row_path = VirtualPath::new(row_path)?;
                let body: Vec<u8> = row.get("contents");
                let content_type_raw: String = row.get("content_type");
                let kind_raw: Option<String> = row.get("kind");
                let indexed_value: serde_json::Value = row.get("indexed");
                let version_raw: i64 = row.get("version");
                let entry =
                    build_entry(&row_path, body, content_type_raw, kind_raw, indexed_value)?;
                let version = record_version_from_i64(&row_path, version_raw)?;
                Ok(VersionedEntry {
                    path: row_path,
                    entry,
                    version,
                })
            })
            .collect()
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let client = self.client().await?;
        let row = cached_query_opt(
            &client,
            "SELECT contents, is_dir FROM root_filesystem_entries WHERE path = $1",
            &[&path.as_str()],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = row else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: bool = row.get("is_dir");
        if is_dir {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        Ok(row.get("contents"))
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let client = self.client().await?;
        let max_bytes = max_bytes as i64;
        let row = cached_query_opt(
            &client,
            r#"
                SELECT
                    CASE
                        WHEN octet_length(contents)::BIGINT <= $2 THEN contents
                        ELSE NULL
                    END AS contents,
                    octet_length(contents)::BIGINT AS len,
                    is_dir
                FROM root_filesystem_entries
                WHERE path = $1
                "#,
            &[&path.as_str(), &max_bytes],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = row else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: bool = row.get("is_dir");
        if is_dir {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        let len: i64 = row.get("len");
        if len > max_bytes {
            return Ok(None);
        }
        Ok(Some(row.get("contents")))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        if matches!(
            self.exact_entry_with_client(&client, path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry_with_client(&client, path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        // PR #3660 reviewer fix: legacy write_file must reset content_type
        // / kind / indexed and bump version, otherwise get() after
        // write_file-overwrite of a previously record-shaped entry
        // returns stale metadata.
        let rows = cached_execute(
            &client,
            r#"
                INSERT INTO root_filesystem_entries
                    (path, contents, is_dir, content_type, kind, indexed, version)
                VALUES ($1, $2, FALSE, 'application/octet-stream', NULL, '{}'::jsonb, 1)
                ON CONFLICT (path) DO UPDATE SET
                    contents = EXCLUDED.contents,
                    is_dir = FALSE,
                    content_type = EXCLUDED.content_type,
                    kind = EXCLUDED.kind,
                    indexed = EXCLUDED.indexed,
                    version = root_filesystem_entries.version + 1,
                    updated_at = NOW()
                WHERE root_filesystem_entries.is_dir = FALSE
                "#,
            &[&path.as_str(), &bytes],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
        if rows == 0 {
            return Err(directory_write_error(path.clone()));
        }
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        if matches!(
            self.exact_entry_with_client(&client, path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry_with_client(&client, path).await?
        {
            return Err(directory_append_error(path.clone()));
        }
        // PR #3660 reviewer fix: append also resets schema metadata.
        // Appending bytes onto a previously record-shaped entry was always
        // a category error; surface it by clearing the schema metadata
        // rather than leaving it stale on top of changed bytes.
        // Note: append rewrites the whole DB row. This is acceptable for
        // the legacy bytes plane (slated for removal in the consumer-
        // migration cleanup pass — see RootFilesystem::append_file's
        // deprecation note). New callers must use `append`/`tail` for
        // log-shaped mounts or `get`+`put` read-modify-write.
        cached_execute(
            &client,
            r#"
                INSERT INTO root_filesystem_entries
                    (path, contents, is_dir, content_type, kind, indexed, version)
                VALUES ($1, $2, FALSE, 'application/octet-stream', NULL, '{}'::jsonb, 1)
                ON CONFLICT (path) DO UPDATE SET
                    contents = root_filesystem_entries.contents || EXCLUDED.contents,
                    is_dir = FALSE,
                    content_type = EXCLUDED.content_type,
                    kind = EXCLUDED.kind,
                    indexed = EXCLUDED.indexed,
                    version = root_filesystem_entries.version + 1,
                    updated_at = NOW()
                "#,
            &[&path.as_str(), &bytes],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let client = self.client().await?;
        let exact_entry = self.exact_entry_with_client(&client, path).await?;
        if matches!(exact_entry, Some((_, FileType::File, _))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self
            .child_entries_with_client(&client, path, FilesystemOperation::ListDir)
            .await?;
        let children = direct_children(path, rows);
        if matches!(exact_entry, Some((_, FileType::Directory, _))) && is_not_found(&children) {
            return Ok(Vec::new());
        }
        children
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let client = self.client().await?;
        if let Some((len, file_type, modified)) =
            self.exact_entry_with_client(&client, path).await?
        {
            return Ok(FileStat {
                path: path.clone(),
                file_type,
                len,
                modified,
                sensitive: false,
            });
        }
        if self.has_child_entry_with_client(&client, path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            });
        }
        Err(not_found(path.clone(), FilesystemOperation::Stat))
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        postgres_delete_with_client(&client, path).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let client = self.client().await?;
        client
            .batch_execute("BEGIN")
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::BeginTxn, error))?;
        Ok(Box::new(PostgresStorageTxn {
            client: Some(client),
            prefix: path.clone(),
            active: true,
        }))
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let client = self.client().await?;
        let row = cached_query_one(
            &client,
            r#"
                INSERT INTO root_filesystem_events (path, payload)
                VALUES ($1, $2)
                RETURNING id
                "#,
            &[&path.as_str(), &payload],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::Append, error))?;
        let id: i64 = row.get("id");
        seq_no_from_i64(path, id, FilesystemOperation::Append)
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.tail_bounded(path, from, usize::MAX).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        if max_records == 0 {
            return Ok(Vec::new());
        }
        let client = self.client().await?;
        let from_raw = i64::try_from(from.get()).map_err(|_| {
            backend_error(
                path.clone(),
                FilesystemOperation::Tail,
                "tail cursor exceeds i64",
            )
        })?;
        // silent-ok: callers can request an unbounded tail; saturating keeps the
        // SQL LIMIT representable without changing the public trait contract.
        let limit_raw = i64::try_from(max_records).unwrap_or(i64::MAX);
        let rows = cached_query(
            &client,
            r#"
                SELECT id, payload
                FROM root_filesystem_events
                WHERE path = $1 AND id > $2
                ORDER BY id ASC
                LIMIT $3
                "#,
            &[&path.as_str(), &from_raw, &limit_raw],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::Tail, error))?;
        rows.into_iter()
            .map(|row| {
                let id: i64 = row.get("id");
                let payload: Vec<u8> = row.get("payload");
                Ok(EventRecord {
                    seq: seq_no_from_i64(path, id, FilesystemOperation::Tail)?,
                    payload,
                })
            })
            .collect()
    }

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        let client = self.client().await?;
        let from_raw = i64::try_from(from.get()).map_err(|_| {
            backend_error(
                path.clone(),
                FilesystemOperation::HeadSeq,
                "head_seq cursor exceeds i64",
            )
        })?;
        let row = cached_query_one(
            &client,
            r#"
                SELECT MAX(id) AS head
                FROM root_filesystem_events
                WHERE path = $1 AND id > $2
                "#,
            &[&path.as_str(), &from_raw],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::HeadSeq, error))?;
        // `MAX(...)` over an empty match set yields SQL NULL.
        let head_raw: Option<i64> = row.get("head");
        match head_raw {
            Some(id) => Ok(Some(seq_no_from_i64(
                path,
                id,
                FilesystemOperation::HeadSeq,
            )?)),
            None => Ok(None),
        }
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        for prefix in virtual_path_prefixes(path)? {
            let row = transaction
                .query_opt(
                    "SELECT is_dir FROM root_filesystem_entries WHERE path = $1",
                    &[&prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
            if row.is_some_and(|row| !row.get::<_, bool>("is_dir")) {
                return Err(FilesystemError::Backend {
                    path: prefix,
                    operation: FilesystemOperation::CreateDirAll,
                    reason: "file exists where directory is required".to_string(),
                });
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir)
                    VALUES ($1, ''::bytea, TRUE)
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    &[&prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    async fn exact_entry_with_client(
        &self,
        client: &deadpool_postgres::Object,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType, Option<std::time::SystemTime>)>, FilesystemError> {
        let row = cached_query_opt(
            client,
            "SELECT OCTET_LENGTH(contents)::bigint AS len, is_dir, EXTRACT(EPOCH FROM updated_at)::bigint AS updated_at_epoch FROM root_filesystem_entries WHERE path = $1",
            &[&path.as_str()],
        )
        .await
        .map_err(|error| db_error(path.clone(), FilesystemOperation::Stat, error))?;
        Ok(row.map(|row| {
            let len: i64 = row.get("len");
            let is_dir: bool = row.get("is_dir");
            let updated_at_epoch: i64 = row.get("updated_at_epoch");
            (
                if is_dir { 0 } else { len.max(0) as u64 },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
                system_time_from_unix_seconds(updated_at_epoch),
            )
        }))
    }

    async fn child_entries_with_client(
        &self,
        client: &deadpool_postgres::Object,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let (prefix_lower, prefix_upper) = descendant_path_range(parent);
        let rows = cached_query(
            client,
            "SELECT path, OCTET_LENGTH(contents)::bigint AS len, is_dir FROM root_filesystem_entries WHERE path >= $1 AND path < $2 ORDER BY path",
            &[&prefix_lower, &prefix_upper],
        )
        .await
        .map_err(|error| db_error(parent.clone(), operation, error))?;
        rows.into_iter()
            .map(|row| {
                let path: String = row.get("path");
                let len: i64 = row.get("len");
                let is_dir: bool = row.get("is_dir");
                Ok((
                    VirtualPath::new(path)?,
                    if is_dir { 0 } else { len.max(0) as u64 },
                    if is_dir {
                        FileType::Directory
                    } else {
                        FileType::File
                    },
                ))
            })
            .collect()
    }

    async fn has_child_entry_with_client(
        &self,
        client: &deadpool_postgres::Object,
        parent: &VirtualPath,
    ) -> Result<bool, FilesystemError> {
        let (prefix_lower, prefix_upper) = descendant_path_range(parent);
        let row = cached_query_opt(
            client,
            "SELECT 1 FROM root_filesystem_entries WHERE path >= $1 AND path < $2 LIMIT 1",
            &[&prefix_lower, &prefix_upper],
        )
        .await
        .map_err(|error| db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        Ok(row.is_some())
    }

    /// Brute-force cosine ranker over candidate rows in this prefix whose
    /// indexed projection has an `IndexValue::Bytes` value at `key`. Used
    /// when the caller's filter is a top-level `VectorNearest`.
    ///
    /// Two-phase to bound memory on large prefixes (review feedback on
    /// the unified-FS rework): first SELECT `(path, indexed, version)`
    /// for every candidate, rank by cosine in Rust, then `get()` the
    /// top-k entries to materialize bodies. Rows outside the cutoff
    /// never have their `contents` bytea loaded.
    async fn vector_nearest_query(
        &self,
        path: &VirtualPath,
        key: &IndexKey,
        embedding: &[f32],
        limit: u32,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let client = self.client().await?;
        let (prefix_lower, prefix_upper) = descendant_path_range(path);
        let rows = client
            .query(
                "SELECT path, indexed, version \
                 FROM root_filesystem_entries \
                 WHERE is_dir = FALSE AND (path = $1 OR (path >= $2 AND path < $3))",
                &[&path.as_str(), &prefix_lower, &prefix_upper],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Query, error))?;
        let mut ranked: Vec<(VirtualPath, RecordVersion, f32)> = Vec::new();
        for row in rows {
            let row_path: String = row.get("path");
            let row_path = VirtualPath::new(row_path)?;
            let indexed_value: serde_json::Value = row.get("indexed");
            let version_raw: i64 = row.get("version");
            let indexed: BTreeMap<IndexKey, IndexValue> = if indexed_value.is_null() {
                BTreeMap::new()
            } else {
                serde_json::from_value(indexed_value).map_err(|_| {
                    FilesystemError::DeserializeIndexed {
                        path: row_path.clone(),
                        operation: FilesystemOperation::Query,
                    }
                })?
            };
            let Some(IndexValue::Bytes(bytes)) = indexed.get(key) else {
                continue;
            };
            let Some(vec) = decode_embedding_blob(bytes) else {
                continue;
            };
            let Some(score) = cosine_similarity(embedding, &vec) else {
                continue;
            };
            let version = record_version_from_i64(&row_path, version_raw)?;
            ranked.push((row_path, version, score));
        }
        // Sort by descending cosine score, then ascending path for a stable
        // tie-breaker so equal-score rows truncate deterministically across
        // runs and across backends. The in-memory reference uses the same
        // tie-breaker; this keeps cross-backend behavior aligned.
        ranked.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.as_str().cmp(b.0.as_str()))
        });
        ranked.truncate(limit as usize);
        // Release the client so each `get()` below claims its own
        // pooled connection rather than serializing through one.
        drop(client);
        self.materialize_ranked(ranked).await
    }

    /// Phase-2 of [`vector_nearest_query`]: load full [`VersionedEntry`]
    /// bodies for the ranked-and-truncated candidate set. Mirrors the
    /// libSQL backend's `materialize_ranked`, including the silently-skip
    /// behaviour when a candidate path disappears between phase-1
    /// ranking and the phase-2 `get`. Pulled out so the concurrent-delete
    /// branch has a deterministic test seam.
    pub(crate) async fn materialize_ranked(
        &self,
        ranked: Vec<(VirtualPath, RecordVersion, f32)>,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let mut out = Vec::with_capacity(ranked.len());
        for (row_path, _version, _score) in ranked {
            let Some(versioned) = self.get(&row_path).await? else {
                // Concurrent delete between the ranking SELECT and
                // the body fetch — skip rather than error so the
                // search doesn't blow up on a race.
                continue;
            };
            out.push(versioned);
        }
        Ok(out)
    }
}

#[cfg(feature = "postgres")]
struct PostgresStorageTxn {
    client: Option<deadpool_postgres::Object>,
    prefix: VirtualPath,
    active: bool,
}

#[cfg(feature = "postgres")]
impl PostgresStorageTxn {
    fn client(&self) -> Result<&deadpool_postgres::Object, FilesystemError> {
        self.client
            .as_ref()
            .ok_or_else(|| FilesystemError::Backend {
                path: self.prefix.clone(),
                operation: FilesystemOperation::BeginTxn,
                reason: "postgres transaction already finished".to_string(),
            })
    }

    fn check_path(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if crate::path_prefix_matches(self.prefix.as_str(), path.as_str()) {
            Ok(())
        } else {
            Err(FilesystemError::PathOutsideMount { path: path.clone() })
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl StorageTxn for PostgresStorageTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check_path(path)?;
        postgres_put_with_client(self.client()?, path, entry, cas).await
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check_path(path)?;
        postgres_get_with_client(self.client()?, path).await
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check_path(path)?;
        postgres_delete_with_client(self.client()?, path).await
    }

    async fn commit(mut self: Box<Self>) -> Result<(), FilesystemError> {
        let client = self.client.take().ok_or_else(|| FilesystemError::Backend {
            path: self.prefix.clone(),
            operation: FilesystemOperation::BeginTxn,
            reason: "postgres transaction already finished".to_string(),
        })?;
        match client.batch_execute("COMMIT").await {
            Ok(()) => {
                self.active = false;
                Ok(())
            }
            Err(error) => {
                let mapped = db_error(self.prefix.clone(), FilesystemOperation::BeginTxn, error);
                let _ = client.batch_execute("ROLLBACK").await;
                self.active = false;
                Err(mapped)
            }
        }
    }

    async fn rollback(mut self: Box<Self>) {
        if let Some(client) = self.client.take()
            && self.active
        {
            let _ = client.batch_execute("ROLLBACK").await;
            self.active = false;
        }
    }
}

#[cfg(feature = "postgres")]
impl Drop for PostgresStorageTxn {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(client) = self.client.take() {
            tokio::spawn(async move {
                let _ = client.batch_execute("ROLLBACK").await;
            });
        }
    }
}

/// Prepare-cached variants of the hot fixed-SQL query paths.
///
/// `deadpool_postgres` keeps a per-connection statement cache, so issuing a
/// fixed SQL string through `prepare_cached` pays the `Parse` round-trip once
/// per connection instead of on every call (~2.8ms RTT to remote Postgres in
/// production). The pooled connection is held for less time per op, which is
/// what keeps the small hosted pool from starving the heartbeat/webui and
/// wedging the runner lease.
///
/// Only use these with *static* SQL — dynamic SQL would grow the cache
/// unbounded, so the filter `query` and index DDL paths stay on the uncached
/// `tokio_postgres` calls. The error type stays `tokio_postgres::Error` so
/// existing `db_error` mapping at call sites is unchanged.
#[cfg(feature = "postgres")]
async fn cached_query_opt(
    client: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<Option<tokio_postgres::Row>, tokio_postgres::Error> {
    let statement = client.prepare_cached(sql).await?;
    client.query_opt(&statement, params).await
}

#[cfg(feature = "postgres")]
async fn cached_query(
    client: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<Vec<tokio_postgres::Row>, tokio_postgres::Error> {
    let statement = client.prepare_cached(sql).await?;
    client.query(&statement, params).await
}

#[cfg(feature = "postgres")]
async fn cached_query_one(
    client: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<tokio_postgres::Row, tokio_postgres::Error> {
    let statement = client.prepare_cached(sql).await?;
    client.query_one(&statement, params).await
}

#[cfg(feature = "postgres")]
async fn cached_execute(
    client: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<u64, tokio_postgres::Error> {
    let statement = client.prepare_cached(sql).await?;
    client.execute(&statement, params).await
}

/// `CasExpectation::Absent` put: insert iff `path` is not an implicit directory
/// (no descendant in the half-open `[prefix/, prefix0)` range); the `ON CONFLICT
/// DO NOTHING` also blocks an explicit directory or existing file at `path`.
#[cfg(feature = "postgres")]
const PUT_ABSENT_SQL: &str = r#"
    INSERT INTO root_filesystem_entries
        (path, contents, is_dir, content_type, kind, indexed, version)
    SELECT $1, $2, FALSE, $3, $4, $5, 1
    WHERE NOT EXISTS (
        SELECT 1 FROM root_filesystem_entries
        WHERE path >= $6 AND path < $7
        LIMIT 1
    )
    ON CONFLICT (path) DO NOTHING
    "#;

/// `CasExpectation::Version` put: update the file row at the expected version,
/// rejecting an explicit directory (`is_dir = FALSE`) and — for parity with the
/// insert arms — any implicit-directory descendant.
#[cfg(feature = "postgres")]
const PUT_VERSION_SQL: &str = r#"
    UPDATE root_filesystem_entries
    SET contents = $1,
        content_type = $2,
        kind = $3,
        indexed = $4,
        version = version + 1,
        updated_at = NOW()
    WHERE path = $5 AND is_dir = FALSE AND version = $6
      AND NOT EXISTS (
          SELECT 1 FROM root_filesystem_entries AS child
          WHERE child.path >= $7 AND child.path < $8
          LIMIT 1
      )
    "#;

/// `CasExpectation::Any` put: upsert unless a descendant makes `path` an
/// implicit directory; the `ON CONFLICT` guard rejects an explicit directory.
/// `RETURNING version` removes the separate version read-back.
#[cfg(feature = "postgres")]
const PUT_ANY_SQL: &str = r#"
    INSERT INTO root_filesystem_entries
        (path, contents, is_dir, content_type, kind, indexed, version)
    SELECT $1, $2, FALSE, $3, $4, $5, 1
    WHERE NOT EXISTS (
        SELECT 1 FROM root_filesystem_entries
        WHERE path >= $6 AND path < $7
        LIMIT 1
    )
    ON CONFLICT (path) DO UPDATE SET
        contents = EXCLUDED.contents,
        content_type = EXCLUDED.content_type,
        kind = EXCLUDED.kind,
        indexed = EXCLUDED.indexed,
        version = root_filesystem_entries.version + 1,
        updated_at = NOW()
    WHERE root_filesystem_entries.is_dir = FALSE
    RETURNING version
    "#;

/// CAS put for the Postgres backend.
///
/// **Round-trip budget: 1 statement on the happy path.** The directory
/// invariant — reject writing a file where (a) an explicit directory exists at
/// the exact path or (b) an implicit-directory child exists — is folded into the
/// single write statement rather than issued as two separate `SELECT`
/// pre-checks. Each CAS arm guards the write with a `NOT EXISTS` descendant scan
/// over the half-open `[prefix/, prefix0)` range (rejecting implicit
/// directories). An explicit directory at `path` is rejected by the
/// `ON CONFLICT` clause in the INSERT arms and by `is_dir = FALSE` in the UPDATE
/// arm, so a successful put costs exactly one round-trip (previously:
/// exact-entry `SELECT` + child-scan `SELECT` + the write = three).
///
/// On the *rare* 0-row outcome we make follow-up reads
/// (`diagnose_put_failure`, up to three: `is_dir`, child-scan, then current
/// version) to reproduce the exact error the old pre-check and version read
/// would have produced: `directory_write_error` for a directory conflict,
/// `VersionMismatch` otherwise. The happy path never pays for it.
#[cfg(feature = "postgres")]
async fn postgres_put_with_client(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<RecordVersion, FilesystemError> {
    let indexed_json =
        serde_json::to_value(&entry.indexed).map_err(|_| FilesystemError::SerializeIndexed {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
        })?;
    let kind_str = entry.kind.as_ref().map(|k| k.as_str().to_string());
    let content_type_str = entry.content_type.as_str().to_string();
    let body = entry.body;
    let path_str = path.as_str();
    let (child_lower, child_upper) = descendant_path_range(path);

    match cas {
        CasExpectation::Absent => {
            // INSERT only when no descendant (implicit directory) exists; the
            // half-open range excludes `path` itself, so it never sees the row
            // being inserted. ON CONFLICT DO NOTHING also yields 0 rows when an
            // explicit directory already occupies `path` — disambiguated below.
            let rows = cached_execute(
                client,
                PUT_ABSENT_SQL,
                &[
                    &path_str,
                    &body,
                    &content_type_str,
                    &kind_str,
                    &indexed_json,
                    &child_lower,
                    &child_upper,
                ],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
            if rows == 0 {
                return Err(diagnose_put_failure(client, path, None).await?);
            }
            Ok(RecordVersion::from_backend(1))
        }
        CasExpectation::Version(expected) => {
            let expected_raw = record_version_to_i64(path, expected)?;
            // A `Version` CAS implies the file row already exists, so a child
            // under a file path is impossible by construction; the `NOT EXISTS`
            // descendant guard is carried for defense-in-depth parity with the
            // INSERT arms. `is_dir = FALSE` rejects an explicit directory.
            let rows = cached_execute(
                client,
                PUT_VERSION_SQL,
                &[
                    &body,
                    &content_type_str,
                    &kind_str,
                    &indexed_json,
                    &path_str,
                    &expected_raw,
                    &child_lower,
                    &child_upper,
                ],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
            if rows == 0 {
                return Err(diagnose_put_failure(client, path, Some(expected)).await?);
            }
            Ok(expected.next())
        }
        CasExpectation::Any => {
            // Upsert unless a descendant makes `path` an implicit directory; the
            // ON CONFLICT guard rejects an explicit directory at `path`. A 0-row
            // RETURNING is therefore always a directory conflict for `Any`.
            let row = cached_query_opt(
                client,
                PUT_ANY_SQL,
                &[
                    &path_str,
                    &body,
                    &content_type_str,
                    &kind_str,
                    &indexed_json,
                    &child_lower,
                    &child_upper,
                ],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
            let Some(row) = row else {
                return Err(directory_write_error(path.clone()));
            };
            let version: i64 = row.get("version");
            record_version_from_i64(path, version)
        }
    }
}

/// Diagnose the precise error for a CAS put that wrote 0 rows.
///
/// Only reached on the (rare) failure path. Distinguishes a directory conflict
/// (explicit directory at `path`, or an implicit-directory child) from a CAS
/// version mismatch — exactly the information the old pre-check + version read
/// produced, but issued only when the write actually failed.
///
/// The return type is `Result<FilesystemError, FilesystemError>` because the
/// diagnosis itself issues reads: the **`Ok`** value is the diagnosed error to
/// surface to the caller, and the **`Err`** value is a backend failure that
/// occurred *while* diagnosing. Call sites therefore wrap the result as
/// `Err(diagnose_put_failure(..).await?)` — the `?` propagates a diagnosis-time
/// backend error, and the `Err(..)` surfaces the diagnosed error.
///
/// **Classification is best-effort: it only reflects what these follow-up reads
/// observe, not the exact snapshot that failed the write.** This diagnosis runs
/// purely to pick *which error variant* to report on an already-failed write —
/// it never writes, so it cannot itself corrupt state. A concurrent writer
/// mutating the directory/child row between the failed write and these reads can
/// flip the variant; the reported variant reflects the reads' observed state,
/// which is sufficient for the canonical CAS-retry caller's next attempt. We
/// deliberately do not wrap the write + diagnosis in a transaction or higher
/// isolation: that would add a serialization-retry error mode (and cost) to a
/// rare path for no gain in *this* classification.
///
/// Scope note: the put write statement evaluates its directory guard against a
/// single snapshot, which removes the old separate-pre-check-then-write TOCTOUs
/// for the *same* path. It does NOT serialize concurrent writes to *different*
/// parent/child paths (e.g. `put(/a)` racing `put(/a/b)`), so the file-vs-dir
/// invariant can still be violated under cross-path write-skew. That gap is
/// pre-existing (the old 3-read pre-check had a wider window) and tracked
/// separately — see `put_statements_are_single_round_trip` for the boundary of
/// what the single-statement guard does and does not guarantee.
#[cfg(feature = "postgres")]
async fn diagnose_put_failure(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
    expected: Option<RecordVersion>,
) -> Result<FilesystemError, FilesystemError> {
    if postgres_is_dir_with_client(client, path).await?
        || postgres_has_child_entry_with_client(client, path).await?
    {
        return Ok(directory_write_error(path.clone()));
    }
    let found = postgres_current_version_with_client(client, path).await?;
    Ok(FilesystemError::VersionMismatch {
        path: path.clone(),
        expected,
        found,
    })
}

#[cfg(feature = "postgres")]
async fn postgres_get_with_client(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
) -> Result<Option<VersionedEntry>, FilesystemError> {
    let row = cached_query_opt(
        client,
        r#"
            SELECT contents, is_dir, content_type, kind, indexed, version
            FROM root_filesystem_entries
            WHERE path = $1
            "#,
        &[&path.as_str()],
    )
    .await
    .map_err(|error| db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let is_dir: bool = row.get("is_dir");
    if is_dir {
        return Ok(None);
    }
    let body: Vec<u8> = row.get("contents");
    let content_type_raw: String = row.get("content_type");
    let kind_raw: Option<String> = row.get("kind");
    let indexed_value: serde_json::Value = row.get("indexed");
    let version_raw: i64 = row.get("version");
    let entry = build_entry(path, body, content_type_raw, kind_raw, indexed_value)?;
    Ok(Some(VersionedEntry {
        path: path.clone(),
        entry,
        version: record_version_from_i64(path, version_raw)?,
    }))
}

#[cfg(feature = "postgres")]
async fn postgres_delete_with_client(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    let (prefix_lower, prefix_upper) = descendant_path_range(path);
    let deleted = cached_execute(
        client,
        "DELETE FROM root_filesystem_entries WHERE path = $1 OR (path >= $2 AND path < $3)",
        &[&path.as_str(), &prefix_lower, &prefix_upper],
    )
    .await
    .map_err(|error| db_error(path.clone(), FilesystemOperation::Delete, error))?;
    if deleted == 0 {
        return Err(not_found(path.clone(), FilesystemOperation::Delete));
    }
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_current_version_with_client(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
) -> Result<Option<RecordVersion>, FilesystemError> {
    let row = cached_query_opt(
        client,
        "SELECT version FROM root_filesystem_entries WHERE path = $1 AND is_dir = FALSE",
        &[&path.as_str()],
    )
    .await
    .map_err(|error| db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    row.map(|row| {
        let version: i64 = row.get("version");
        record_version_from_i64(path, version)
    })
    .transpose()
}

/// Whether an explicit directory row exists at the exact `path`.
///
/// Used only on the rare CAS-put failure path. Selects the `is_dir` flag alone
/// and never reads `OCTET_LENGTH(contents)`, so it does not touch the
/// (potentially TOAST'd) body to answer a question that only needs one boolean.
#[cfg(feature = "postgres")]
async fn postgres_is_dir_with_client(
    client: &deadpool_postgres::Object,
    path: &VirtualPath,
) -> Result<bool, FilesystemError> {
    let row = cached_query_opt(
        client,
        "SELECT is_dir FROM root_filesystem_entries WHERE path = $1",
        &[&path.as_str()],
    )
    .await
    .map_err(|error| db_error(path.clone(), FilesystemOperation::Stat, error))?;
    Ok(row.is_some_and(|row| row.get::<_, bool>("is_dir")))
}

#[cfg(feature = "postgres")]
async fn postgres_has_child_entry_with_client(
    client: &deadpool_postgres::Object,
    parent: &VirtualPath,
) -> Result<bool, FilesystemError> {
    let (prefix_lower, prefix_upper) = descendant_path_range(parent);
    let row = cached_query_opt(
        client,
        "SELECT 1 FROM root_filesystem_entries WHERE path >= $1 AND path < $2 LIMIT 1",
        &[&prefix_lower, &prefix_upper],
    )
    .await
    .map_err(|error| db_error(parent.clone(), FilesystemOperation::Stat, error))?;
    Ok(row.is_some())
}

#[cfg(feature = "postgres")]
fn descendant_path_range(path: &VirtualPath) -> (String, String) {
    let prefix = path.as_str().trim_end_matches('/');
    // Descendants share the literal "{prefix}/" component boundary. The
    // exclusive upper bound "{prefix}0" works because '/' sorts before '0'
    // in the normalized virtual path alphabet used by these storage paths.
    (format!("{prefix}/"), format!("{prefix}0"))
}

/// Translate a [`Filter`] tree into a postgres WHERE-clause fragment.
/// Bound parameters use `$N` placeholders sized from `params.len() + 1`.
///
/// PR #3661 fixes carried over from the libsql translator:
/// - `Filter::All` emits `TRUE`; empty `And` → `TRUE`, empty `Or` →
///   `FALSE` (matching in-memory `all`/`any` semantics).
/// - `Filter::Range` on `IndexValue::I64` bounds casts both sides to
///   `BIGINT` so the comparison is numeric, not lexicographic on text.
#[cfg(feature = "postgres")]
fn translate_filter(
    path: &VirtualPath,
    filter: &Filter,
    out: &mut String,
    params: &mut Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
) -> Result<(), FilesystemError> {
    match filter {
        Filter::All => {
            out.push_str("TRUE");
            Ok(())
        }
        Filter::Eq { key, value } => {
            let placeholder = bind_index_value(path, value, params)?;
            out.push_str(&format!("(indexed->>'{}' = ${placeholder})", key.as_str()));
            Ok(())
        }
        Filter::PrefixOn { key, value } => {
            let IndexValue::Text(prefix_value) = value else {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                });
            };
            let escaped = escape_like_literal(prefix_value);
            params.push(Box::new(format!("{escaped}%")));
            out.push_str(&format!(
                "(indexed->>'{}' LIKE ${} ESCAPE '!')",
                key.as_str(),
                params.len()
            ));
            Ok(())
        }
        Filter::Range { key, lo, hi } => {
            // Mixed-variant bounds have no meaningful BETWEEN. Reject rather
            // than fall through to a lexicographic-on-text comparison that
            // silently produces wrong results. Matches the in-memory
            // backend's `discriminant(lo) == discriminant(hi)` requirement.
            if std::mem::discriminant(lo) != std::mem::discriminant(hi) {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                });
            }
            // PR #3661 reviewer fix: when both bounds are `I64`, cast both
            // the extracted JSON text and bound params to `BIGINT` so the
            // BETWEEN comparison is numeric. Otherwise `'2' BETWEEN '10'
            // AND '99'` would compare lexicographically and miss values.
            //
            // PR #3659 review fix: guard each cast with a `jsonb_typeof`
            // check so a row whose stored value at `'{key}'` is a different
            // variant (e.g. text under a numeric range) is filtered out
            // BEFORE the cast — otherwise one stored text value can fail
            // the whole query with a `bigint` cast error.
            match (lo, hi) {
                (IndexValue::I64(lo_val), IndexValue::I64(hi_val)) => {
                    params.push(Box::new(*lo_val));
                    let lo_idx = params.len();
                    params.push(Box::new(*hi_val));
                    let hi_idx = params.len();
                    out.push_str(&format!(
                        "(jsonb_typeof(indexed->'{}') = 'number' \
                         AND (indexed->>'{}')::bigint BETWEEN ${lo_idx} AND ${hi_idx})",
                        key.as_str(),
                        key.as_str(),
                    ));
                }
                _ => {
                    let lo_idx = bind_index_value(path, lo, params)?;
                    let hi_idx = bind_index_value(path, hi, params)?;
                    let expected_json_type = index_value_jsonb_typeof(lo);
                    out.push_str(&format!(
                        "(jsonb_typeof(indexed->'{}') = '{expected_json_type}' \
                         AND indexed->>'{}' BETWEEN ${lo_idx} AND ${hi_idx})",
                        key.as_str(),
                        key.as_str(),
                    ));
                }
            }
            Ok(())
        }
        Filter::Fts { key, query } => {
            // `plainto_tsquery` is the user-input-safe parser; we never
            // splice the user query into SQL. Match against an expression
            // identical to the `to_tsvector(...)` used by the GIN index in
            // ensure_index so the planner can use it.
            params.push(Box::new(query.clone()));
            out.push_str(&format!(
                "(to_tsvector('english', COALESCE(indexed->>'{}', '')) @@ plainto_tsquery('english', ${}))",
                key.as_str(),
                params.len()
            ));
            Ok(())
        }
        Filter::VectorNearest { .. } => Err(FilesystemError::Unsupported {
            // Same reason as libsql: VectorNearest is a ranking operation
            // and is evaluated at the top-level `query` method, not as a
            // WHERE-clause predicate. Nested usage is unsupported.
            path: path.clone(),
            operation: FilesystemOperation::Query,
        }),
        Filter::And(children) => translate_compound(path, children, " AND ", "TRUE", out, params),
        Filter::Or(children) => translate_compound(path, children, " OR ", "FALSE", out, params),
    }
}

#[cfg(feature = "postgres")]
fn translate_compound(
    path: &VirtualPath,
    children: &[Filter],
    joiner: &str,
    empty_identity: &str,
    out: &mut String,
    params: &mut Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
) -> Result<(), FilesystemError> {
    if children.is_empty() {
        out.push_str(empty_identity);
        return Ok(());
    }
    out.push('(');
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            out.push_str(joiner);
        }
        translate_filter(path, child, out, params)?;
    }
    out.push(')');
    Ok(())
}

/// Maps an [`IndexValue`] variant to its Postgres `jsonb_typeof` string.
/// Used to guard `Filter::Range` so cross-variant stored values are filtered
/// out before any cast/comparison (PR #3659 review fix). Postgres returns:
/// `"string"` / `"number"` / `"boolean"` / `"null"` / `"object"` / `"array"`.
#[cfg(feature = "postgres")]
fn index_value_jsonb_typeof(value: &IndexValue) -> &'static str {
    match value {
        IndexValue::Text(_) | IndexValue::Bytes(_) => "string",
        IndexValue::I64(_) => "number",
        IndexValue::Bool(_) => "boolean",
    }
}

#[cfg(feature = "postgres")]
fn bind_index_value(
    path: &VirtualPath,
    value: &IndexValue,
    params: &mut Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
) -> Result<usize, FilesystemError> {
    // `indexed->>'key'` returns text regardless of the underlying JSON type,
    // so we bind every supported variant as text. This keeps the index
    // (which is also an expression on the text form) usable for all three
    // variants without dialect branches.
    let bound: Box<dyn tokio_postgres::types::ToSql + Sync + Send> = match value {
        IndexValue::Text(s) => Box::new(s.clone()),
        IndexValue::I64(n) => Box::new(n.to_string()),
        IndexValue::Bool(b) => Box::new(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        IndexValue::Bytes(_) => {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Query,
            });
        }
    };
    params.push(bound);
    Ok(params.len())
}

#[cfg(feature = "postgres")]
fn build_entry(
    path: &VirtualPath,
    body: Vec<u8>,
    content_type_raw: String,
    kind_raw: Option<String>,
    indexed_value: serde_json::Value,
) -> Result<Entry, FilesystemError> {
    let content_type = ContentType::new(content_type_raw).map_err(FilesystemError::Contract)?;
    let kind = kind_raw
        .map(RecordKind::new)
        .transpose()
        .map_err(FilesystemError::Contract)?;
    let indexed: BTreeMap<IndexKey, IndexValue> = if indexed_value.is_null() {
        BTreeMap::new()
    } else {
        serde_json::from_value(indexed_value).map_err(|_| FilesystemError::DeserializeIndexed {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
        })?
    };
    Ok(Entry {
        body,
        content_type,
        kind,
        indexed,
    })
}

#[cfg(feature = "postgres")]
fn seq_no_from_i64(
    path: &VirtualPath,
    raw: i64,
    operation: FilesystemOperation,
) -> Result<SeqNo, FilesystemError> {
    u64::try_from(raw)
        .map(SeqNo::from_backend)
        .map_err(|_| FilesystemError::Backend {
            path: path.clone(),
            operation,
            reason: format!("event seq {raw} is not representable"),
        })
}

#[cfg(feature = "postgres")]
fn backend_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    reason: impl Into<String>,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: reason.into(),
    }
}

#[cfg(feature = "postgres")]
const POSTGRES_ROOT_FILESYSTEM_SCHEMA: &str = concat!(
    include_str!("../../../migrations/V26__root_filesystem_entries.sql"),
    "\n",
    include_str!("../../../migrations/V27__root_filesystem_entries_directories.sql"),
    "\n",
    include_str!("../../../migrations/V28__root_filesystem_records.sql"),
    "\n",
    include_str!("../../../migrations/V29__root_filesystem_index_specs.sql"),
    "\n",
    include_str!("../../../migrations/V30__root_filesystem_events.sql"),
    "\n",
    include_str!("../../../migrations/V31__root_filesystem_path_collation.sql"),
);

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    #[test]
    fn postgres_migration_connect_backoff_is_capped() {
        assert_eq!(
            postgres_migration_connect_backoff(0),
            POSTGRES_MIGRATION_CONNECT_INITIAL_BACKOFF
        );
        assert_eq!(
            postgres_migration_connect_backoff(20),
            POSTGRES_MIGRATION_CONNECT_MAX_BACKOFF
        );
    }

    /// Returns the number of top-level statements in a SQL string by counting
    /// statement terminators (`;`) that are not inside a quoted literal. The
    /// put statements use no string literals containing `;`, so a simple
    /// non-empty trailing-segment count is exact here.
    fn top_level_statement_count(sql: &str) -> usize {
        sql.split(';').filter(|s| !s.trim().is_empty()).count()
    }

    /// The CAS put round-trip fix: each `CasExpectation` arm must issue exactly
    /// ONE statement. Before the fix, a put ran three round-trips — an
    /// exact-entry `SELECT`, a child-scan `SELECT`, then the write. The
    /// directory invariant is now folded into the single write statement, so a
    /// successful put costs one round-trip. This guards against a regression
    /// re-splitting the pre-check back out into separate queries.
    ///
    /// What this pins, precisely: the directory guard is evaluated in the *same*
    /// statement that performs the write (one round-trip), against one snapshot —
    /// rather than as a separate pre-check `SELECT` followed by the write. That
    /// is the property the PR delivers, and keeping the guard folded in is what
    /// makes the rare-path error classification in `diagnose_put_failure`
    /// coherent (same-snapshot guard, no separate-pre-check window for the same
    /// path).
    ///
    /// What this does NOT pin — read before relying on it: single-statement
    /// evaluation is not isolation. `NOT EXISTS` is a snapshot predicate with no
    /// range lock, so concurrent writes to *different* parent/child paths
    /// (`put(/a)` racing `put(/a/b)`) can each pass their own guard and both
    /// commit, leaving `/a` a file with a descendant — the file-vs-directory
    /// invariant violated by write-skew. This gap is pre-existing (the old
    /// 3-read pre-check had a wider window) and closing it requires path-prefix
    /// serialization across all backends (advisory lock / SERIALIZABLE + retry),
    /// which is out of scope for a round-trip optimization. Tracked separately;
    /// do not read this test as a concurrency guarantee.
    #[test]
    fn put_statements_are_single_round_trip() {
        for (name, sql) in [
            ("absent", PUT_ABSENT_SQL),
            ("version", PUT_VERSION_SQL),
            ("any", PUT_ANY_SQL),
        ] {
            assert_eq!(
                top_level_statement_count(sql),
                1,
                "{name} put must be a single statement (one round-trip); a split \
                 back into a separate pre-check + write reintroduces the same-path \
                 check-then-write window this PR removed. Got: {sql}"
            );
        }
    }

    /// Each put statement must carry the folded directory invariant guard so it
    /// rejects writing a file over a directory without a separate pre-check
    /// round-trip. Every arm guards implicit directories with a `NOT EXISTS`
    /// descendant scan. Explicit directories are rejected differently per arm:
    /// the INSERT arms (`PUT_ABSENT_SQL`, `PUT_ANY_SQL`) rely on `ON CONFLICT`
    /// (the existing directory row collides on the `path` primary key), while
    /// the UPDATE arm (`PUT_VERSION_SQL`) and the `PUT_ANY_SQL` conflict clause
    /// use an explicit `is_dir = FALSE` predicate.
    #[test]
    fn put_statements_fold_in_directory_guard() {
        // Implicit-directory (descendant) guard — every arm.
        assert!(PUT_ABSENT_SQL.contains("NOT EXISTS"));
        assert!(PUT_VERSION_SQL.contains("NOT EXISTS"));
        assert!(PUT_ANY_SQL.contains("NOT EXISTS"));
        // Explicit-directory: INSERT arms reject via ON CONFLICT on the path PK.
        assert!(PUT_ABSENT_SQL.contains("ON CONFLICT (path)"));
        assert!(PUT_ANY_SQL.contains("ON CONFLICT (path)"));
        // Explicit-directory: the UPDATE arm and the upsert conflict clause use
        // an explicit is_dir = FALSE predicate.
        assert!(PUT_VERSION_SQL.contains("is_dir = FALSE"));
        assert!(PUT_ANY_SQL.contains("is_dir = FALSE"));
    }

    /// The descendant range is the half-open `[prefix/, prefix0)` band that the
    /// folded `NOT EXISTS` child guard scans. A wrong bound would silently
    /// mis-scope the directory invariant, so pin the exact bytes and the
    /// sort-order invariant (`'/'` < real children < `'0'`).
    #[test]
    fn descendant_path_range_is_half_open_child_band() {
        let path = VirtualPath::new("/secrets/a/b").unwrap();
        let (lower, upper) = descendant_path_range(&path);
        assert_eq!(lower, "/secrets/a/b/");
        assert_eq!(upper, "/secrets/a/b0");
        let (lower, upper) = (lower.as_str(), upper.as_str());
        assert!(lower < upper);
        // A real descendant sorts inside the band; the path itself and a
        // sibling sharing the prefix do not.
        assert!("/secrets/a/b/child" >= lower && "/secrets/a/b/child" < upper);
        assert!("/secrets/a/b" < lower); // the path itself is excluded
        assert!("/secrets/a/bb" >= upper); // prefix-sharing sibling excluded
    }
}
