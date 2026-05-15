use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::EventRecord;
use crate::db::{
    child_path_like_pattern, direct_children, directory_append_error, directory_write_error,
    escape_like_literal, escape_like_with_trailing_wildcard, is_not_found, libsql_db_error,
    not_found, record_version_from_i64, sql_index_name, system_time_from_unix_seconds,
    valid_engine_path, virtual_path_prefixes,
};
use crate::{
    BackendCapabilities, Capability, CasExpectation, ContentType, DirEntry, Entry, FileStat,
    FileType, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind, IndexSpec,
    IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, SeqNo, VersionedEntry,
};

#[cfg(feature = "libsql")]
/// libSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct LibSqlRootFilesystem {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| {
                libsql_db_error(
                    valid_engine_path(),
                    FilesystemOperation::CreateDirAll,
                    error,
                )
            })?;
        ensure_libsql_root_is_dir_column(&conn).await?;
        ensure_libsql_records_columns(&conn).await?;
        ensure_libsql_index_specs_table(&conn).await?;
        ensure_libsql_events_table(&conn).await?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, FilesystemError> {
        let conn = self
            .db
            .connect()
            .map_err(|error| FilesystemError::Backend {
                path: valid_engine_path(),
                operation: FilesystemOperation::Stat,
                reason: error.to_string(),
            })?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|error| {
                libsql_db_error(valid_engine_path(), FilesystemOperation::Stat, error)
            })?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl RootFilesystem for LibSqlRootFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        // sql_typical covers read/write/append/list/stat/delete/records/query
        // /IndexExact/IndexPrefix/CAS. The append/tail backing table is in
        // place so Events is on; FTS5 is built into libSQL and a brute-force
        // cosine ranker for vectors is implemented in Rust, so IndexFts and
        // IndexVector are advertised here too.
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
        // Reject writes that would clobber a directory or a path that has
        // children (mirrors `write_file` semantics so legacy and new ops
        // stay consistent).
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry(path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        let indexed_json = serde_json::to_string(&entry.indexed).map_err(|_| {
            FilesystemError::SerializeIndexed {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            }
        })?;
        let kind_str = entry.kind.as_ref().map(|k| k.as_str().to_string());
        let content_type_str = entry.content_type.as_str().to_string();
        let body = entry.body;

        match cas {
            CasExpectation::Absent => {
                let conn = self.connect().await?;
                let rows = conn
                    .execute(
                        r#"
                        INSERT INTO root_filesystem_entries
                            (path, contents, is_dir, content_type, kind, indexed, version, updated_at)
                        VALUES (?1, ?2, 0, ?3, ?4, ?5, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                        ON CONFLICT (path) DO NOTHING
                        "#,
                        libsql::params![
                            path.as_str(),
                            libsql::Value::Blob(body),
                            content_type_str,
                            kind_str,
                            indexed_json,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                if rows == 0 {
                    let found = self.current_version(path).await?;
                    return Err(FilesystemError::VersionMismatch {
                        path: path.clone(),
                        expected: None,
                        found,
                    });
                }
                Ok(RecordVersion::from_backend(1))
            }
            CasExpectation::Version(expected) => {
                let conn = self.connect().await?;
                let expected_raw = expected.get() as i64;
                let rows = conn
                    .execute(
                        r#"
                        UPDATE root_filesystem_entries
                        SET contents = ?1,
                            content_type = ?2,
                            kind = ?3,
                            indexed = ?4,
                            version = version + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE path = ?5 AND is_dir = 0 AND version = ?6
                        "#,
                        libsql::params![
                            libsql::Value::Blob(body),
                            content_type_str,
                            kind_str,
                            indexed_json,
                            path.as_str(),
                            expected_raw,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                if rows == 0 {
                    let found = self.current_version(path).await?;
                    return Err(FilesystemError::VersionMismatch {
                        path: path.clone(),
                        expected: Some(expected),
                        found,
                    });
                }
                Ok(expected.next())
            }
            CasExpectation::Any => {
                let conn = self.connect().await?;
                let rows = conn
                    .execute(
                        r#"
                        INSERT INTO root_filesystem_entries
                            (path, contents, is_dir, content_type, kind, indexed, version, updated_at)
                        VALUES (?1, ?2, 0, ?3, ?4, ?5, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                        ON CONFLICT (path) DO UPDATE SET
                            contents = excluded.contents,
                            content_type = excluded.content_type,
                            kind = excluded.kind,
                            indexed = excluded.indexed,
                            version = root_filesystem_entries.version + 1,
                            updated_at = excluded.updated_at
                        WHERE root_filesystem_entries.is_dir = 0
                        "#,
                        libsql::params![
                            path.as_str(),
                            libsql::Value::Blob(body),
                            content_type_str,
                            kind_str,
                            indexed_json,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
                    })?;
                if rows == 0 {
                    return Err(directory_write_error(path.clone()));
                }
                let version =
                    self.current_version(path)
                        .await?
                        .ok_or_else(|| FilesystemError::Backend {
                            path: path.clone(),
                            operation: FilesystemOperation::WriteFile,
                            reason: "put succeeded but version lookup found no row".to_string(),
                        })?;
                Ok(version)
            }
        }
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT contents, is_dir, content_type, kind, indexed, version
                FROM root_filesystem_entries
                WHERE path = ?1
                "#,
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?
        else {
            return Ok(None);
        };
        let is_dir: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if is_dir != 0 {
            // Directories are not addressable as Entries.
            return Ok(None);
        }
        let body: Vec<u8> = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let content_type_raw: String = row
            .get(2)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let kind_raw: Option<String> = row.get(3).ok();
        let indexed_raw: String = row
            .get(4)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let version_raw: i64 = row
            .get(5)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let entry = build_entry(path, body, content_type_raw, kind_raw, indexed_raw)?;
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
        // Exact/Prefix create a SQLite expression index over the indexed JSON
        // projection. Fts creates an FTS5 virtual table mirroring the
        // indexed text key on this prefix, kept in sync by AFTER INSERT/
        // UPDATE/DELETE triggers. Vector { dim } records the dimension in
        // the spec catalog; storage uses IndexValue::Bytes in the indexed
        // projection and brute-force cosine on query (the libSQL vector
        // extension is unreliable across builds).
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
        let keys_json = serde_json::to_string(
            &spec
                .keys
                .iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| FilesystemError::SerializeIndexed {
            path: path.clone(),
            operation: FilesystemOperation::EnsureIndex,
        })?;

        let conn = self.connect().await?;
        // PR #3661 reviewer fix: the prior SELECT-then-INSERT was racey.
        // Two processes declaring the same spec concurrently could both
        // miss the row and then one would hit a unique-constraint backend
        // error instead of getting the promised idempotent success.
        //
        // Fix: INSERT ... ON CONFLICT DO NOTHING in a single round-trip,
        // then read back the canonical row and compare. If the stored
        // spec matches ours we're idempotent; if it differs we surface
        // IndexConflict.
        conn.execute(
            "INSERT INTO root_filesystem_index_specs (prefix, name, keys, kind) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (prefix, name) DO NOTHING",
            libsql::params![
                path.as_str(),
                spec.name.as_str(),
                keys_json.clone(),
                kind_str.clone(),
            ],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error))?;

        // Read back what's there and validate it matches.
        let mut rows = conn
            .query(
                "SELECT keys, kind FROM root_filesystem_index_specs WHERE prefix = ?1 AND name = ?2",
                libsql::params![path.as_str(), spec.name.as_str()],
            )
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
            })?;
        let row = rows
            .next()
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
            })?
            .ok_or_else(|| FilesystemError::IndexSpecMissingAfterUpsert {
                path: path.clone(),
                name: spec.name.clone(),
            })?;
        let existing_keys: String = row.get(0).map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
        })?;
        let existing_kind: String = row.get(1).map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
        })?;
        if existing_keys != keys_json || existing_kind != kind_str {
            return Err(FilesystemError::IndexConflict {
                path: path.clone(),
                name: spec.name.clone(),
                reason: crate::IndexConflictReason::SpecMismatch,
            });
        }
        drop(rows);

        let index_name = sql_index_name(path.as_str(), spec.name.as_str());
        match &spec.kind {
            IndexKind::Exact | IndexKind::Prefix => {
                let expressions: Vec<String> = spec
                    .keys
                    .iter()
                    .map(|k| format!("json_extract(indexed, '$.{}')", k.as_str()))
                    .collect();
                let ddl = format!(
                    "CREATE INDEX IF NOT EXISTS {index_name} ON root_filesystem_entries ({})",
                    expressions.join(", ")
                );
                conn.execute(&ddl, ()).await.map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
            }
            IndexKind::Fts => {
                // FTS indexes need exactly one text key; the FTS5 vtable has
                // one shadow column per indexed key, but the filter surface
                // currently exposes Fts { key, query } as single-keyed.
                if spec.keys.len() != 1 {
                    return Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    });
                }
                let fts_key = spec.keys[0].as_str();
                let path_prefix = path.as_str();
                let trailing_prefix = format!("{}/", path_prefix.trim_end_matches('/'));
                let trailing_pattern =
                    escape_like_with_trailing_wildcard(&format!("{trailing_prefix}%"));
                let exact_path_lit = path_prefix.replace('\'', "''");
                let trailing_pattern_lit = trailing_pattern.replace('\'', "''");
                // FTS5 vtable: stores (path, text). We mirror per-mount-
                // prefix so different prefixes (with different keys) don't
                // collide on a single FTS table.
                let fts_table = format!("{index_name}_fts");
                let create_vtab = format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {fts_table} \
                     USING fts5(path UNINDEXED, content)"
                );
                conn.execute(&create_vtab, ()).await.map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
                // Triggers keep the FTS table in sync with entries whose
                // path is within this prefix. They extract the indexed
                // text via json_extract; non-text values fall through as
                // empty strings (FTS5 won't match them).
                let trigger_insert = format!(
                    "CREATE TRIGGER IF NOT EXISTS {index_name}_ai \
                     AFTER INSERT ON root_filesystem_entries \
                     WHEN new.is_dir = 0 \
                       AND (new.path = '{exact_path_lit}' OR new.path LIKE '{trailing_pattern_lit}' ESCAPE '!') \
                     BEGIN \
                       INSERT INTO {fts_table}(path, content) \
                       VALUES (new.path, COALESCE(json_extract(new.indexed, '$.{fts_key}'), '')); \
                     END"
                );
                conn.execute(&trigger_insert, ()).await.map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
                let trigger_update = format!(
                    "CREATE TRIGGER IF NOT EXISTS {index_name}_au \
                     AFTER UPDATE ON root_filesystem_entries \
                     WHEN new.is_dir = 0 \
                       AND (new.path = '{exact_path_lit}' OR new.path LIKE '{trailing_pattern_lit}' ESCAPE '!') \
                     BEGIN \
                       DELETE FROM {fts_table} WHERE path = old.path; \
                       INSERT INTO {fts_table}(path, content) \
                       VALUES (new.path, COALESCE(json_extract(new.indexed, '$.{fts_key}'), '')); \
                     END"
                );
                conn.execute(&trigger_update, ()).await.map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
                let trigger_delete = format!(
                    "CREATE TRIGGER IF NOT EXISTS {index_name}_ad \
                     AFTER DELETE ON root_filesystem_entries \
                     WHEN old.is_dir = 0 \
                       AND (old.path = '{exact_path_lit}' OR old.path LIKE '{trailing_pattern_lit}' ESCAPE '!') \
                     BEGIN \
                       DELETE FROM {fts_table} WHERE path = old.path; \
                     END"
                );
                conn.execute(&trigger_delete, ()).await.map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
                // Backfill any rows present before the index was declared.
                let backfill = format!(
                    "INSERT INTO {fts_table}(path, content) \
                     SELECT path, COALESCE(json_extract(indexed, '$.{fts_key}'), '') \
                     FROM root_filesystem_entries \
                     WHERE is_dir = 0 \
                       AND (path = ?1 OR path LIKE ?2 ESCAPE '!') \
                       AND NOT EXISTS \
                           (SELECT 1 FROM {fts_table} WHERE {fts_table}.path = root_filesystem_entries.path)"
                );
                conn.execute(
                    &backfill,
                    libsql::params![path_prefix, trailing_pattern.clone()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                })?;
            }
            IndexKind::Vector { dim } => {
                // Storage shape: IndexValue::Bytes under the indexed key.
                // The vector dim was recorded in the spec catalog above so
                // re-declaration with a different dim is rejected as a
                // SpecMismatch. No per-row table or index is created; the
                // brute-force ranker scans entries in this prefix at
                // query time. Validate dim > 0 here as a guardrail.
                if *dim == 0 {
                    return Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    });
                }
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
        // Vector-nearest is a top-k ranking operation; evaluate by scanning
        // the candidate set in this prefix and ranking by cosine in Rust.
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
        let fts_tables = self.discover_fts_tables_for_filter(path, filter).await?;
        let mut params: Vec<libsql::Value> = vec![libsql::Value::Text(path.as_str().to_string())];
        let prefix_pattern = format!("{}/%", path.as_str());
        params.push(libsql::Value::Text(escape_like_with_trailing_wildcard(
            &prefix_pattern,
        )));

        let mut conditions = String::new();
        translate_filter(path, filter, &mut conditions, &mut params, &fts_tables)?;

        let mut sql = String::from(
            "SELECT path, contents, content_type, kind, indexed, version \
             FROM root_filesystem_entries \
             WHERE is_dir = 0 AND (path = ?1 OR path LIKE ?2 ESCAPE '!')",
        );
        if !conditions.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&conditions);
        }
        sql.push_str(" ORDER BY path LIMIT ? OFFSET ?");
        params.push(libsql::Value::Integer(
            page.limit.min(crate::Page::MAX_LIMIT) as i64,
        ));
        params.push(libsql::Value::Integer(page.offset as i64));

        let conn = self.connect().await?;
        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?
        {
            let row_path: String = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let row_path = VirtualPath::new(row_path)?;
            let body: Vec<u8> = row.get(1).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let content_type_raw: String = row.get(2).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let kind_raw: Option<String> = row.get(3).ok();
            let indexed_raw: String = row.get(4).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let version_raw: i64 = row.get(5).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let entry = build_entry(&row_path, body, content_type_raw, kind_raw, indexed_raw)?;
            let version = record_version_from_i64(&row_path, version_raw)?;
            out.push(VersionedEntry {
                path: row_path,
                entry,
                version,
            });
        }
        Ok(out)
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT contents, is_dir FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?
        else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if is_dir != 0 {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        row.get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry(path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        let conn = self.connect().await?;
        // PR #3660 reviewer fix: legacy write_file must also reset the
        // record metadata (content_type / kind / indexed) and bump the
        // version, otherwise a get() after a write_file-overwrite of a
        // previously record-shaped entry returns stale metadata. Treat
        // legacy writes as opaque-file entries: kind=NULL, indexed='{}',
        // content_type=application/octet-stream, version bumped from the
        // current row's version (or 1 for new entries).
        let rows = conn
            .execute(
                r#"
                INSERT INTO root_filesystem_entries
                    (path, contents, is_dir, content_type, kind, indexed, version, updated_at)
                VALUES (?1, ?2, 0, 'application/octet-stream', NULL, '{}', 1,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ON CONFLICT (path) DO UPDATE SET
                    contents = excluded.contents,
                    is_dir = 0,
                    content_type = excluded.content_type,
                    kind = excluded.kind,
                    indexed = excluded.indexed,
                    version = root_filesystem_entries.version + 1,
                    updated_at = excluded.updated_at
                WHERE root_filesystem_entries.is_dir = 0
                "#,
                libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
            )
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
            })?;
        if rows == 0 {
            return Err(directory_write_error(path.clone()));
        }
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry(path).await?
        {
            return Err(directory_append_error(path.clone()));
        }
        let conn = self.connect().await?;
        // PR #3660 reviewer fix: same metadata-reset concern as write_file.
        // Append also resets kind/indexed/content_type to opaque-file
        // defaults — appending bytes onto a previously record-shaped
        // entry was always a category error, and we surface that by
        // clearing the schema metadata rather than leaving it stale.
        // Note: append rewrites the whole DB row. This is acceptable for
        // the legacy bytes plane (slated for removal in the consumer-
        // migration cleanup pass — see RootFilesystem::append_file's
        // deprecation note). New callers must use `append`/`tail` for
        // log-shaped mounts or `get`+`put` read-modify-write — both avoid
        // the full-row rewrite.
        conn.execute(
            r#"
            INSERT INTO root_filesystem_entries
                (path, contents, is_dir, content_type, kind, indexed, version, updated_at)
            VALUES (?1, ?2, 0, 'application/octet-stream', NULL, '{}', 1,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (path) DO UPDATE SET
                contents = CAST(root_filesystem_entries.contents || excluded.contents AS BLOB),
                is_dir = 0,
                content_type = excluded.content_type,
                kind = excluded.kind,
                indexed = excluded.indexed,
                version = root_filesystem_entries.version + 1,
                updated_at = excluded.updated_at
            "#,
            libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let exact_entry = self.exact_entry(path).await?;
        if matches!(exact_entry, Some((_, FileType::File, _))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self
            .child_entries(path, FilesystemOperation::ListDir)
            .await?;
        let children = direct_children(path, rows);
        if matches!(exact_entry, Some((_, FileType::Directory, _))) && is_not_found(&children) {
            return Ok(Vec::new());
        }
        children
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        if let Some((len, file_type, modified)) = self.exact_entry(path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type,
                len,
                modified,
                sensitive: false,
            });
        }
        if self.has_child_entry(path).await? {
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
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'",
                libsql::params![path.as_str(), child_path_like_pattern(path)],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        if deleted == 0 {
            return Err(not_found(path.clone(), FilesystemOperation::Delete));
        }
        Ok(())
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let conn = self.connect().await?;
        // INTEGER PRIMARY KEY AUTOINCREMENT assigns a fresh monotonic id per
        // insert. We capture the assigned id via last_insert_rowid() under
        // the same connection so concurrent writers don't observe each
        // other's rowids — libsql's per-connection model gives us that
        // for free.
        conn.execute(
            r#"
            INSERT INTO root_filesystem_events (path, payload, created_at)
            VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            "#,
            libsql::params![path.as_str(), libsql::Value::Blob(payload)],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        let mut rows = conn
            .query("SELECT last_insert_rowid()", ())
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error)
            })?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))?
            .ok_or_else(|| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: "last_insert_rowid returned no row after insert".to_string(),
            })?;
        let seq_raw: i64 = row.get(0).map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error)
        })?;
        seq_no_from_i64(path, seq_raw, FilesystemOperation::AppendFile)
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let conn = self.connect().await?;
        let from_raw = i64::try_from(from.get()).map_err(|_| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Tail,
            reason: "tail cursor exceeds i64".to_string(),
        })?;
        let mut rows = conn
            .query(
                r#"
                SELECT seq, payload
                FROM root_filesystem_events
                WHERE path = ?1 AND seq > ?2
                ORDER BY seq ASC
                "#,
                libsql::params![path.as_str(), from_raw],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Tail, error))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Tail, error))?
        {
            let seq_raw: i64 = row
                .get(0)
                .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Tail, error))?;
            let payload: Vec<u8> = row
                .get(1)
                .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Tail, error))?;
            out.push(EventRecord {
                seq: seq_no_from_i64(path, seq_raw, FilesystemOperation::Tail)?,
                payload,
            });
        }
        Ok(out)
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        let transaction = conn.transaction().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        for prefix in virtual_path_prefixes(path)? {
            let mut rows = transaction
                .query(
                    "SELECT is_dir FROM root_filesystem_entries WHERE path = ?1",
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
            if let Some(row) = rows.next().await.map_err(|error| {
                libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
            })? {
                let is_dir: i64 = row.get(0).map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
                if is_dir == 0 {
                    return Err(FilesystemError::Backend {
                        path: prefix,
                        operation: FilesystemOperation::CreateDirAll,
                        reason: "file exists where directory is required".to_string(),
                    });
                }
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
                    VALUES (?1, X'', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        Ok(())
    }
}

#[cfg(feature = "libsql")]
async fn ensure_libsql_root_is_dir_column(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('root_filesystem_entries') WHERE name = 'is_dir'",
            (),
        )
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?;
    if rows
        .next()
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE root_filesystem_entries ADD COLUMN is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1))",
        (),
    )
    .await
    .map_err(|error| {
        libsql_db_error(
            valid_engine_path(),
            FilesystemOperation::CreateDirAll,
            error,
        )
    })?;
    Ok(())
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    async fn exact_entry(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType, Option<std::time::SystemTime>)>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT length(contents), is_dir, CAST(strftime('%s', updated_at) AS INTEGER) AS updated_at_epoch FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let Some(row) = row else { return Ok(None) };
        let len_raw: i64 = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let is_dir_raw: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let updated_at_epoch: i64 = row
            .get(2)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let len = len_raw.max(0) as u64;
        let is_dir = is_dir_raw != 0;
        Ok(Some((
            if is_dir { 0 } else { len },
            if is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
            system_time_from_unix_seconds(updated_at_epoch),
        )))
    }

    async fn child_entries(
        &self,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT path, length(contents), is_dir FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' ORDER BY path",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?
        {
            let path: String = row
                .get(0)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len_raw: i64 = row
                .get(1)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let is_dir_raw: i64 = row
                .get(2)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len = len_raw.max(0) as u64;
            let is_dir = is_dir_raw != 0;
            paths.push((
                VirtualPath::new(path)?,
                if is_dir { 0 } else { len },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
            ));
        }
        Ok(paths)
    }

    async fn has_child_entry(&self, parent: &VirtualPath) -> Result<bool, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT 1 FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' LIMIT 1",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        Ok(rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?
            .is_some())
    }

    /// Resolve every FTS index name covering `path` whose first key is
    /// referenced by `filter`. Returns a map from index-key (the JSON
    /// indexed-projection key) to the FTS5 vtable name created by
    /// `ensure_index`. Used by the WHERE-clause translator.
    async fn discover_fts_tables_for_filter(
        &self,
        path: &VirtualPath,
        filter: &Filter,
    ) -> Result<std::collections::HashMap<String, String>, FilesystemError> {
        let mut keys: Vec<String> = Vec::new();
        collect_fts_keys(filter, &mut keys);
        if keys.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.connect().await?;
        let mut out = std::collections::HashMap::new();
        // Scan the spec catalog for FTS specs whose prefix is path or any
        // ancestor (so callers may declare the index on a higher prefix
        // and query a child path).
        let candidate_prefixes = ancestor_prefixes(path.as_str());
        let placeholders: Vec<String> = (1..=candidate_prefixes.len())
            .map(|i| format!("?{i}"))
            .collect();
        let sql = format!(
            "SELECT prefix, name, keys FROM root_filesystem_index_specs \
             WHERE kind = 'fts' AND prefix IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<libsql::Value> = candidate_prefixes
            .iter()
            .map(|p| libsql::Value::Text(p.clone()))
            .collect();
        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?;
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?
        {
            let prefix: String = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let name: String = row.get(1).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let keys_json: String = row.get(2).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let parsed_keys: Vec<String> =
                serde_json::from_str(&keys_json).map_err(|_| FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                    reason: "corrupt index spec keys".to_string(),
                })?;
            let Some(first_key) = parsed_keys.first() else {
                continue;
            };
            if !keys.iter().any(|k| k == first_key) {
                continue;
            }
            // First match wins; if the caller declared multiple FTS
            // indexes for the same key on overlapping prefixes the most
            // specific (longest matching prefix) wins because the
            // candidate_prefixes list is ordered most-specific-first
            // below.
            out.entry(first_key.clone())
                .or_insert_with(|| format!("{}_fts", sql_index_name(&prefix, &name)));
        }
        Ok(out)
    }

    /// Brute-force cosine over candidates under `path` whose indexed
    /// projection has an `IndexValue::Bytes` value at `key` decoded as a
    /// little-endian f32 buffer of any non-zero length matching the query
    /// embedding's length. Returns the top `limit` results.
    async fn vector_nearest_query(
        &self,
        path: &VirtualPath,
        key: &IndexKey,
        embedding: &[f32],
        limit: u32,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let conn = self.connect().await?;
        let prefix_pattern = format!("{}/%", path.as_str());
        let escaped = escape_like_with_trailing_wildcard(&prefix_pattern);
        // Pull every row in this prefix that has *some* value under the
        // indexed key. Filter to byte values + matching length in Rust.
        let sql = "SELECT path, contents, content_type, kind, indexed, version \
                   FROM root_filesystem_entries \
                   WHERE is_dir = 0 AND (path = ?1 OR path LIKE ?2 ESCAPE '!')";
        let mut rows = conn
            .query(sql, libsql::params![path.as_str(), escaped.clone()])
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?;
        let mut ranked: Vec<(VersionedEntry, f32)> = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?
        {
            let row_path: String = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let row_path = VirtualPath::new(row_path)?;
            let body: Vec<u8> = row.get(1).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let content_type_raw: String = row.get(2).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let kind_raw: Option<String> = row.get(3).ok();
            let indexed_raw: String = row.get(4).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let version_raw: i64 = row.get(5).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let entry = build_entry(&row_path, body, content_type_raw, kind_raw, indexed_raw)?;
            let Some(IndexValue::Bytes(bytes)) = entry.indexed.get(key) else {
                continue;
            };
            let Some(vec) = decode_embedding_blob(bytes) else {
                continue;
            };
            let Some(score) = cosine_similarity(embedding, &vec) else {
                continue;
            };
            let version = record_version_from_i64(&row_path, version_raw)?;
            ranked.push((VersionedEntry { entry, version }, score));
        }
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit as usize);
        Ok(ranked.into_iter().map(|(entry, _)| entry).collect())
    }

    async fn current_version(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<RecordVersion>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT version FROM root_filesystem_entries WHERE path = ?1 AND is_dir = 0",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?
        else {
            return Ok(None);
        };
        let version: i64 = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        Ok(Some(record_version_from_i64(path, version)?))
    }
}

