// arch-exempt: large_file, targeted libSQL contention regression stays with its backend, plan #4088
use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::EventRecord;
use crate::db::{
    child_path_like_pattern, descendant_path_range, direct_children, directory_append_error,
    directory_write_error, escape_like_literal, escape_like_with_trailing_wildcard,
    infrastructure_libsql_error, is_not_found, libsql_db_error, not_found, page_offset_to_i64,
    record_version_from_i64, record_version_to_i64, sql_index_name, system_time_from_unix_seconds,
    virtual_path_prefixes,
};
use crate::libsql_pool::{LibSqlPool, PooledLibSqlConnection, build_libsql_pool};
use crate::vector::{cosine_similarity, decode_embedding_blob};
use crate::{
    BackendCapabilities, Capability, CasExpectation, ContentType, DirEntry, Entry, FileStat,
    FileType, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind, IndexSpec,
    IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, SeqNo, VersionedEntry,
};
/// libSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct LibSqlRootFilesystem {
    pool: LibSqlPool,
}
const LIBSQL_CHILD_ENTRIES_SQL: &str = "SELECT path, length(contents), is_dir \
    FROM root_filesystem_entries \
    WHERE path >= ?1 AND path < ?2 \
    ORDER BY path";
const LIBSQL_HAS_CHILD_ENTRY_SQL: &str = "SELECT 1 \
    FROM root_filesystem_entries \
    WHERE path >= ?1 AND path < ?2 \
    LIMIT 1";
