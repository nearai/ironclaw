use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::EventRecord;
use crate::db::{
    child_path_like_pattern, db_error, direct_children, directory_append_error,
    directory_write_error, escape_like_literal, escape_like_with_trailing_wildcard, is_not_found,
    not_found, record_version_from_i64, sql_index_name, system_time_from_unix_seconds,
    valid_engine_path, virtual_path_prefixes,
};
use crate::{
    BackendCapabilities, Capability, CasExpectation, ContentType, DirEntry, Entry, FileStat,
    FileType, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind, IndexSpec,
    IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, SeqNo, VersionedEntry,
};

#[cfg(feature = "postgres")]
/// PostgreSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct PostgresRootFilesystem {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        client
            .batch_execute(POSTGRES_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| {
                db_error(
                    valid_engine_path(),
                    FilesystemOperation::CreateDirAll,
                    error,
                )
            })
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, FilesystemError> {
        self.pool
            .get()
            .await
            .map_err(|error| FilesystemError::Backend {
                path: valid_engine_path(),
                operation: FilesystemOperation::Stat,
                reason: error.to_string(),
            })
    }
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
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let client = self.client().await?;
        if matches!(
            self.exact_entry_with_client(&client, path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry_with_client(&client, path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        let indexed_json = serde_json::to_value(&entry.indexed).map_err(|_| {
            FilesystemError::SerializeIndexed {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            }
        })?;
        let kind_str = entry.kind.as_ref().map(|k| k.as_str().to_string());
        let content_type_str = entry.content_type.as_str().to_string();
        let body = entry.body;
        let path_str = path.as_str();

        match cas {
            CasExpectation::Absent => {
                let rows = client
                    .execute(
                        r#"
                        INSERT INTO root_filesystem_entries
                            (path, contents, is_dir, content_type, kind, indexed, version)
                        VALUES ($1, $2, FALSE, $3, $4, $5, 1)
                        ON CONFLICT (path) DO NOTHING
                        "#,
                        &[
                            &path_str,
                            &body,
                            &content_type_str,
                            &kind_str,
                            &indexed_json,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                if rows == 0 {
                    let found = self.current_version_with_client(&client, path).await?;
                    return Err(FilesystemError::VersionMismatch {
                        path: path.clone(),
                        expected: None,
                        found,
                    });
                }
                Ok(RecordVersion::from_backend(1))
            }
            CasExpectation::Version(expected) => {
                let expected_raw = expected.get() as i64;
                let rows = client
                    .execute(
                        r#"
                        UPDATE root_filesystem_entries
                        SET contents = $1,
                            content_type = $2,
                            kind = $3,
                            indexed = $4,
                            version = version + 1,
                            updated_at = NOW()
                        WHERE path = $5 AND is_dir = FALSE AND version = $6
                        "#,
                        &[
                            &body,
                            &content_type_str,
                            &kind_str,
                            &indexed_json,
                            &path_str,
                            &expected_raw,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                if rows == 0 {
                    let found = self.current_version_with_client(&client, path).await?;
                    return Err(FilesystemError::VersionMismatch {
                        path: path.clone(),
                        expected: Some(expected),
                        found,
                    });
                }
                Ok(expected.next())
            }
            CasExpectation::Any => {
                let row = client
                    .query_opt(
                        r#"
                        INSERT INTO root_filesystem_entries
                            (path, contents, is_dir, content_type, kind, indexed, version)
                        VALUES ($1, $2, FALSE, $3, $4, $5, 1)
                        ON CONFLICT (path) DO UPDATE SET
                            contents = EXCLUDED.contents,
                            content_type = EXCLUDED.content_type,
                            kind = EXCLUDED.kind,
                            indexed = EXCLUDED.indexed,
                            version = root_filesystem_entries.version + 1,
                            updated_at = NOW()
                        WHERE root_filesystem_entries.is_dir = FALSE
                        RETURNING version
                        "#,
                        &[
                            &path_str,
                            &body,
                            &content_type_str,
                            &kind_str,
                            &indexed_json,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                let Some(row) = row else {
                    return Err(directory_write_error(path.clone()));
                };
                let version: i64 = row.get("version");
                record_version_from_i64(path, version)
            }
        }
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
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
        client
            .execute(
                "INSERT INTO root_filesystem_index_specs (prefix, name, keys, kind) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (prefix, name) DO NOTHING",
                &[&path.as_str(), &spec.name.as_str(), &keys_json, &kind_str],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::EnsureIndex, error))?;

        let row = client
            .query_opt(
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
                let ddl = format!(
                    "CREATE INDEX IF NOT EXISTS {index_name} ON root_filesystem_entries \
                     USING GIN (to_tsvector('english', COALESCE(indexed->>'{fts_key}', '')))"
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
        let prefix_pattern = escape_like_with_trailing_wildcard(&format!("{}/%", path.as_str()));
        params.push(Box::new(path_str));
        params.push(Box::new(prefix_pattern));

        let mut conditions = String::new();
        translate_filter(path, filter, &mut conditions, &mut params)?;

        let mut sql = String::from(
            "SELECT path, contents, content_type, kind, indexed, version \
             FROM root_filesystem_entries \
             WHERE is_dir = FALSE AND (path = $1 OR path LIKE $2 ESCAPE '!')",
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
        params.push(Box::new(i64::from(page.limit.min(Page::MAX_LIMIT))));
        params.push(Box::new(page.offset as i64));

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
        let row = client
            .query_opt(
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
        let rows = client
            .execute(
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
        client
            .execute(
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
        let child_pattern = child_path_like_pattern(path);
        let deleted = client
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = $1 OR path LIKE $2 ESCAPE '!'",
                &[&path.as_str(), &child_pattern],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Delete, error))?;
        if deleted == 0 {
            return Err(not_found(path.clone(), FilesystemOperation::Delete));
        }
        Ok(())
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO root_filesystem_events (path, payload)
                VALUES ($1, $2)
                RETURNING id
                "#,
                &[&path.as_str(), &payload],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        let id: i64 = row.get("id");
        seq_no_from_i64(path, id)
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let client = self.client().await?;
        let from_raw = i64::try_from(from.get()).map_err(|_| {
            backend_error(
                path.clone(),
                FilesystemOperation::Tail,
                "tail cursor exceeds i64",
            )
        })?;
        let rows = client
            .query(
                r#"
                SELECT id, payload
                FROM root_filesystem_events
                WHERE path = $1 AND id > $2
                ORDER BY id ASC
                "#,
                &[&path.as_str(), &from_raw],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Tail, error))?;
        rows.into_iter()
            .map(|row| {
                let id: i64 = row.get("id");
                let payload: Vec<u8> = row.get("payload");
                Ok(EventRecord {
                    seq: seq_no_from_i64(path, id)?,
                    payload,
                })
            })
            .collect()
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
        client: &tokio_postgres::Client,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType, Option<std::time::SystemTime>)>, FilesystemError> {
        let row = client
            .query_opt(
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
        client: &tokio_postgres::Client,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let pattern = child_path_like_pattern(parent);
        let rows = client
            .query(
                "SELECT path, OCTET_LENGTH(contents)::bigint AS len, is_dir FROM root_filesystem_entries WHERE path LIKE $1 ESCAPE '!' ORDER BY path",
                &[&pattern],
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
        client: &tokio_postgres::Client,
        parent: &VirtualPath,
    ) -> Result<bool, FilesystemError> {
        let pattern = child_path_like_pattern(parent);
        let row = client
            .query_opt(
                "SELECT 1 FROM root_filesystem_entries WHERE path LIKE $1 ESCAPE '!' LIMIT 1",
                &[&pattern],
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
        let prefix_pattern = escape_like_with_trailing_wildcard(&format!("{}/%", path.as_str()));
        let rows = client
            .query(
                "SELECT path, indexed, version \
                 FROM root_filesystem_entries \
                 WHERE is_dir = FALSE AND (path = $1 OR path LIKE $2 ESCAPE '!')",
                &[&path.as_str(), &prefix_pattern],
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
        ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit as usize);
        // Release the client so each `get()` below claims its own
        // pooled connection rather than serializing through one.
        drop(client);
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

    async fn current_version_with_client(
        &self,
        client: &tokio_postgres::Client,
        path: &VirtualPath,
    ) -> Result<Option<RecordVersion>, FilesystemError> {
        let row = client
            .query_opt(
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
fn decode_embedding_blob(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

#[cfg(feature = "postgres")]
fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return None;
    }
    let score = dot / (left_norm.sqrt() * right_norm.sqrt());
    if score.is_finite() { Some(score) } else { None }
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
fn seq_no_from_i64(path: &VirtualPath, raw: i64) -> Result<SeqNo, FilesystemError> {
    u64::try_from(raw)
        .map(SeqNo::from_backend)
        .map_err(|_| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Tail,
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
);