#[cfg(feature = "libsql")]
fn build_entry(
    path: &VirtualPath,
    body: Vec<u8>,
    content_type_raw: String,
    kind_raw: Option<String>,
    indexed_raw: String,
) -> Result<Entry, FilesystemError> {
    let content_type = ContentType::new(content_type_raw).map_err(FilesystemError::Contract)?;
    let kind = kind_raw
        .map(RecordKind::new)
        .transpose()
        .map_err(FilesystemError::Contract)?;
    let indexed: BTreeMap<IndexKey, IndexValue> = if indexed_raw.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_str(&indexed_raw).map_err(|_| FilesystemError::DeserializeIndexed {
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

#[cfg(feature = "libsql")]
async fn ensure_libsql_records_columns(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    add_column_if_missing(
        conn,
        "content_type",
        "ALTER TABLE root_filesystem_entries ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/octet-stream'",
    )
    .await?;
    add_column_if_missing(
        conn,
        "kind",
        "ALTER TABLE root_filesystem_entries ADD COLUMN kind TEXT",
    )
    .await?;
    add_column_if_missing(
        conn,
        "indexed",
        "ALTER TABLE root_filesystem_entries ADD COLUMN indexed TEXT NOT NULL DEFAULT '{}'",
    )
    .await?;
    add_column_if_missing(
        conn,
        "version",
        "ALTER TABLE root_filesystem_entries ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn ensure_libsql_index_specs_table(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_INDEX_SPECS_SCHEMA)
        .await
        .map_err(|error| {
            libsql_db_error(valid_engine_path(), FilesystemOperation::EnsureIndex, error)
        })?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn ensure_libsql_events_table(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_EVENTS_SCHEMA)
        .await
        .map_err(|error| {
            libsql_db_error(valid_engine_path(), FilesystemOperation::AppendFile, error)
        })?;
    Ok(())
}

#[cfg(feature = "libsql")]
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

/// Translate a [`Filter`] tree into a libsql WHERE-clause fragment.
///
/// Reviewer (PR #3661) flagged that the prior version's "skip empty
/// children" logic conflated `Filter::All` with the identity element of
/// each compound, so `Or([])` returned every row instead of none and
/// `And([All])` could emit malformed SQL. The fix: every node always
/// produces a non-empty fragment — `Filter::All` becomes the literal
/// `TRUE`, empty `And` becomes `TRUE`, empty `Or` becomes `FALSE`. This
/// matches the in-memory backend's `all`/`any` semantics.
#[cfg(feature = "libsql")]
fn translate_filter(
    path: &VirtualPath,
    filter: &Filter,
    out: &mut String,
    params: &mut Vec<libsql::Value>,
    fts_tables: &std::collections::HashMap<String, String>,
) -> Result<(), FilesystemError> {
    match filter {
        Filter::All => {
            out.push_str("TRUE");
            Ok(())
        }
        Filter::Eq { key, value } => {
            let placeholder = bind_index_value(path, value, params)?;
            out.push_str(&format!(
                "(json_extract(indexed, '$.{}') = ?{})",
                key.as_str(),
                placeholder
            ));
            Ok(())
        }
        Filter::PrefixOn { key, value } => {
            let IndexValue::Text(prefix_value) = value else {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                });
            };
            // PR #3661 reviewer fix: user-input prefix must be fully
            // escaped (including any literal `%` characters) before
            // appending the LIKE wildcard.
            let escaped = escape_like_literal(prefix_value);
            params.push(libsql::Value::Text(format!("{escaped}%")));
            out.push_str(&format!(
                "(json_extract(indexed, '$.{}') LIKE ?{} ESCAPE '!')",
                key.as_str(),
                params.len()
            ));
            Ok(())
        }
        Filter::Range { key, lo, hi } => {
            // PR #3659 review fix: guard the comparison with a JSON-type
            // check so a row whose stored value at `$.{key}` is a different
            // variant (e.g. text under a numeric range) does NOT participate
            // in BETWEEN. Without this guard a mixed-variant store can pull
            // unrelated values into the result set or fail the query
            // entirely on a cast failure.
            let lo_idx = bind_index_value(path, lo, params)?;
            let hi_idx = bind_index_value(path, hi, params)?;
            let expected_json_type = index_value_json_type(lo);
            out.push_str(&format!(
                "(json_type(indexed, '$.{}') = '{expected_json_type}' \
                 AND json_extract(indexed, '$.{}') BETWEEN ?{lo_idx} AND ?{hi_idx})",
                key.as_str(),
                key.as_str(),
            ));
            Ok(())
        }
        Filter::Fts { key, query } => {
            let Some(fts_table) = fts_tables.get(key.as_str()) else {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                });
            };
            params.push(libsql::Value::Text(query.clone()));
            out.push_str(&format!(
                "(path IN (SELECT path FROM {fts_table} WHERE {fts_table} MATCH ?{}))",
                params.len()
            ));
            Ok(())
        }
        Filter::VectorNearest { .. } => Err(FilesystemError::Unsupported {
            // VectorNearest is evaluated by the top-level `query` method,
            // not inside the WHERE fragment. Reaching the translator
            // means a caller composed it inside an And/Or — which would
            // throw away the ranking. Surface as Unsupported so the
            // caller restructures the query.
            path: path.clone(),
            operation: FilesystemOperation::Query,
        }),
        Filter::And(children) => {
            translate_compound(path, children, " AND ", "TRUE", out, params, fts_tables)
        }
        Filter::Or(children) => {
            translate_compound(path, children, " OR ", "FALSE", out, params, fts_tables)
        }
    }
}