impl LibSqlRootFilesystem {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self {
            pool: build_libsql_pool(db),
        }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        // Switch the database to WAL journaling once, here, before any
        // transaction is opened. WAL is persisted in the database header, so
        // a single successful run sticks for the life of the file and for
        // every future connection; re-running migrations on an
        // already-WAL database is a cheap no-op.
        //
        // This is the single biggest lever on concurrent-write latency: the
        // default `DELETE` rollback journal takes an EXCLUSIVE lock over the
        // whole file for every commit and blocks readers for the duration,
        // so the many read-before-write checks on the turn/loop path
        // serialise behind each writer. WAL lets readers run concurrently
        // with the (still single) writer and turns each commit into an
        // append to the WAL instead of a rollback-journal create/fsync/
        // delete cycle.
        //
        // `journal_mode` cannot be changed inside a transaction, so it must
        // run before the `BEGIN IMMEDIATE` below. Use `query` to drain the
        // single row the pragma returns (the resulting mode).
        conn.query("PRAGMA journal_mode = WAL", ())
            .await
            .map_err(|error| {
                infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error)
            })?;
        // Wrap every step in a single SQLite transaction so a mid-migration
        // crash can't leave concurrent readers observing a half-migrated
        // schema (e.g. `is_dir` column present but `version` missing). SQLite
        // supports transactional DDL — CREATE TABLE, CREATE INDEX, and
        // ALTER TABLE ADD COLUMN all participate in BEGIN/COMMIT.
        //
        // `BEGIN IMMEDIATE` acquires the write lock up front so two
        // concurrent processes attempting first-time migration serialise
        // rather than both racing the pragma checks.
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(|error| {
            infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error)
        })?;
        let result = run_libsql_migrations_inner(&conn).await;
        match result {
            Ok(()) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| ())
                .map_err(|error| {
                    infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error)
                }),
            Err(err) => {
                // Best-effort rollback. If ROLLBACK itself fails (e.g. the
                // connection is already aborted) we still surface the
                // original migration error to the caller — `_` is the
                // documented pattern for unwinding here. SQLite auto-rolls-
                // back on connection close as a final safety net.
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    /// Check out a pooled connection for exclusive use until the guard
    /// drops. Callers must drop the guard before `.await`-ing any other
    /// `self` method that also checks out — see the invariant note in
    /// [`crate::libsql_pool`].
    async fn connect(&self) -> Result<PooledLibSqlConnection, FilesystemError> {
        self.pool.get().await.map_err(|error| match error {
            deadpool::managed::PoolError::Backend(error) => {
                let reason = error.to_string();
                tracing::debug!(%reason, "libSQL root filesystem pool checkout failed");
                error
            }
            other => {
                let reason = format!("libSQL connection pool checkout failed: {other}");
                tracing::debug!(%reason, "libSQL root filesystem pool checkout failed");
                crate::db::infrastructure_error(FilesystemOperation::Connect, reason)
            }
        })
    }
}
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
        let indexed_json = serde_json::to_string(&entry.indexed).map_err(|_| {
            FilesystemError::SerializeIndexed {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            }
        })?;
        let kind_str = entry.kind.as_ref().map(|k| k.as_str().to_string());
        let content_type_str = entry.content_type.as_str().to_string();
        let body = entry.body;

        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
        })?;
        let result = put_libsql_inner(
            &conn,
            path,
            body,
            content_type_str,
            kind_str,
            indexed_json,
            cas,
        )
        .await;
        match result {
            Ok(version) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| version)
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
                }),
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
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
                // Defense in depth: the FTS5 sync triggers below splice the
                // mount-prefix path directly into DDL string literals because
                // SQLite's trigger language has no parameter binding. The
                // standard `'`-doubling escape is correct, but a path that
                // legitimately reaches here with any non-identifier character
                // is suspicious and we refuse to emit DDL for it. Accept only
                // characters that are unambiguously safe in a string literal
                // (`[A-Za-z0-9_/.-]`). `VirtualPath` validation rejects NUL,
                // control chars, backslashes, and `..`, but does not (today)
                // reject `'`, `"`, `;`, or other punctuation. This check is
                // narrower than VirtualPath's and keeps the DDL emitter
                // self-contained.
                if !path_prefix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-'))
                {
                    return Err(FilesystemError::Backend {
                        path: path.clone(),
                        operation: FilesystemOperation::EnsureIndex,
                        reason: "FTS index path contains characters outside \
                                 [A-Za-z0-9_/.-]; refusing to emit DDL"
                            .to_string(),
                    });
                }
                let trailing_prefix = format!("{}/", path_prefix.trim_end_matches('/'));
                let trailing_pattern =
                    escape_like_with_trailing_wildcard(&format!("{trailing_prefix}%"));
                // After the identifier-safe check above, `'`-doubling is a
                // belt-and-suspenders safety net; the input cannot contain
                // `'` so the replace is a no-op on valid inputs.
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
        // `page.limit` is `u32` and clamped to `Page::MAX_LIMIT` (1024),
        // so the i64 cast is bounded and safe. `page.offset` is `u64`
        // and is user-supplied — guard with `try_from` so values ≥ 2^63
        // surface a typed `Backend` error instead of wrapping to a
        // negative OFFSET. (Audit finding F6.)
        params.push(libsql::Value::Integer(i64::from(
            page.limit.min(crate::Page::MAX_LIMIT),
        )));
        params.push(libsql::Value::Integer(page_offset_to_i64(
            path,
            page.offset,
        )?));

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

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let conn = self.connect().await?;
        let max_bytes = max_bytes as i64;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    CASE
                        WHEN length(contents) <= ?2 THEN contents
                        ELSE NULL
                    END,
                    length(contents),
                    is_dir
                FROM root_filesystem_entries
                WHERE path = ?1
                "#,
                libsql::params![path.as_str(), max_bytes],
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
            .get(2)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if is_dir != 0 {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        let len: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if len > max_bytes {
            return Ok(None);
        }
        row.get(0)
            .map(Some)
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
        // Sweep the append-event log for this path and its subtree. Append-only
        // finalized assistant messages live in `root_filesystem_events`, so a
        // delete/recreate of the same thread would otherwise replay stale
        // history from the old log. Mirrors the entries-delete predicate above.
        conn.execute(
            "DELETE FROM root_filesystem_events WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'",
            libsql::params![path.as_str(), child_path_like_pattern(path)],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        // Sweep any reserved sequence counter for this path and its subtree so
        // a delete/recreate restarts sequences from 1 rather than resuming
        // stale state. Mirrors the entries-delete predicate above.
        conn.execute(
            "DELETE FROM root_filesystem_sequences WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'",
            libsql::params![path.as_str(), child_path_like_pattern(path)],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        Ok(())
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        // Single-key CAS delete: unlike `delete`, no subtree/event/sequence
        // sweep. `is_dir = 0` scopes it to the record plane, matching `put`'s
        // Version arm and `current_version_libsql`.
        //
        // Review fix (PR #5749): the conditional DELETE and the zero-rows
        // diagnosis read must be atomic w.r.t. a concurrent delete+recreate
        // on the same path, or the diagnosis can observe a version written
        // *after* our DELETE decided 0 rows matched, misclassifying the
        // outcome. `BEGIN IMMEDIATE` takes the write lock up front (same
        // idiom as `put`) so the DELETE and the follow-up SELECT run as one
        // unit on one connection — this also keeps the call stack to a
        // single checkout, matching the one-checkout-per-call-stack
        // invariant the bounded pool (see `libsql_pool`, issue #5466) enforces
        // (no nested `self.connect()`).
        //
        // Round-A review: validate `expected_version` before taking the
        // pool checkout / write lock. An out-of-range version can never
        // match a real row, so failing closed here avoids holding a
        // contended connection (and SQLite's write lock) for a call
        // destined to error — relevant under the concurrent CAS storms
        // this pool exists to survive.
        let expected_raw = record_version_to_i64(path, expected_version)?;
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        let result =
            delete_if_version_libsql_inner(&conn, path, expected_version, expected_raw).await;
        match result {
            Ok(()) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| ())
                .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error)),
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
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
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        let mut rows = conn
            .query("SELECT last_insert_rowid()", ())
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?
            .ok_or_else(|| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Append,
                reason: "last_insert_rowid returned no row after insert".to_string(),
            })?;
        let seq_raw: i64 = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        seq_no_from_i64(path, seq_raw, FilesystemOperation::Append)
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connect().await?;
        // One multi-row INSERT per chunk collapses N appends into one round-trip.
        // `seq` is INTEGER PRIMARY KEY AUTOINCREMENT, assigned in VALUES order;
        // `RETURNING seq` then sorted ASC recovers payload order
        // deterministically. Chunk the batch so the bound parameter count
        // (2 per row) stays well under SQLite's default 999-parameter limit.
        // All chunks run inside a single transaction handle that auto-rolls-back
        // on drop if not committed, making this cancellation-safe.
        const ROWS_PER_STATEMENT: usize = 256;
        let tx = conn
            .transaction()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        let mut seqs: Vec<i64> = Vec::with_capacity(payloads.len());
        let mut iter = payloads.into_iter().peekable();
        while iter.peek().is_some() {
            let mut sql =
                String::from("INSERT INTO root_filesystem_events (path, payload) VALUES ");
            let mut params: Vec<libsql::Value> = Vec::new();
            for (row_idx, payload) in (&mut iter).take(ROWS_PER_STATEMENT).enumerate() {
                if row_idx > 0 {
                    sql.push(',');
                }
                sql.push_str("(?, ?)");
                params.push(libsql::Value::Text(path.as_str().to_string()));
                params.push(libsql::Value::Blob(payload));
            }
            sql.push_str(" RETURNING seq");
            let mut rows = tx.query(&sql, params).await.map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Append, error)
            })?;
            while let Some(row) = rows.next().await.map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Append, error)
            })? {
                let seq_raw: i64 = row.get(0).map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::Append, error)
                })?;
                seqs.push(seq_raw);
            }
        }
        tx.commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        seqs.sort_unstable();
        seqs.into_iter()
            .map(|seq_raw| seq_no_from_i64(path, seq_raw, FilesystemOperation::Append))
            .collect()
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
        let conn = self.connect().await?;
        let from_raw = i64::try_from(from.get()).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Tail,
            reason: format!("tail cursor exceeds i64: {error}"),
        })?;
        // silent-ok: callers can request an unbounded tail; saturating keeps the
        // SQL LIMIT representable without changing the public trait contract.
        let limit_raw = i64::try_from(max_records).unwrap_or(i64::MAX);
        let mut rows = conn
            .query(
                r#"
                SELECT seq, payload
                FROM root_filesystem_events
                WHERE path = ?1 AND seq > ?2
                ORDER BY seq ASC
                LIMIT ?3
                "#,
                libsql::params![path.as_str(), from_raw, limit_raw],
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

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        let conn = self.connect().await?;
        let from_raw = i64::try_from(from.get()).map_err(|_| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::HeadSeq,
            reason: "head_seq cursor exceeds i64".to_string(),
        })?;
        let mut rows = conn
            .query(
                r#"
                SELECT MAX(seq) AS head
                FROM root_filesystem_events
                WHERE path = ?1 AND seq > ?2
                "#,
                libsql::params![path.as_str(), from_raw],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::HeadSeq, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::HeadSeq, error))?
        else {
            return Ok(None);
        };
        // `MAX(...)` over an empty match set yields SQL NULL.
        let head_raw: Option<i64> = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::HeadSeq, error))?;
        match head_raw {
            Some(seq_raw) => Ok(Some(seq_no_from_i64(
                path,
                seq_raw,
                FilesystemOperation::HeadSeq,
            )?)),
            None => Ok(None),
        }
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                INSERT INTO root_filesystem_sequences (path, next_seq, updated_at)
                VALUES (?1, 2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ON CONFLICT(path) DO UPDATE SET
                    next_seq = root_filesystem_sequences.next_seq + 1,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                RETURNING next_seq - 1
                "#,
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error)
            })?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?
            .ok_or_else(|| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReserveSeq,
                reason: "sequence reservation returned no row".to_string(),
            })?;
        let seq_raw: i64 = row.get(0).map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error)
        })?;
        seq_no_from_i64(path, seq_raw, FilesystemOperation::ReserveSeq)
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        let result = create_dir_all_libsql_inner(&conn, path).await;
        match result {
            Ok(()) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| ())
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                }),
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }
}
async fn put_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
    body: Vec<u8>,
    content_type_str: String,
    kind_str: Option<String>,
    indexed_json: String,
    cas: CasExpectation,
) -> Result<RecordVersion, FilesystemError> {
    // Reject writes that would clobber a directory or a path that has
    // children (mirrors `write_file` semantics so legacy and new ops stay
    // consistent). Run these checks inside the write transaction so concurrent
    // writers queue at BEGIN IMMEDIATE instead of racing read-then-write
    // upgrades through independent connections.
    if matches!(
        exact_entry_libsql(conn, path).await?,
        Some((_, FileType::Directory, _))
    ) || has_child_entry_libsql(conn, path).await?
    {
        return Err(directory_write_error(path.clone()));
    }

    match cas {
        CasExpectation::Absent => {
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
                let found = current_version_libsql(conn, path).await?;
                return Err(FilesystemError::VersionMismatch {
                    path: path.clone(),
                    expected: None,
                    found,
                });
            }
            Ok(RecordVersion::from_backend(1))
        }
        CasExpectation::Version(expected) => {
            let expected_raw = record_version_to_i64(path, expected)?;
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
                let found = current_version_libsql(conn, path).await?;
                return Err(FilesystemError::VersionMismatch {
                    path: path.clone(),
                    expected: Some(expected),
                    found,
                });
            }
            Ok(expected.next())
        }
        CasExpectation::Any => {
            let mut rows = conn
                .query(
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
                    RETURNING version
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
            let row = rows
                .next()
                .await
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
                })?
                .ok_or_else(|| directory_write_error(path.clone()))?;
            let version_raw: i64 = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
            })?;
            record_version_from_i64(path, version_raw)
        }
    }
}
async fn create_dir_all_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    for prefix in virtual_path_prefixes(path)? {
        let mut rows = conn
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
        conn.execute(
            r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
                    VALUES (?1, X'', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                    ON CONFLICT (path) DO NOTHING
                    "#,
            libsql::params![prefix.as_str()],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
    }
    Ok(())
}
async fn exact_entry_libsql(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<Option<(u64, FileType, Option<std::time::SystemTime>)>, FilesystemError> {
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
async fn has_child_entry_libsql(
    conn: &libsql::Connection,
    parent: &VirtualPath,
) -> Result<bool, FilesystemError> {
    let (prefix_lower, prefix_upper) = descendant_path_range(parent);
    let mut rows = conn
        .query(
            LIBSQL_HAS_CHILD_ENTRY_SQL,
            libsql::params![prefix_lower, prefix_upper],
        )
        .await
        .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?;
    Ok(rows
        .next()
        .await
        .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?
        .is_some())
}
async fn current_version_libsql(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<Option<RecordVersion>, FilesystemError> {
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

/// Body of `delete_if_version` extracted so the outer caller can wrap the
/// conditional DELETE and the zero-rows diagnosis SELECT in one
/// BEGIN IMMEDIATE / COMMIT transaction, with a single ROLLBACK path.
/// Running both statements on the same connection inside the same
/// transaction is what makes the classification atomic: nothing else can
/// delete-then-recreate the row between the DELETE and the diagnosis read.
async fn delete_if_version_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
    expected_version: RecordVersion,
    expected_raw: i64,
) -> Result<(), FilesystemError> {
    let deleted = conn
        .execute(
            "DELETE FROM root_filesystem_entries \
             WHERE path = ?1 AND is_dir = 0 AND version = ?2",
            libsql::params![path.as_str(), expected_raw],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
    if deleted > 0 {
        return Ok(());
    }
    // 0 rows: absent row → NotFound (already gone, benign); row present
    // at another version → VersionMismatch (gone stale). Distinct from
    // put's diagnosis, which collapses absent into VersionMismatch.
    if let Some(found) = current_version_libsql(conn, path).await? {
        return Err(FilesystemError::VersionMismatch {
            path: path.clone(),
            expected: Some(expected_version),
            found: Some(found),
        });
    }
    Err(not_found(path.clone(), FilesystemOperation::Delete))
}

/// Body of `run_migrations` extracted so the outer caller can wrap the
/// whole sequence in BEGIN IMMEDIATE / COMMIT with one rollback path.
async fn run_libsql_migrations_inner(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_ROOT_FILESYSTEM_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    ensure_libsql_root_is_dir_column(conn).await?;
    ensure_libsql_records_columns(conn).await?;
    ensure_libsql_index_specs_table(conn).await?;
    ensure_libsql_events_table(conn).await?;
    ensure_libsql_sequences_table(conn).await?;
    Ok(())
}
async fn ensure_libsql_root_is_dir_column(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('root_filesystem_entries') WHERE name = 'is_dir'",
            (),
        )
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    if rows
        .next()
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE root_filesystem_entries ADD COLUMN is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1))",
        (),
    )
    .await
    .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    Ok(())
}
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
        let (prefix_lower, prefix_upper) = descendant_path_range(parent);
        let mut rows = conn
            .query(
                LIBSQL_CHILD_ENTRIES_SQL,
                libsql::params![prefix_lower, prefix_upper],
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
        has_child_entry_libsql(&conn, parent).await
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
    ///
    /// Two-phase to bound memory on large prefixes (review feedback on
    /// the unified-FS rework): first SELECT `(path, indexed, version)`
    /// for every candidate, rank by cosine in Rust, then `get()` the
    /// top-k entries to materialize bodies. Rows that don't survive
    /// the cutoff never have their `contents` blob loaded.
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
        let sql = "SELECT path, indexed, version \
                   FROM root_filesystem_entries \
                   WHERE is_dir = 0 AND (path = ?1 OR path LIKE ?2 ESCAPE '!')";
        let mut rows = conn
            .query(sql, libsql::params![path.as_str(), escaped.clone()])
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?;
        let mut ranked: Vec<(VirtualPath, RecordVersion, f32)> = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?
        {
            let row_path: String = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let row_path = VirtualPath::new(row_path)?;
            let indexed_raw: String = row.get(1).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let version_raw: i64 = row.get(2).map_err(|error| {
                libsql_db_error(row_path.clone(), FilesystemOperation::Query, error)
            })?;
            let indexed: BTreeMap<IndexKey, IndexValue> = if indexed_raw.is_empty() {
                BTreeMap::new()
            } else {
                serde_json::from_str(&indexed_raw).map_err(|_| {
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
        // Materialize bodies only for the top-k. Drop the streaming
        // iterator + connection so each `get()` claims its own
        // connection via the pool helper.
        drop(rows);
        drop(conn);
        self.materialize_ranked(ranked).await
    }

    /// Phase-2 of [`vector_nearest_query`]: load full [`VersionedEntry`]
    /// bodies for the ranked-and-truncated candidate set.
    ///
    /// A path that disappears between phase-1 ranking and phase-2 `get` is
    /// silently dropped from the result — the search "fails open" so a
    /// concurrent delete doesn't blow up an in-flight query. Pulled out
    /// of `vector_nearest_query` to give the concurrent-delete branch a
    /// deterministic test seam (otherwise we'd need to time a delete
    /// between the phase-1 SELECT and phase-2 `get` from outside the
    /// function, which the runtime gives no control over).
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
async fn ensure_libsql_index_specs_table(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_INDEX_SPECS_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error))?;
    Ok(())
}
async fn ensure_libsql_events_table(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_EVENTS_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::Append, error))?;
    Ok(())
}
async fn ensure_libsql_sequences_table(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_SEQUENCES_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::ReserveSeq, error))?;
    Ok(())
}
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
            // Mixed-variant bounds (e.g. `lo: I64(0)`, `hi: Text("x")`) have
            // no meaningful BETWEEN — reject closed rather than fall back to
            // lexicographic comparison. Matches the in-memory backend's
            // `discriminant(lo) == discriminant(hi)` requirement and keeps
            // cross-backend semantics aligned.
            if std::mem::discriminant(lo) != std::mem::discriminant(hi) {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                });
            }
            // PR #3659 review fix: guard the comparison with a JSON-type
            // check so a row whose stored value at `$.{key}` is a different
            // variant (e.g. text under a numeric range) does NOT participate
            // in BETWEEN. Without this guard a mixed-variant store can pull
            // unrelated values into the result set or fail the query
            // entirely on a cast failure.
            let lo_idx = bind_index_value(path, lo, params)?;
            let hi_idx = bind_index_value(path, hi, params)?;
            let json_type_guard = index_value_json_type_guard(key, lo);
            out.push_str(&format!(
                "({json_type_guard} \
                 AND json_extract(indexed, '$.{}') BETWEEN ?{lo_idx} AND ?{hi_idx})",
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

/// Build a `json_type(indexed, '$.{key}')`-shaped guard expression that
/// admits only rows whose stored value at `$.{key}` is the same JSON shape
/// as `value`. Used to guard `Filter::Range` so cross-variant stored values
/// don't participate in BETWEEN comparisons (PR #3659 review fix).
///
/// SQLite's `json_type` returns the literal strings `"true"` / `"false"` for
/// JSON booleans rather than `"boolean"`, so the bool guard checks for
/// either. A prior version emitted `= 'integer'` for `IndexValue::Bool`,
/// which never matched a stored boolean and silently dropped every row.
fn index_value_json_type_guard(key: &IndexKey, value: &IndexValue) -> String {
    let key = key.as_str();
    match value {
        IndexValue::Text(_) => format!("json_type(indexed, '$.{key}') = 'text'"),
        IndexValue::I64(_) => format!("json_type(indexed, '$.{key}') = 'integer'"),
        IndexValue::Bool(_) => {
            format!("json_type(indexed, '$.{key}') IN ('true', 'false')")
        }
        // Bytes can't reach this code: `bind_index_value` rejects Bytes
        // bounds with Unsupported before the guard is built.
        IndexValue::Bytes(_) => format!("json_type(indexed, '$.{key}') = 'text'"),
    }
}
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
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    if rows
        .next()
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(ddl, ())
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    Ok(())
}
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
const LIBSQL_INDEX_SPECS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_index_specs (
    prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    keys TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (prefix, name)
);
"#;
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
const LIBSQL_SEQUENCES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_sequences (
    path TEXT PRIMARY KEY,
    next_seq INTEGER NOT NULL CHECK (next_seq > 0),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

#[cfg(test)]
mod tests {
    //! Deterministic regression tests for libSQL behaviours that aren't
    //! easily exercised from the integration test surface (`tests/`),
    //! either because they need `pub(crate)` seams or because they
    //! manipulate state between internal phases. Cross-backend
    //! contract tests live in `tests/db_root_filesystem_contract.rs`;
    //! tests here cover internals that the integration surface can't
    //! reach.

    use super::*;
    use crate::libsql_pool::{LIBSQL_CONNECT_ATTEMPTS, connect_with_retry};
    use crate::{CasExpectation, Entry, RecordKind};
    use ironclaw_host_api::VirtualPath;

    async fn fresh_backend() -> (LibSqlRootFilesystem, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vector-test.db");
        let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem::new(db);
        fs.run_migrations().await.unwrap();
        (fs, dir)
    }

    #[tokio::test]
    async fn child_entries_query_uses_the_path_index_for_descendant_ranges() {
        let (fs, _dir) = fresh_backend().await;
        let parent = VirtualPath::new("/tenants/tenant/users/user/secrets/product-auth").unwrap();
        let (prefix_lower, prefix_upper) = descendant_path_range(&parent);
        assert_eq!(
            prefix_lower,
            "/tenants/tenant/users/user/secrets/product-auth/"
        );
        assert_eq!(
            prefix_upper,
            "/tenants/tenant/users/user/secrets/product-auth0"
        );
        let conn = fs.connect().await.unwrap();
        for query in [LIBSQL_CHILD_ENTRIES_SQL, LIBSQL_HAS_CHILD_ENTRY_SQL] {
            let explain_sql = format!("EXPLAIN QUERY PLAN {query}");
            let mut rows = conn
                .query(
                    &explain_sql,
                    libsql::params![prefix_lower.clone(), prefix_upper.clone()],
                )
                .await
                .unwrap();
            let mut details = Vec::new();
            while let Some(row) = rows.next().await.unwrap() {
                details.push(row.get::<String>(3).unwrap());
            }

            assert!(
                details.iter().any(|detail| {
                    detail.contains("SEARCH root_filesystem_entries USING")
                        && detail.contains("path>?")
                        && detail.contains("path<?")
                }),
                "descendant lookup must seek through the path index, plan: {details:?}"
            );
            assert!(
                details
                    .iter()
                    .all(|detail| !detail.contains("SCAN root_filesystem_entries")),
                "descendant lookup must not scan the complete path index, plan: {details:?}"
            );
        }
    }

    /// Drive the phase-2 materialize step directly with a synthesised
    /// ranked candidate list that includes a path which no longer exists
    /// in the backend. Locks in the "fail open on concurrent delete"
    /// branch in `vector_nearest_query` — between phase-1 ranking and
    /// the phase-2 `get`, a row may have been deleted by another writer;
    /// the query must skip that row rather than fail. We can't time a
    /// real concurrent delete from outside the function, so the
    /// extracted `materialize_ranked` seam stands in for it.
    #[tokio::test]
    async fn materialize_ranked_silently_skips_missing_paths() {
        let (fs, _dir) = fresh_backend().await;
        let present = VirtualPath::new("/memory/present").unwrap();
        let missing = VirtualPath::new("/memory/never_inserted").unwrap();

        // Only `present` is inserted — `missing` never exists in the DB,
        // which is exactly the state phase-2 sees if `missing` was ranked
        // in phase 1 but deleted before the get() call.
        let kind = RecordKind::new("chunk").unwrap();
        let entry = Entry::record(kind, &serde_json::json!({})).unwrap();
        fs.put(&present, entry, CasExpectation::Absent)
            .await
            .unwrap();

        let ranked = vec![
            (present.clone(), RecordVersion::from_backend(1), 0.9_f32),
            (missing.clone(), RecordVersion::from_backend(1), 0.5_f32),
        ];
        let out = fs.materialize_ranked(ranked).await.unwrap();
        // The missing row is dropped silently; the present row survives.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, present);
    }

    /// Companion to the test above: materialize_ranked must surface
    /// non-NotFound errors (anything other than the get-returns-None
    /// branch) rather than swallowing them. Empty ranked list short-
    /// circuits to an empty result without touching the DB — verify
    /// no implicit work happens for a no-op call.
    #[tokio::test]
    async fn materialize_ranked_empty_input_returns_empty_output() {
        let (fs, _dir) = fresh_backend().await;
        let out = fs.materialize_ranked(Vec::new()).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn connect_sets_busy_timeout_under_concurrent_file_backed_opens() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("connect-retry-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = Arc::new(LibSqlRootFilesystem::new(db));
        fs.run_migrations().await.unwrap();

        let mut handles = Vec::new();
        for _ in 0..10 {
            let fs = Arc::clone(&fs);
            handles.push(tokio::spawn(async move {
                let conn = fs.connect().await?;
                let mut rows = conn
                    .query("PRAGMA busy_timeout", ())
                    .await
                    .map_err(|error| {
                        infrastructure_libsql_error(FilesystemOperation::Stat, error)
                    })?;
                let row = rows
                    .next()
                    .await
                    .map_err(|error| infrastructure_libsql_error(FilesystemOperation::Stat, error))?
                    .ok_or_else(|| {
                        crate::db::infrastructure_error(
                            FilesystemOperation::Stat,
                            "PRAGMA busy_timeout returned no rows",
                        )
                    })?;
                let timeout: i64 = row.get(0).map_err(|error| {
                    crate::db::infrastructure_error(FilesystemOperation::Stat, error.to_string())
                })?;
                Ok::<_, FilesystemError>(timeout)
            }));
        }

        for handle in handles {
            let timeout = handle.await.unwrap().unwrap();
            assert_eq!(timeout, 5000);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_batch_surfaces_real_writer_contention_as_backend_busy() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("append-contention-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = Arc::new(LibSqlRootFilesystem {
            pool: crate::libsql_pool::build_libsql_pool_with_config(
                db,
                2,
                std::time::Duration::from_secs(1),
            ),
        });
        fs.run_migrations().await.unwrap();

        let writer = fs.connect().await.unwrap();
        writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();

        // Configure the pool's only other connection to fail quickly while
        // the first connection holds SQLite's single-writer lock.
        let contender = fs.connect().await.unwrap();
        let mut configured = contender
            .query("PRAGMA busy_timeout = 1", ())
            .await
            .unwrap();
        while configured.next().await.unwrap().is_some() {}
        drop(configured);
        let mut rows = contender.query("PRAGMA busy_timeout", ()).await.unwrap();
        let timeout_ms: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(timeout_ms, 1);
        drop(rows);
        drop(contender);

        let path = VirtualPath::new("/resources/deltas/log").unwrap();
        let append_fs = Arc::clone(&fs);
        let append_path = path.clone();
        let mut append = tokio::spawn(async move {
            append_fs
                .append_batch(&append_path, vec![b"delta".to_vec()])
                .await
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), &mut append).await;
        writer.execute("ROLLBACK", ()).await.unwrap();
        let joined = match result {
            Ok(joined) => joined,
            Err(_) => {
                append.abort();
                panic!("contended append batch must respect its busy timeout");
            }
        };
        let error = joined
            .expect("append task must not panic")
            .expect_err("the held writer lock must reject the append batch");

        assert!(matches!(
            error,
            FilesystemError::BackendBusy {
                path: error_path,
                operation: FilesystemOperation::Append,
            } if error_path == path
        ));
    }

    #[tokio::test]
    async fn connect_retries_transient_open_failures_before_succeeding() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("connect-retry-branch-test.db");
        let db = libsql::Builder::new_local(db_path).build().await.unwrap();
        let mut attempts = 0;

        let conn = connect_with_retry(|| {
            attempts += 1;
            if attempts < LIBSQL_CONNECT_ATTEMPTS {
                return Err(libsql::Error::ConnectionFailed(format!(
                    "synthetic transient failure {attempts}"
                )));
            }
            db.connect()
        })
        .await
        .unwrap();

        assert_eq!(attempts, LIBSQL_CONNECT_ATTEMPTS);
        let mut rows = conn.query("PRAGMA busy_timeout", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let timeout: i64 = row.get(0).unwrap();
        assert_eq!(timeout, 5000);
    }

    /// `run_migrations` must switch the database into WAL journaling, which
    /// is the property that lets readers run concurrently with the single
    /// writer instead of serialising behind a whole-file EXCLUSIVE lock.
    /// WAL is persisted in the file header, so this also asserts that a
    /// *fresh* connection opened after migration observes the mode — i.e.
    /// the setting stuck rather than applying only to the migration
    /// connection.
    #[tokio::test]
    async fn migrations_enable_wal_journal_mode() {
        let (fs, _dir) = fresh_backend().await;
        let conn = fs.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA journal_mode", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let mode: String = row.get(0).unwrap();
        assert_eq!(
            mode.to_ascii_lowercase(),
            "wal",
            "migrations must leave the database in WAL journaling mode"
        );
    }

    /// Every connection handed out by `connect` must carry the
    /// throughput-tuning PRAGMAs, not just `busy_timeout`. `synchronous`
    /// and `temp_store` are the two with stable, asserted numeric encodings
    /// (`NORMAL` = 1, `MEMORY` = 2); checking them confirms the whole batch
    /// was applied to the connection rather than silently skipped.
    #[tokio::test]
    async fn connect_applies_performance_pragmas() {
        let (fs, _dir) = fresh_backend().await;
        let conn = fs.connect().await.unwrap();

        let mut rows = conn.query("PRAGMA synchronous", ()).await.unwrap();
        let synchronous: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(synchronous, 1, "synchronous must be NORMAL (1)");

        let mut rows = conn.query("PRAGMA temp_store", ()).await.unwrap();
        let temp_store: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(temp_store, 2, "temp_store must be MEMORY (2)");

        let mut rows = conn.query("PRAGMA busy_timeout", ()).await.unwrap();
        let busy_timeout: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(busy_timeout, 5000, "busy_timeout must remain 5000");
    }

    /// A pool checkout that times out waiting for a free connection (every
    /// slot held by another in-flight operation) must surface as a
    /// `FilesystemOperation::Connect` infrastructure error through
    /// `connect()`'s `other` match arm — not panic, hang past the
    /// configured timeout, or lose the fact that this was a pool
    /// exhaustion rather than some other backend failure. Uses the
    /// `build_libsql_pool_with_config` test seam to build a deliberately
    /// tiny (size-1), fast-timing-out pool so the test doesn't wait out
    /// the real 10s production timeout.
    #[tokio::test]
    async fn connect_maps_pool_checkout_timeout_to_connect_infrastructure_error() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("checkout-timeout-test.db");
        let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem {
            pool: crate::libsql_pool::build_libsql_pool_with_config(
                db,
                1,
                std::time::Duration::from_millis(50),
            ),
        };
        fs.run_migrations().await.unwrap();

        // Hold the pool's only connection for the rest of the test.
        let _held = fs.connect().await.unwrap();

        // The pool has no free connection and none will be returned before
        // the 50ms wait_timeout elapses, so this checkout must time out
        // rather than hang or succeed.
        let Err(err) = fs.connect().await else {
            panic!("checkout must fail while the only connection is held");
        };
        match err {
            FilesystemError::BackendInfrastructure { operation, reason } => {
                assert_eq!(operation, FilesystemOperation::Connect);
                assert!(
                    !reason.is_empty(),
                    "checkout-timeout reason must not be empty"
                );
            }
            other => panic!("expected FilesystemError::BackendInfrastructure, got {other:?}"),
        }
    }

    /// Deterministic, single-task regression pin for the atomicity fix
    /// (commit 1792aebb2 / PR #5749 round 4): `delete_if_version`'s
    /// zero-rows diagnosis must reuse the SAME connection the conditional
    /// DELETE ran on, not check out a second one. Round-B review finding:
    /// the concurrency storm test in `tests/concurrent_cas_storm.rs`
    /// doesn't actually discriminate this — every racer shares one
    /// pre-fetched version and nothing recreates the path mid-round, so
    /// it passes with or without the fix. This test does discriminate it,
    /// with no concurrency required: build a deliberately size-1 pool (via
    /// `build_libsql_pool_with_config`), let `delete_if_version` check out
    /// its only connection, and hit the stale-version (0-rows) branch. If
    /// the diagnosis internally called `self.connect()` again — the
    /// pre-fix pattern — that second checkout would deadlock against the
    /// first (nothing else can return the only connection) and time out;
    /// reusing the passed-in `conn` completes immediately.
    #[tokio::test]
    async fn delete_if_version_diagnosis_reuses_the_delete_connection_under_a_size_one_pool() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("delete-single-conn-test.db");
        let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem {
            pool: crate::libsql_pool::build_libsql_pool_with_config(
                db,
                1,
                std::time::Duration::from_millis(200),
            ),
        };
        fs.run_migrations().await.unwrap();

        let path = VirtualPath::new("/secrets/single-conn").unwrap();
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();

        // Stale version drives the 0-rows branch, which must diagnose
        // NotFound/VersionMismatch via `current_version_libsql(conn, ...)`
        // on the connection already checked out above, not a second
        // checkout — a second checkout would time out against the
        // size-1 pool's only (self-held) connection.
        let stale = RecordVersion::from_backend(v1.get() + 1);
        let err = fs.delete_if_version(&path, stale).await.unwrap_err();
        assert!(
            matches!(err, FilesystemError::VersionMismatch { .. }),
            "expected VersionMismatch (proves the diagnosis ran to \
             completion without deadlocking on the size-1 pool), got: {err:?}"
        );

        // Round-C review: the assertion above only proves the diagnosis
        // didn't deadlock: `ROLLBACK` itself could still fail to run (or
        // fail and leave the connection mid-transaction) without failing
        // that assertion. Prove the connection actually came back to the
        // size-1 pool in a clean, reusable state by checking it out again
        // for a real CAS delete — a still-open transaction from the
        // VersionMismatch path would make this second call either hang
        // against the size-1 pool or fail on a nested-transaction error.
        fs.delete_if_version(&path, v1)
            .await
            .expect("connection must return to the size-1 pool clean after a VersionMismatch, not deadlock or error on a leftover transaction");
        assert!(fs.get(&path).await.unwrap().is_none());
    }
}