#[cfg(feature = "libsql")]
fn translate_compound(
    path: &VirtualPath,
    children: &[Filter],
    joiner: &str,
    empty_identity: &str,
    out: &mut String,
    params: &mut Vec<libsql::Value>,
    fts_tables: &std::collections::HashMap<String, String>,
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
        // Recurse: every child now produces a non-empty fragment thanks to
        // the `Filter::All -> TRUE` rule, so we don't need the prior
        // "skip empty" branch that broke `Or([])`/`And([All])`.
        translate_filter(path, child, out, params, fts_tables)?;
    }
    out.push(')');
    Ok(())
}

#[cfg(feature = "libsql")]
fn collect_fts_keys(filter: &Filter, out: &mut Vec<String>) {
    match filter {
        Filter::Fts { key, .. } => {
            let k = key.as_str().to_string();
            if !out.contains(&k) {
                out.push(k);
            }
        }
        Filter::And(children) | Filter::Or(children) => {
            for child in children {
                collect_fts_keys(child, out);
            }
        }
        _ => {}
    }
}

/// All ancestor paths of `path`, **most specific first**, ending at `/`.
/// Used to find an FTS index declared on a higher prefix that should still
/// cover descendant queries.
#[cfg(feature = "libsql")]
fn ancestor_prefixes(path: &str) -> Vec<String> {
    let mut out = vec![path.trim_end_matches('/').to_string()];
    let mut cur = path.trim_end_matches('/').to_string();
    while let Some(idx) = cur.rfind('/') {
        if idx == 0 {
            out.push("/".to_string());
            break;
        }
        cur.truncate(idx);
        out.push(cur.clone());
    }
    out
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
fn bind_index_value(
    path: &VirtualPath,
    value: &IndexValue,
    params: &mut Vec<libsql::Value>,
) -> Result<usize, FilesystemError> {
    let bound = match value {
        IndexValue::Text(s) => libsql::Value::Text(s.clone()),
        IndexValue::I64(n) => libsql::Value::Integer(*n),
        IndexValue::Bool(b) => libsql::Value::Integer(i64::from(*b)),
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

/// Maps an [`IndexValue`] variant to the corresponding SQLite `json_type`
/// discriminator string. Used to guard `Filter::Range` so cross-variant
/// stored values don't participate in BETWEEN comparisons (PR #3659 review
/// fix).
#[cfg(feature = "libsql")]
fn index_value_json_type(value: &IndexValue) -> &'static str {
    match value {
        IndexValue::Text(_) => "text",
        IndexValue::I64(_) => "integer",
        // SQLite's json_type returns "true" / "false" for booleans, not "boolean".
        IndexValue::Bool(_) => "integer", // we encode bools as 0/1 integers above
        IndexValue::Bytes(_) => "text",
    }
}

#[cfg(feature = "libsql")]
async fn add_column_if_missing(
    conn: &libsql::Connection,
    column: &str,
    ddl: &str,
) -> Result<(), FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('root_filesystem_entries') WHERE name = ?1",
            libsql::params![column],
        )
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?;
    if rows
        .next()
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(ddl, ()).await.map_err(|error| {
        libsql_db_error(
            valid_engine_path(),
            FilesystemOperation::CreateDirAll,
            error,
        )
    })?;
    Ok(())
}

#[cfg(feature = "libsql")]
const LIBSQL_ROOT_FILESYSTEM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY,
    contents BLOB NOT NULL DEFAULT X'',
    is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
-- The PRIMARY KEY on `path` already provides a unique index for equality
-- lookups, so no separate index is created.
"#;

#[cfg(feature = "libsql")]
const LIBSQL_INDEX_SPECS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_index_specs (
    prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    keys TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (prefix, name)
);
"#;

#[cfg(feature = "libsql")]
const LIBSQL_EVENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    payload BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_root_filesystem_events_path_seq
    ON root_filesystem_events(path, seq);
"#;
