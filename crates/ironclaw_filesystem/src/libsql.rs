// arch-exempt: large_file, targeted libSQL contention regression stays with its backend, plan #4088
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex as StdMutex},
};

use async_trait::async_trait;
use ironclaw_host_api::path::VirtualPath;
use ironclaw_libsql_runtime::{
    LibSqlCheckoutFailureReason, LibSqlLane, LibSqlReadConnectionLease, LibSqlRuntime,
    LibSqlRuntimeError, LibSqlWriteConnectionLease,
};

use crate::backend::{EventRecord, StorageTxn};
use crate::db::{
    descendant_path_range, direct_children, directory_append_error, directory_write_error,
    escape_like_literal, escape_like_with_trailing_wildcard, infrastructure_libsql_error,
    is_not_found, libsql_db_error, not_found, page_offset_to_i64, record_version_from_i64,
    record_version_to_i64, sql_index_name, system_time_from_unix_seconds, virtual_path_prefixes,
};
use crate::vector::{cosine_similarity, decode_embedding_blob};
use crate::{
    AtomicSubtreeEntry, BackendCapabilities, Capability, CasExpectation, ContentType, DirEntry,
    Entry, FileStat, FileType, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind,
    IndexName, IndexSpec, IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, SeqNo,
    TxnCapability, VersionedEntry, root::validate_atomic_subtree_entries,
};
/// libSQL-backed [`RootFilesystem`] storing file contents by virtual path.
#[derive(Debug)]
pub struct LibSqlRootFilesystem {
    runtime: Arc<LibSqlRuntime>,
    index_ddl_lock: tokio::sync::Mutex<()>,
    projection_specs: StdMutex<HashMap<(String, IndexName), IndexSpec>>,
}
const LIBSQL_CHILD_ENTRIES_SQL: &str = "SELECT path, length(contents), is_dir \
    FROM root_filesystem_entries \
    WHERE path >= ?1 AND path < ?2 \
    ORDER BY path";
const LIBSQL_HAS_CHILD_ENTRY_SQL: &str = "SELECT 1 \
    FROM root_filesystem_entries \
    WHERE path >= ?1 AND path < ?2 \
    LIMIT 1";
// Descendant-prefix predicates for record reads. A range over the primary key
// seeks the path index; `LIKE ... ESCAPE` cannot use it, so the same predicate
// degrades to a full scan of `root_filesystem_entries` whose cost grows with
// the total row count of the database rather than with what the caller asked
// for. `descendant_path_range` supplies the bounds: '/' sorts before '0', so
// ["{prefix}/", "{prefix}0") is exactly the descendant set under the BINARY
// collation these paths use -- and it needs no LIKE escaping at all.
const RECORD_QUERY_PREFIX_SQL: &str = "SELECT path, contents, content_type, kind, indexed, version \
     FROM root_filesystem_entries \
     WHERE is_dir = 0 AND (path = ?1 OR (path >= ?2 AND path < ?3))";
const INDEXED_QUERY_PREFIX_SQL: &str = "SELECT path, indexed, version \
     FROM root_filesystem_entries \
     WHERE is_dir = 0 AND (path = ?1 OR (path >= ?2 AND path < ?3))";
impl LibSqlRootFilesystem {
    pub fn new(db: Arc<libsql::Database>) -> Result<Self, FilesystemError> {
        let runtime = LibSqlRuntime::new(db).map_err(map_runtime_connection_error)?;
        Ok(Self::from_runtime(Arc::new(runtime)))
    }

    pub fn from_runtime(runtime: Arc<LibSqlRuntime>) -> Self {
        Self {
            runtime,
            index_ddl_lock: tokio::sync::Mutex::new(()),
            projection_specs: StdMutex::new(HashMap::new()),
        }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self.migration_write_connection().await?;
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
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error)
            })?;
        run_libsql_migrations_inner(&transaction).await?;
        transaction
            .commit()
            .await
            .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))
    }

    async fn read_connection(&self) -> Result<LibSqlReadConnectionLease, FilesystemError> {
        self.runtime
            .read()
            .await
            .map_err(map_runtime_connection_error)
    }

    async fn migration_write_connection(
        &self,
    ) -> Result<LibSqlWriteConnectionLease, FilesystemError> {
        self.runtime
            .write()
            .await
            .map_err(map_runtime_connection_error)
    }

    async fn write_connection(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<LibSqlWriteConnectionLease, FilesystemError> {
        self.runtime
            .write()
            .await
            .map_err(|error| map_runtime_write_connection_error(path.clone(), operation, error))
    }
}

fn map_runtime_connection_error(
    error: ironclaw_libsql_runtime::LibSqlRuntimeError,
) -> FilesystemError {
    let reason = error.to_string();
    let source = format_error_source_chain(&error);
    tracing::debug!(
        %reason,
        source = source.as_deref().unwrap_or("none"),
        "libSQL root filesystem connection checkout failed"
    );
    crate::db::infrastructure_error(FilesystemOperation::Connect, reason)
}

fn map_runtime_write_connection_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: LibSqlRuntimeError,
) -> FilesystemError {
    let retryable_admission_timeout = matches!(
        &error,
        LibSqlRuntimeError::Checkout {
            lane: LibSqlLane::Write,
            reason: LibSqlCheckoutFailureReason::Timeout,
        }
    );
    let reason = error.to_string();
    let source = format_error_source_chain(&error);
    tracing::debug!(
        %operation,
        %reason,
        source = source.as_deref().unwrap_or("none"),
        retryable_admission_timeout,
        "libSQL root filesystem writer checkout failed"
    );
    if retryable_admission_timeout {
        FilesystemError::BackendBusy { path, operation }
    } else {
        crate::db::infrastructure_error(operation, reason)
    }
}

fn format_error_source_chain(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut source = error.source();
    let mut reason = source.map(ToString::to_string)?;
    source = source.and_then(std::error::Error::source);
    while let Some(error) = source {
        reason.push_str(": ");
        reason.push_str(&error.to_string());
        source = error.source();
    }
    Some(reason)
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
            .with_txn(TxnCapability::MultiKey)
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

        let conn = self
            .write_connection(path, FilesystemOperation::WriteFile)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
            })?;
        let version = put_libsql_inner(
            &transaction,
            path,
            body,
            content_type_str,
            kind_str,
            indexed_json,
            cas,
        )
        .await?;
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
        })?;
        Ok(version)
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let conn = self.read_connection().await?;
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
        Ok(Some(VersionedEntry {
            path: path.clone(),
            entry: build_entry(path, body, content_type_raw, kind_raw, indexed_raw)?,
            version: record_version_from_i64(path, version_raw)?,
        }))
    }

    async fn create_subtree_atomic(
        &self,
        prefix: &VirtualPath,
        entries: Vec<AtomicSubtreeEntry>,
    ) -> Result<Vec<RecordVersion>, FilesystemError> {
        validate_atomic_subtree_entries(prefix, &entries)?;
        let conn = self
            .write_connection(prefix, FilesystemOperation::CreateSubtreeAtomic)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(
                    prefix.clone(),
                    FilesystemOperation::CreateSubtreeAtomic,
                    error,
                )
            })?;

        let (lower, upper) = descendant_path_range(prefix);
        let mut existing = transaction
            .query(
                "SELECT path, version FROM root_filesystem_entries \
                 WHERE path = ?1 OR (path >= ?2 AND path < ?3) LIMIT 1",
                libsql::params![prefix.as_str(), lower, upper],
            )
            .await
            .map_err(|error| {
                libsql_db_error(
                    prefix.clone(),
                    FilesystemOperation::CreateSubtreeAtomic,
                    error,
                )
            })?;
        if let Some(row) = existing.next().await.map_err(|error| {
            libsql_db_error(
                prefix.clone(),
                FilesystemOperation::CreateSubtreeAtomic,
                error,
            )
        })? {
            let version_raw: i64 = row.get(1).map_err(|error| {
                libsql_db_error(
                    prefix.clone(),
                    FilesystemOperation::CreateSubtreeAtomic,
                    error,
                )
            })?;
            return Err(FilesystemError::VersionMismatch {
                path: prefix.clone(),
                expected: None,
                found: Some(record_version_from_i64(prefix, version_raw)?),
            });
        }
        drop(existing);

        let mut versions = Vec::with_capacity(entries.len());
        for item in entries {
            let indexed_json = serde_json::to_string(&item.entry.indexed).map_err(|_| {
                FilesystemError::SerializeIndexed {
                    path: item.path.clone(),
                    operation: FilesystemOperation::CreateSubtreeAtomic,
                }
            })?;
            let kind = item
                .entry
                .kind
                .as_ref()
                .map(|value| value.as_str().to_string());
            let content_type = item.entry.content_type.as_str().to_string();
            versions.push(
                put_libsql_inner(
                    &transaction,
                    &item.path,
                    item.entry.body,
                    content_type,
                    kind,
                    indexed_json,
                    CasExpectation::Absent,
                )
                .await?,
            );
        }
        transaction.commit().await.map_err(|error| {
            libsql_db_error(
                prefix.clone(),
                FilesystemOperation::CreateSubtreeAtomic,
                error,
            )
        })?;
        Ok(versions)
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
        let shared_projection = matches!(spec.kind, IndexKind::Exact | IndexKind::Prefix);
        let projection_key = (path.as_str().to_string(), spec.name.clone());
        if shared_projection {
            let cache = self
                .projection_specs
                .lock()
                .map_err(|_| FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::EnsureIndex,
                    reason: "projection index cache mutex poisoned".to_string(),
                })?;
            if let Some(existing) = cache.get(&projection_key) {
                return if existing == spec {
                    Ok(())
                } else {
                    Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    })
                };
            }
        }
        let _ddl_guard = self.index_ddl_lock.lock().await;
        if shared_projection {
            let cache = self
                .projection_specs
                .lock()
                .map_err(|_| FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::EnsureIndex,
                    reason: "projection index cache mutex poisoned".to_string(),
                })?;
            if let Some(existing) = cache.get(&projection_key) {
                return if existing == spec {
                    Ok(())
                } else {
                    Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    })
                };
            }
        }
        let catalog_prefix = path.as_str();
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

        let conn = self
            .write_connection(path, FilesystemOperation::EnsureIndex)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
            })?;
        // PR #3661 reviewer fix: the prior SELECT-then-INSERT was racey.
        // Two processes declaring the same spec concurrently could both
        // miss the row and then one would hit a unique-constraint backend
        // error instead of getting the promised idempotent success.
        //
        // Fix: INSERT ... ON CONFLICT DO NOTHING in a single round-trip,
        // then read back the canonical row and compare. If the stored
        // spec matches ours we're idempotent; if it differs we surface
        // IndexConflict.
        transaction
            .execute(
                "INSERT INTO root_filesystem_index_specs (prefix, name, keys, kind) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (prefix, name) DO NOTHING",
                libsql::params![
                    catalog_prefix,
                    spec.name.as_str(),
                    keys_json.clone(),
                    kind_str.clone(),
                ],
            )
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
            })?;

        // Read back what's there and validate it matches.
        let mut rows = transaction
            .query(
                "SELECT keys, kind FROM root_filesystem_index_specs WHERE prefix = ?1 AND name = ?2",
                libsql::params![catalog_prefix, spec.name.as_str()],
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
                ensure_libsql_ordered_projection(&conn, path, spec).await?;
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
                transaction
                    .execute(&create_vtab, ())
                    .await
                    .map_err(|error| {
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
                transaction
                    .execute(&trigger_insert, ())
                    .await
                    .map_err(|error| {
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
                transaction
                    .execute(&trigger_update, ())
                    .await
                    .map_err(|error| {
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
                transaction
                    .execute(&trigger_delete, ())
                    .await
                    .map_err(|error| {
                        libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
                    })?;
                // Backfill any rows present before the index was declared.
                let backfill = format!(
                    "INSERT INTO {fts_table}(path, content) \
                     SELECT path, COALESCE(json_extract(indexed, '$.{fts_key}'), '') \
                     FROM root_filesystem_entries \
                     WHERE is_dir = 0 \
                       AND (path = ?1 OR (path >= ?2 AND path < ?3)) \
                       AND NOT EXISTS \
                           (SELECT 1 FROM {fts_table} WHERE {fts_table}.path = root_filesystem_entries.path)"
                );
                let (backfill_lower, backfill_upper) = descendant_path_range(path);
                transaction
                    .execute(
                        &backfill,
                        libsql::params![path_prefix, backfill_lower, backfill_upper],
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
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::EnsureIndex, error)
        })?;
        if shared_projection {
            self.projection_specs
                .lock()
                .map_err(|_| FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::EnsureIndex,
                    reason: "projection index cache mutex poisoned".to_string(),
                })?
                .insert(projection_key, spec.clone());
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
        let (prefix_lower, prefix_upper) = descendant_path_range(path);
        params.push(libsql::Value::Text(prefix_lower));
        params.push(libsql::Value::Text(prefix_upper));

        let mut conditions = String::new();
        translate_filter(path, filter, &mut conditions, &mut params, &fts_tables)?;

        let mut sql = String::from(RECORD_QUERY_PREFIX_SQL);
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

        let conn = self.read_connection().await?;
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &crate::OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let conn = self.read_connection().await?;
        // Resolve the spec against `path` and every ancestor prefix, most
        // specific first, so a caller may declare the index once on a higher
        // prefix and query a child path. `/shared` keeps its existing
        // precedence over every path-derived candidate. The candidate list is
        // bounded by path depth, so this stays a keyed lookup, not a scan.
        let candidate_prefixes = crate::index::ancestor_prefixes(path.as_str());
        let mut spec_params: Vec<libsql::Value> = candidate_prefixes
            .iter()
            .map(|prefix| libsql::Value::Text((*prefix).to_string()))
            .collect();
        spec_params.push(libsql::Value::Text("/shared".to_string()));
        let spec_placeholders: Vec<String> = (1..=spec_params.len())
            .map(|position| format!("?{position}"))
            .collect();
        let name_placeholder = format!("?{}", spec_params.len() + 1);
        spec_params.push(libsql::Value::Text(page.index.as_str().to_string()));
        let spec_sql = format!(
            "SELECT keys, kind FROM root_filesystem_index_specs \
             WHERE prefix IN ({}) AND name = {name_placeholder} \
             ORDER BY CASE WHEN prefix = '/shared' THEN 0 ELSE 1 END, LENGTH(prefix) DESC \
             LIMIT 1",
            spec_placeholders.join(", ")
        );
        let mut spec_rows = conn
            .query(&spec_sql, spec_params)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?;
        let spec = if let Some(row) = spec_rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Query, error))?
        {
            let keys_json: String = row.get(0).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let kind: String = row.get(1).map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::Query, error)
            })?;
            let keys = serde_json::from_str::<Vec<String>>(&keys_json)
                .map_err(|_| FilesystemError::DeserializeIndexed {
                    path: path.clone(),
                    operation: FilesystemOperation::Query,
                })?
                .into_iter()
                .map(IndexKey::new)
                .collect::<Result<Vec<_>, _>>()?;
            let kind = match kind.as_str() {
                "exact" => IndexKind::Exact,
                "prefix" => IndexKind::Prefix,
                _ => {
                    return Err(FilesystemError::Unsupported {
                        path: path.clone(),
                        operation: FilesystemOperation::Query,
                    });
                }
            };
            Some(IndexSpec::new(page.index.clone(), keys, kind))
        } else {
            None
        };
        drop(spec_rows);
        let Some(spec) = spec else {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Query,
            });
        };
        let Some(prefix_values) = crate::index::ordered_query_prefix_values(&spec, filter, page)
        else {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Query,
            });
        };
        let sort_position = prefix_values.len();
        let tie_position = sort_position.saturating_add(1);
        if tie_position >= 8 {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Query,
            });
        }
        let expression = format!("ordered.k{sort_position}");
        let tie_expression = format!("ordered.k{tie_position}");
        let mut params = vec![
            libsql::Value::Text(page.index.as_str().to_string()),
            libsql::Value::Text(path.as_str().to_string()),
        ];
        let prefix_pattern = format!("{}/%", path.as_str().trim_end_matches('/'));
        params.push(libsql::Value::Text(escape_like_with_trailing_wildcard(
            &prefix_pattern,
        )));
        let mut sql = String::from(
            "SELECT entry.path, entry.contents, entry.content_type, entry.kind, \
                    entry.indexed, entry.version \
             FROM root_filesystem_ordered_index_rows AS ordered \
             JOIN root_filesystem_entries AS entry ON entry.path = ordered.path \
             WHERE ordered.index_name = ?1 \
               AND (ordered.path = ?2 OR ordered.path LIKE ?3 ESCAPE '!')",
        );
        for (position, value) in prefix_values.iter().enumerate() {
            let value_index = bind_index_value(path, value, &mut params)?;
            sql.push_str(&format!(" AND ordered.k{position} = ?{value_index}"));
        }
        if let Some(cursor) = &page.after {
            let value_index = bind_index_value(path, &cursor.value, &mut params)?;
            let tie_index = bind_index_value(path, &cursor.tie_breaker, &mut params)?;
            let comparison = match page.direction {
                crate::SortDirection::Ascending => ">",
                crate::SortDirection::Descending => "<",
            };
            sql.push_str(&format!(
                " AND ({expression} {comparison} ?{value_index} \
                 OR ({expression} = ?{value_index} AND {tie_expression} {comparison} ?{tie_index}))"
            ));
        }
        let direction = match page.direction {
            crate::SortDirection::Ascending => "ASC",
            crate::SortDirection::Descending => "DESC",
        };
        params.push(libsql::Value::Integer(i64::from(
            page.limit.min(crate::Page::MAX_LIMIT),
        )));
        let limit_index = params.len();
        sql.push_str(&format!(
            " ORDER BY {expression} {direction}, {tie_expression} {direction} LIMIT ?{limit_index}"
        ));

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
        let conn = self.read_connection().await?;
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
        let conn = self.read_connection().await?;
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
        let conn = self
            .write_connection(path, FilesystemOperation::WriteFile)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
            })?;
        // PR #3660 reviewer fix: legacy write_file must also reset the
        // record metadata (content_type / kind / indexed) and bump the
        // version, otherwise a get() after a write_file-overwrite of a
        // previously record-shaped entry returns stale metadata. Treat
        // legacy writes as opaque-file entries: kind=NULL, indexed='{}',
        // content_type=application/octet-stream, version bumped from the
        // current row's version (or 1 for new entries).
        if matches!(
            exact_entry_libsql(&transaction, path).await?,
            Some((_, FileType::Directory, _))
        ) || has_child_entry_libsql(&transaction, path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        let rows = transaction
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
        transaction
            .commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error))
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let conn = self
            .write_connection(path, FilesystemOperation::AppendFile)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error)
            })?;
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
        if matches!(
            exact_entry_libsql(&transaction, path).await?,
            Some((_, FileType::Directory, _))
        ) || has_child_entry_libsql(&transaction, path).await?
        {
            return Err(directory_append_error(path.clone()));
        }
        transaction
            .execute(
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
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error)
            })?;
        transaction
            .commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))
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
        let conn = self
            .write_connection(path, FilesystemOperation::Delete)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        delete_libsql_inner(&transaction, path).await?;
        transaction
            .commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))
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
        // single writer lease, matching the one-lease-per-call-stack invariant
        // the shared libSQL runtime enforces (no nested writer acquisition).
        //
        // Round-A review: validate `expected_version` before taking the
        // writer checkout / write lock. An out-of-range version can never
        // match a real row, so failing closed here avoids holding a
        // contended connection (and SQLite's write lock) for a call
        // destined to error — relevant under the concurrent CAS storms
        // this pool exists to survive.
        let expected_raw = record_version_to_i64(path, expected_version)?;
        let conn = self
            .write_connection(path, FilesystemOperation::Delete)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        #[cfg(test)]
        tests::pause_delete_if_version_after_transaction_begin(
            Arc::as_ptr(&self.runtime) as usize,
            path,
        )
        .await;
        delete_if_version_libsql_inner(&transaction, path, expected_version, expected_raw).await?;
        transaction
            .commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let conn = self
            .write_connection(path, FilesystemOperation::BeginTxn)
            .await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::BeginTxn, error))?;
        Ok(Box::new(LibSqlStorageTxn {
            conn: Some(conn),
            prefix: path.clone(),
            active: true,
        }))
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let conn = self
            .write_connection(path, FilesystemOperation::Append)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        let seq = append_libsql_inner(&transaction, path, payload).await?;
        #[cfg(test)]
        tests::pause_append_after_insert(Arc::as_ptr(&self.runtime) as usize, path).await;
        transaction
            .commit()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
        Ok(seq)
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .write_connection(path, FilesystemOperation::Append)
            .await?;
        // One multi-row INSERT per chunk collapses N appends into one round-trip.
        // `seq` is INTEGER PRIMARY KEY AUTOINCREMENT, assigned in VALUES order;
        // `RETURNING seq` then sorted ASC recovers payload order
        // deterministically. Chunk the batch so the bound parameter count
        // (2 per row) stays well under SQLite's default 999-parameter limit.
        // An immediate RAII transaction acquires SQLite's writer lock before
        // any batch work and rolls back automatically if this future is
        // cancelled. The shared writer lease prevents in-process competition;
        // an external writer still surfaces as retryable contention.
        const ROWS_PER_STATEMENT: usize = 256;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
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
            let mut rows = transaction.query(&sql, params).await.map_err(|error| {
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
        transaction
            .commit()
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
        let conn = self.read_connection().await?;
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
        let conn = self.read_connection().await?;
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
        let conn = self
            .write_connection(path, FilesystemOperation::ReserveSeq)
            .await?;
        reserve_sequence_libsql_inner(&conn, path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self
            .write_connection(path, FilesystemOperation::CreateDirAll)
            .await?;
        let transaction = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
            })?;
        create_dir_all_libsql_inner(&transaction, path).await?;
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })
    }
}
struct LibSqlStorageTxn {
    conn: Option<LibSqlWriteConnectionLease>,
    prefix: VirtualPath,
    active: bool,
}

impl LibSqlStorageTxn {
    fn conn(&self) -> Result<&libsql::Connection, FilesystemError> {
        self.conn
            .as_deref()
            .ok_or_else(|| FilesystemError::Backend {
                path: self.prefix.clone(),
                operation: FilesystemOperation::BeginTxn,
                reason: "libSQL transaction already finished".to_string(),
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

#[async_trait]
impl StorageTxn for LibSqlStorageTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check_path(path)?;
        let indexed_json = serde_json::to_string(&entry.indexed).map_err(|_| {
            FilesystemError::SerializeIndexed {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            }
        })?;
        let kind_str = entry.kind.as_ref().map(|kind| kind.as_str().to_string());
        let content_type_str = entry.content_type.as_str().to_string();
        put_libsql_inner(
            self.conn()?,
            path,
            entry.body,
            content_type_str,
            kind_str,
            indexed_json,
            cas,
        )
        .await
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check_path(path)?;
        get_libsql_inner(self.conn()?, path).await
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check_path(path)?;
        delete_libsql_inner(self.conn()?, path).await
    }

    async fn reserve_sequence(&mut self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.check_path(path)?;
        reserve_sequence_libsql_inner(self.conn()?, path).await
    }

    async fn reserve_sequence_range(
        &mut self,
        path: &VirtualPath,
        count: u64,
    ) -> Result<SeqNo, FilesystemError> {
        self.check_path(path)?;
        reserve_sequence_range_libsql_inner(self.conn()?, path, count).await
    }

    async fn commit(mut self: Box<Self>) -> Result<(), FilesystemError> {
        let conn = self.conn.take().ok_or_else(|| FilesystemError::Backend {
            path: self.prefix.clone(),
            operation: FilesystemOperation::BeginTxn,
            reason: "libSQL transaction already finished".to_string(),
        })?;
        match conn.execute("COMMIT", ()).await {
            Ok(_) => {
                self.active = false;
                Ok(())
            }
            Err(error) => {
                let mapped =
                    libsql_db_error(self.prefix.clone(), FilesystemOperation::BeginTxn, error);
                let _ = conn.execute("ROLLBACK", ()).await;
                self.active = false;
                Err(mapped)
            }
        }
    }

    async fn rollback(mut self: Box<Self>) {
        if let Some(conn) = self.conn.take()
            && self.active
        {
            let _ = conn.execute("ROLLBACK", ()).await;
            self.active = false;
        }
    }
}

impl Drop for LibSqlStorageTxn {
    fn drop(&mut self) {
        if self.active
            && let Some(conn) = self.conn.take()
        {
            // A destructor cannot await ROLLBACK. Discard the connection so
            // SQLite rolls back while closing it and the pool can create a
            // clean replacement without a detached task retaining its only
            // writer lease.
            conn.discard();
        }
    }
}

async fn get_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<Option<VersionedEntry>, FilesystemError> {
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
        return Ok(None);
    }
    let body = row
        .get(0)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    let content_type = row
        .get(2)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    let kind = row.get(3).ok();
    let indexed = row
        .get(4)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    let version: i64 = row
        .get(5)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
    Ok(Some(VersionedEntry {
        path: path.clone(),
        entry: build_entry(path, body, content_type, kind, indexed)?,
        version: record_version_from_i64(path, version)?,
    }))
}

async fn delete_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    // Range bounds let every subtree sweep seek its path index. Keep this
    // shared helper so root and transactional deletes retain identical
    // semantics.
    let (prefix_lower, prefix_upper) = descendant_path_range(path);
    let deleted = conn
        .execute(
            "DELETE FROM root_filesystem_entries \
             WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
            libsql::params![path.as_str(), prefix_lower.clone(), prefix_upper.clone()],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
    if deleted == 0 {
        return Err(not_found(path.clone(), FilesystemOperation::Delete));
    }
    conn.execute(
        "DELETE FROM root_filesystem_events \
         WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
        libsql::params![path.as_str(), prefix_lower.clone(), prefix_upper.clone()],
    )
    .await
    .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
    conn.execute(
        "DELETE FROM root_filesystem_sequences \
         WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
        libsql::params![path.as_str(), prefix_lower, prefix_upper],
    )
    .await
    .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
    Ok(())
}

async fn append_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
    payload: Vec<u8>,
) -> Result<SeqNo, FilesystemError> {
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
    let seq: i64 = row
        .get(0)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Append, error))?;
    seq_no_from_i64(path, seq, FilesystemOperation::Append)
}

async fn reserve_sequence_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
) -> Result<SeqNo, FilesystemError> {
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
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?;
    let row = rows
        .next()
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?
        .ok_or_else(|| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReserveSeq,
            reason: "sequence reservation returned no row".to_string(),
        })?;
    let seq: i64 = row
        .get(0)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?;
    seq_no_from_i64(path, seq, FilesystemOperation::ReserveSeq)
}

async fn reserve_sequence_range_libsql_inner(
    conn: &libsql::Connection,
    path: &VirtualPath,
    count: u64,
) -> Result<SeqNo, FilesystemError> {
    if count == 0 {
        return Ok(SeqNo::ZERO);
    }
    let count = i64::try_from(count).map_err(|_| FilesystemError::Backend {
        path: path.clone(),
        operation: FilesystemOperation::ReserveSeq,
        reason: "sequence reservation range exceeds i64".to_string(),
    })?;
    let mut rows = conn
        .query(
            r#"
            INSERT INTO root_filesystem_sequences (path, next_seq, updated_at)
            VALUES (?1, ?2 + 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(path) DO UPDATE SET
                next_seq = root_filesystem_sequences.next_seq + ?2,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            RETURNING next_seq - 1
            "#,
            libsql::params![path.as_str(), count],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?;
    let row = rows
        .next()
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?
        .ok_or_else(|| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReserveSeq,
            reason: "sequence range reservation returned no row".to_string(),
        })?;
    let seq: i64 = row
        .get(0)
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReserveSeq, error))?;
    seq_no_from_i64(path, seq, FilesystemOperation::ReserveSeq)
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
/// immediate transaction, with rollback-on-drop for every non-commit path.
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
/// whole sequence in one immediate transaction with rollback-on-drop.
async fn run_libsql_migrations_inner(conn: &libsql::Connection) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_ROOT_FILESYSTEM_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::CreateDirAll, error))?;
    ensure_libsql_root_is_dir_column(conn).await?;
    ensure_libsql_records_columns(conn).await?;
    ensure_libsql_index_specs_table(conn).await?;
    ensure_libsql_ordered_index_table(conn).await?;
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
        let conn = self.read_connection().await?;
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
        let conn = self.read_connection().await?;
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
        let conn = self.read_connection().await?;
        let (prefix_lower, prefix_upper) = descendant_path_range(parent);
        let mut rows = conn
            .query(
                LIBSQL_HAS_CHILD_ENTRY_SQL,
                libsql::params![prefix_lower, prefix_upper],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        rows.next()
            .await
            .map(|row| row.is_some())
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))
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
        let conn = self.read_connection().await?;
        let mut out = std::collections::HashMap::new();
        // Scan the spec catalog for FTS specs whose prefix is path or any
        // ancestor (so callers may declare the index on a higher prefix
        // and query a child path).
        let candidate_prefixes = crate::index::ancestor_prefixes(path.as_str());
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
            .map(|p| libsql::Value::Text((*p).to_string()))
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
        let conn = self.read_connection().await?;
        let (prefix_lower, prefix_upper) = descendant_path_range(path);
        let mut rows = conn
            .query(
                INDEXED_QUERY_PREFIX_SQL,
                libsql::params![path.as_str(), prefix_lower, prefix_upper],
            )
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
async fn ensure_libsql_ordered_index_table(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    conn.execute_batch(LIBSQL_ORDERED_INDEX_SCHEMA)
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error))?;
    Ok(())
}

async fn ensure_libsql_ordered_projection(
    conn: &libsql::Connection,
    path: &VirtualPath,
    spec: &IndexSpec,
) -> Result<(), FilesystemError> {
    if spec.keys.len() > crate::index::MAX_ORDERED_INDEX_KEYS {
        return Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::EnsureIndex,
        });
    }
    ensure_libsql_static_ordered_projection(conn)
        .await
        .map_err(|error| match error {
            // The static ensure reports path-less infrastructure errors; the
            // declaration seam's contract is a path-carrying backend error.
            FilesystemError::Backend {
                operation, reason, ..
            }
            | FilesystemError::BackendInfrastructure { operation, reason } => {
                FilesystemError::Backend {
                    path: path.clone(),
                    operation,
                    reason,
                }
            }
            other => other,
        })
}

/// Projection triggers are static: three triggers total, whose bodies project
/// by joining the spec catalog on prefix containment. The previous design
/// installed three triggers per (declaration prefix, spec); every entry write
/// then evaluated every trigger ever declared — O(total declarations) per
/// statement (measured 3,750 triggers and ~38ms/statement after a 50-user
/// run). The catalog join is O(catalog rows) with a tiny constant, and
/// declaration becomes a pure catalog insert.
///
/// Semantics are unchanged, including "declaration never backfills": triggers
/// only fire on writes after the fact, and a spec projects exactly the
/// subtree of its declared prefix.
async fn ensure_libsql_static_ordered_projection(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    // Drop the legacy per-declaration triggers exactly once per database.
    // SQLite has no dynamic DDL in plain SQL, so the sweep is done here; it
    // is cheap when nothing matches (single catalog scan).
    let mut legacy = conn
        .query(
            "SELECT name FROM sqlite_master \
             WHERE type = 'trigger' \
               AND (name LIKE 'idx_rfs_%' \
                    OR (name LIKE 'rfs_ordered_projection_%' \
                        AND name NOT LIKE 'rfs_ordered_projection_v3_%')) \
               AND sql LIKE '%root_filesystem_ordered_index_rows%'",
            (),
        )
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error))?;
    let mut drops = String::new();
    while let Some(row) = legacy
        .next()
        .await
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error))?
    {
        let name: String = row.get(0).map_err(|error| {
            infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error)
        })?;
        // The name is interpolated into DDL unquoted, so the ASCII
        // alphanumeric/underscore check below is the whole safeguard, not a
        // belt-and-braces extra: relaxing it would admit injection.
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            drops.push_str(&format!("DROP TRIGGER IF EXISTS {name};"));
        }
    }
    drop(legacy);
    if !drops.is_empty() {
        conn.execute_batch(&drops).await.map_err(|error| {
            infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error)
        })?;
    }

    // Containment: spec.prefix is the path itself or a proper ancestor. The
    // half-open band `(prefix || '/', prefix || '0')` is the range form of
    // `LIKE prefix || '/%'` without wildcard-injection from stored prefixes;
    // '/' needs its own arm because '//' breaks the band trick.
    // Ancestors of the written path, walked by string surgery: rtrim with the
    // path's own non-slash characters strips the trailing segment, leaving
    // ".../" to trim. Joining the catalog by equality on these turns the
    // projection into an index seek on the `(prefix, name)` key. Comparing
    // every catalog row against the path instead — which is what a
    // containment predicate does — scans every declaration ever made, and an
    // upgraded database keeps its per-thread declaration rows forever.
    let ancestors = "WITH RECURSIVE rfs_ancestors(prefix) AS ( \
           SELECT new.path \
           UNION ALL \
           SELECT CASE \
             WHEN length(rtrim(prefix, replace(prefix, '/', ''))) <= 1 THEN '/' \
             ELSE substr( \
               rtrim(prefix, replace(prefix, '/', '')), \
               1, \
               length(rtrim(prefix, replace(prefix, '/', ''))) - 1 \
             ) \
           END \
           FROM rfs_ancestors WHERE prefix <> '/' \
         )";
    let mut key_values = Vec::new();
    let mut key_presence = Vec::new();
    for i in 0..crate::index::MAX_ORDERED_INDEX_KEYS {
        key_values.push(format!(
            "CASE WHEN json_array_length(s.keys) > {i} \
             THEN json_extract(new.indexed, '$.' || json_extract(s.keys, '$[{i}]')) END"
        ));
        key_presence.push(format!(
            "(json_array_length(s.keys) <= {i} \
             OR json_type(new.indexed, '$.' || json_extract(s.keys, '$[{i}]')) IS NOT NULL)"
        ));
    }
    let key_values = key_values.join(", ");
    let key_presence = key_presence.join(" AND ");
    let project = format!(
        "INSERT OR REPLACE INTO root_filesystem_ordered_index_rows(\
           index_name, path, k0, k1, k2, k3, k4, k5, k6, k7\
         ) \
         {ancestors} \
         SELECT s.name, new.path, {key_values} \
         FROM rfs_ancestors a \
         JOIN root_filesystem_index_specs s ON s.prefix = a.prefix \
         WHERE s.kind IN ('exact', 'prefix') \
           AND new.is_dir = 0 \
           AND new.indexed IS NOT NULL \
           AND {key_presence} \
         ORDER BY LENGTH(s.prefix) ASC;"
    );
    let statements = format!(
        "CREATE INDEX IF NOT EXISTS idx_root_filesystem_ordered_rows_path \
           ON root_filesystem_ordered_index_rows(path);\
         CREATE TRIGGER IF NOT EXISTS rfs_ordered_projection_v3_ai \
           AFTER INSERT ON root_filesystem_entries \
           BEGIN {project} END;\
         CREATE TRIGGER IF NOT EXISTS rfs_ordered_projection_v3_au \
           AFTER UPDATE ON root_filesystem_entries \
           BEGIN \
             DELETE FROM root_filesystem_ordered_index_rows WHERE path = old.path; \
             {project} \
           END;\
         CREATE TRIGGER IF NOT EXISTS rfs_ordered_projection_v3_ad \
           AFTER DELETE ON root_filesystem_entries \
           BEGIN \
             DELETE FROM root_filesystem_ordered_index_rows WHERE path = old.path; \
           END;"
    );
    conn.execute_batch(&statements)
        .await
        .map(|_| ())
        .map_err(|error| infrastructure_libsql_error(FilesystemOperation::EnsureIndex, error))
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
const LIBSQL_ORDERED_INDEX_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_ordered_index_rows (
    index_name TEXT NOT NULL,
    path TEXT NOT NULL,
    k0,
    k1,
    k2,
    k3,
    k4,
    k5,
    k6,
    k7,
    PRIMARY KEY (index_name, path)
);
CREATE INDEX IF NOT EXISTS idx_root_filesystem_ordered_values_v1
    ON root_filesystem_ordered_index_rows(
        index_name, k0, k1, k2, k3, k4, k5, k6, k7, path
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
    use crate::{CasExpectation, Entry, IndexName, RecordKind};
    use ironclaw_host_api::path::VirtualPath;

    struct DeleteIfVersionCancellationGate {
        runtime_id: usize,
        path: VirtualPath,
        begun: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    }

    static DELETE_IF_VERSION_CANCELLATION_GATE: std::sync::Mutex<
        Option<DeleteIfVersionCancellationGate>,
    > = std::sync::Mutex::new(None);

    struct AppendCancellationGate {
        runtime_id: usize,
        path: VirtualPath,
        inserted: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    }

    static APPEND_CANCELLATION_GATE: std::sync::Mutex<Option<AppendCancellationGate>> =
        std::sync::Mutex::new(None);

    fn install_delete_if_version_cancellation_gate(
        filesystem: &LibSqlRootFilesystem,
        path: &VirtualPath,
        begun: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) {
        *DELETE_IF_VERSION_CANCELLATION_GATE
            .lock()
            .expect("install delete cancellation gate") = Some(DeleteIfVersionCancellationGate {
            runtime_id: Arc::as_ptr(&filesystem.runtime) as usize,
            path: path.clone(),
            begun,
            release,
        });
    }

    pub(super) async fn pause_delete_if_version_after_transaction_begin(
        runtime_id: usize,
        path: &VirtualPath,
    ) {
        let gate = {
            let mut gate = DELETE_IF_VERSION_CANCELLATION_GATE
                .lock()
                .expect("delete cancellation gate");
            let matches_target = gate
                .as_ref()
                .is_some_and(|gate| gate.runtime_id == runtime_id && gate.path == *path);
            if matches_target { gate.take() } else { None }
        };
        if let Some(DeleteIfVersionCancellationGate { begun, release, .. }) = gate {
            let _ = begun.send(());
            let _ = release.await;
        }
    }

    fn install_append_cancellation_gate(
        filesystem: &LibSqlRootFilesystem,
        path: &VirtualPath,
        inserted: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) {
        *APPEND_CANCELLATION_GATE
            .lock()
            .expect("install append cancellation gate") = Some(AppendCancellationGate {
            runtime_id: Arc::as_ptr(&filesystem.runtime) as usize,
            path: path.clone(),
            inserted,
            release,
        });
    }

    pub(super) async fn pause_append_after_insert(runtime_id: usize, path: &VirtualPath) {
        let gate = {
            let mut gate = APPEND_CANCELLATION_GATE
                .lock()
                .expect("append cancellation gate");
            let matches_target = gate
                .as_ref()
                .is_some_and(|gate| gate.runtime_id == runtime_id && gate.path == *path);
            if matches_target { gate.take() } else { None }
        };
        if let Some(AppendCancellationGate {
            inserted, release, ..
        }) = gate
        {
            let _ = inserted.send(());
            let _ = release.await;
        }
    }

    async fn fresh_backend() -> (LibSqlRootFilesystem, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vector-test.db");
        let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem::new(db).expect("filesystem runtime");
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
        let conn = fs.read_connection().await.unwrap();
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

    #[tokio::test]
    async fn ordered_query_plan_uses_declared_composite_projection_index() {
        let (fs, _dir) = fresh_backend().await;
        let prefix = VirtualPath::new("/threads/index").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("thread_activity_v2").unwrap(),
            vec![
                IndexKey::new("scope_key").unwrap(),
                IndexKey::new("activity_sort").unwrap(),
                IndexKey::new("thread_id").unwrap(),
            ],
            IndexKind::Exact,
        );
        fs.ensure_index(&prefix, &spec).await.unwrap();
        let expected_index = "idx_root_filesystem_ordered_values_v1";
        let conn = fs.read_connection().await.unwrap();
        let mut rows = conn
            .query(
                "EXPLAIN QUERY PLAN \
                 SELECT entry.path \
                 FROM root_filesystem_ordered_index_rows AS ordered \
                 JOIN root_filesystem_entries AS entry ON entry.path = ordered.path \
                 WHERE ordered.index_name = ?1 \
                   AND ordered.k0 = ?2 \
                   AND (ordered.path = ?3 OR ordered.path LIKE ?4 ESCAPE '!') \
                 ORDER BY ordered.k1 ASC, ordered.k2 ASC \
                 LIMIT ?5",
                libsql::params![
                    spec.name.as_str(),
                    "scope-a",
                    prefix.as_str(),
                    "/threads/index/%",
                    201_i64
                ],
            )
            .await
            .unwrap();
        let mut details = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            details.push(row.get::<String>(3).unwrap());
        }
        assert!(
            details.iter().any(|detail| detail.contains(expected_index)),
            "ordered query must use {expected_index}; plan={details:?}"
        );
        assert!(
            details
                .iter()
                .all(|detail| !detail.contains("USE TEMP B-TREE FOR ORDER BY")),
            "ordered query must not materialize/sort the scope; plan={details:?}"
        );
    }

    #[tokio::test]
    async fn identical_projection_specs_share_one_physical_index_across_prefixes() {
        let (fs, _dir) = fresh_backend().await;
        let spec = IndexSpec::new(
            IndexName::new("thread_activity_v2").unwrap(),
            vec![
                IndexKey::new("scope_key").unwrap(),
                IndexKey::new("activity_sort").unwrap(),
                IndexKey::new("thread_id").unwrap(),
            ],
            IndexKind::Exact,
        );
        let alias_spec = IndexSpec::new(
            IndexName::new("recent_threads").unwrap(),
            spec.keys.clone(),
            IndexKind::Exact,
        );
        for (prefix, declared_spec) in [
            ("/threads/owner-a", &spec),
            ("/threads/owner-b", &alias_spec),
        ] {
            fs.ensure_index(&VirtualPath::new(prefix).unwrap(), declared_spec)
                .await
                .unwrap();
        }
        let conn = fs.read_connection().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                libsql::params!["idx_root_filesystem_ordered_values_v1"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<i64>(0).unwrap(), 1);
    }

    /// The record `query` path is the hot read for every domain store, and it
    /// must seek the path index like `list_dir` already does. `LIKE ... ESCAPE`
    /// cannot use the primary key, so the prefix predicate degrades to a full
    /// scan of `root_filesystem_entries` -- the cost then grows with total rows
    /// in the database (threads, turns, memory, events), independent of how
    /// much data the caller actually asked for. That is invisible on a local
    /// disk and dominates on network-attached storage: it took the hosted
    /// Extensions page to ~2s across ~40 such queries per load.
    #[tokio::test]
    async fn record_query_seeks_the_path_index_instead_of_scanning() {
        let (fs, _dir) = fresh_backend().await;
        let parent = VirtualPath::new("/memory/extensions/.installations/v2/memberships").unwrap();
        let (prefix_lower, prefix_upper) = descendant_path_range(&parent);
        let conn = fs.read_connection().await.unwrap();
        for sql in [RECORD_QUERY_PREFIX_SQL, INDEXED_QUERY_PREFIX_SQL] {
            let mut rows = conn
                .query(
                    &format!("EXPLAIN QUERY PLAN {sql}"),
                    libsql::params![parent.as_str(), prefix_lower.clone(), prefix_upper.clone()],
                )
                .await
                .unwrap();
            let mut details = Vec::new();
            while let Some(row) = rows.next().await.unwrap() {
                details.push(row.get::<String>(3).unwrap());
            }
            assert!(
                details
                    .iter()
                    .all(|detail| !detail.contains("SCAN root_filesystem_entries")),
                "record query must not scan every row, plan: {details:?}"
            );
            assert!(
                details
                    .iter()
                    .any(|detail| detail.contains("root_filesystem_entries USING")),
                "record query must use the path index, plan: {details:?}"
            );
        }
    }

    /// Differential proof that the range bounds select exactly what the
    /// `LIKE ... ESCAPE` predicate they replaced selected. The range form is
    /// only equivalent because '/' (0x2F) and '0' (0x30) are adjacent under
    /// the BINARY collation `path` uses, so ["{prefix}/", "{prefix}0") is
    /// precisely the descendant set. That argument is easy to state and easy
    /// to get wrong, so assert it against an adversarial corpus instead:
    /// sibling prefixes, LIKE metacharacters (`%`, `_`, and the `!` escape
    /// itself), the boundary code points either side of '/', and multi-byte
    /// paths. Both predicates must return identical rows for every prefix.
    #[tokio::test]
    async fn range_bounds_select_exactly_what_the_like_predicate_did() {
        let (fs, _dir) = fresh_backend().await;
        let conn = fs.migration_write_connection().await.unwrap();
        let corpus = [
            "/memory/a",
            "/memory/a/b",
            "/memory/a/b/c",
            "/memory/a/b/c/d",
            "/memory/a/bc", // sibling whose name extends the prefix
            "/memory/a/b-2",
            "/memory/a/b.hidden", // '.' is 0x2E, immediately below '/'
            "/memory/a/b0",       // '0' is 0x30, immediately above '/'
            "/memory/a/b0/child",
            "/memory/a/b1",
            "/memory/ab",
            "/memory/a%pct", // LIKE wildcard in a real path
            "/memory/a%pct/child",
            "/memory/a_us", // LIKE single-char wildcard
            "/memory/a_us/child",
            "/memory/a!bang", // the ESCAPE character itself
            "/memory/a!bang/child",
            "/memory/a/b/%",
            "/memory/a/b/_",
            "/memory/a/b/!",
            "/memory/a/ünïcode",
            "/memory/a/ünïcode/child",
            "/memory/a/b/\u{10FFFF}",
        ];
        for path in corpus {
            conn.execute(
                "INSERT INTO root_filesystem_entries(path, contents, is_dir, content_type, \
                 kind, indexed, version) VALUES (?1, X'', 0, 'application/json', NULL, '{}', 1)",
                libsql::params![path],
            )
            .await
            .unwrap();
        }

        async fn rows(conn: &libsql::Connection, sql: &str, params: Vec<String>) -> Vec<String> {
            let params: Vec<libsql::Value> = params.into_iter().map(libsql::Value::Text).collect();
            let mut out = Vec::new();
            let mut rows = conn.query(sql, params).await.unwrap();
            while let Some(row) = rows.next().await.unwrap() {
                out.push(row.get::<String>(0).unwrap());
            }
            out.sort();
            out
        }

        const LIKE_SQL: &str = "SELECT path FROM root_filesystem_entries \
             WHERE is_dir = 0 AND (path = ?1 OR path LIKE ?2 ESCAPE '!')";
        const RANGE_SQL: &str = "SELECT path FROM root_filesystem_entries \
             WHERE is_dir = 0 AND (path = ?1 OR (path >= ?2 AND path < ?3))";

        for prefix in corpus {
            let vpath = VirtualPath::new(prefix).unwrap();
            let (lower, upper) = descendant_path_range(&vpath);
            let legacy_pattern =
                crate::db::escape_like_with_trailing_wildcard(&format!("{prefix}/%"));

            let via_like = rows(&conn, LIKE_SQL, vec![prefix.to_string(), legacy_pattern]).await;
            let via_range = rows(&conn, RANGE_SQL, vec![prefix.to_string(), lower, upper]).await;

            assert_eq!(
                via_like, via_range,
                "range bounds must select exactly the LIKE match set for prefix {prefix:?}"
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
        let fs = Arc::new(LibSqlRootFilesystem::new(db).expect("filesystem runtime"));
        fs.run_migrations().await.unwrap();

        let mut handles = Vec::new();
        for _ in 0..10 {
            let fs = Arc::clone(&fs);
            handles.push(tokio::spawn(async move {
                let conn = fs.read_connection().await?;
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
        let fs = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)).expect("filesystem runtime"));
        fs.run_migrations().await.unwrap();

        // Configure the runtime's writer connection to fail quickly against a
        // lock owned outside the shared process-local admission lane.
        let contender = fs.migration_write_connection().await.unwrap();
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

        let writer = db.connect().unwrap();
        writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();

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

    /// The projection trigger set is static: declaring many specs at many
    /// prefixes must leave exactly three triggers on the entries table, and a
    /// surviving legacy per-declaration trigger must be swept on the next
    /// declaration. The per-declaration design accumulated 3 triggers per
    /// (prefix, spec) — O(total declarations) evaluated on every write.
    #[tokio::test]
    async fn ordered_projection_triggers_stay_constant_across_declarations() {
        let (fs, _dir) = fresh_backend().await;
        let writer = fs.migration_write_connection().await.unwrap();
        // A survivor from the per-declaration era, shaped like sql_index_name
        // output and touching the projection table so the sweep matches it.
        writer
            .execute_batch(
                "CREATE TRIGGER idx_rfs_legacy_by_status_ai \
                 AFTER INSERT ON root_filesystem_entries BEGIN \
                   DELETE FROM root_filesystem_ordered_index_rows WHERE path = new.path; \
                 END;",
            )
            .await
            .unwrap();
        drop(writer);

        for i in 0..5 {
            let path = VirtualPath::new(format!("/resources/trigger-count/{i}")).unwrap();
            let spec = IndexSpec::new(
                IndexName::new(format!("by_status_{i}")).unwrap(),
                vec![IndexKey::new("status").unwrap()],
                IndexKind::Exact,
            );
            fs.ensure_index(&path, &spec).await.unwrap();
        }

        let reader = fs.read_connection().await.unwrap();
        let mut rows = reader
            .query(
                "SELECT name FROM sqlite_master WHERE type = 'trigger' \
                 AND sql LIKE '%root_filesystem_ordered_index_rows%' ORDER BY name",
                (),
            )
            .await
            .unwrap();
        let mut names = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            names.push(row.get::<String>(0).unwrap());
        }
        assert_eq!(
            names,
            vec![
                "rfs_ordered_projection_v3_ad".to_string(),
                "rfs_ordered_projection_v3_ai".to_string(),
                "rfs_ordered_projection_v3_au".to_string(),
            ],
            "five declarations leave exactly the static trigger set and the \
             legacy per-declaration trigger is swept"
        );
    }

    #[tokio::test]
    async fn ensure_index_rolls_back_the_catalog_when_ddl_fails() {
        let (fs, _dir) = fresh_backend().await;
        let path = VirtualPath::new("/resources/index-atomicity").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("by_status").unwrap(),
            vec![IndexKey::new("status").unwrap()],
            IndexKind::Exact,
        );
        let writer = fs.migration_write_connection().await.unwrap();
        writer
            .execute("DROP TABLE root_filesystem_entries", ())
            .await
            .unwrap();
        drop(writer);

        let error = fs
            .ensure_index(&path, &spec)
            .await
            .expect_err("the conflicting table must make index DDL fail");
        assert!(matches!(error, FilesystemError::Backend { .. }));

        let reader = fs.read_connection().await.unwrap();
        let mut rows = reader
            .query(
                "SELECT COUNT(*) FROM root_filesystem_index_specs \
                 WHERE prefix = ?1 AND name = ?2",
                libsql::params![path.as_str(), spec.name.as_str()],
            )
            .await
            .unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(
            count, 0,
            "failed index DDL must roll back the preceding catalog upsert"
        );
    }

    #[tokio::test]
    async fn cancelling_append_after_insert_rolls_back_the_event() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("append-cancellation-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = Arc::new(LibSqlRootFilesystem::new(db).expect("filesystem runtime"));
        fs.run_migrations().await.unwrap();

        let path = VirtualPath::new("/resources/cancelled-append").unwrap();
        let (inserted_tx, inserted_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel();
        install_append_cancellation_gate(&fs, &path, inserted_tx, release_rx);

        let append_fs = Arc::clone(&fs);
        let append_path = path.clone();
        let append =
            tokio::spawn(
                async move { append_fs.append(&append_path, b"cancelled".to_vec()).await },
            );
        inserted_rx
            .await
            .expect("append reaches the point after its insert");
        append.abort();
        assert!(
            append
                .await
                .expect_err("cancelled append task")
                .is_cancelled(),
            "append task must be cancelled while its transaction is open"
        );

        assert!(
            fs.tail(&path, SeqNo::ZERO).await.unwrap().is_empty(),
            "cancelling append after INSERT must roll the event back"
        );
        let seq = fs.append(&path, b"accepted".to_vec()).await.unwrap();
        assert_eq!(seq, SeqNo::from_backend(1));
    }

    #[test]
    fn writer_checkout_timeout_maps_to_retryable_backend_busy() {
        let path = VirtualPath::new("/resources/deltas/log").unwrap();
        let error = map_runtime_write_connection_error(
            path.clone(),
            FilesystemOperation::Append,
            ironclaw_libsql_runtime::LibSqlRuntimeError::Checkout {
                lane: ironclaw_libsql_runtime::LibSqlLane::Write,
                reason: ironclaw_libsql_runtime::LibSqlCheckoutFailureReason::Timeout,
            },
        );

        assert!(matches!(
            error,
            FilesystemError::BackendBusy {
                path: error_path,
                operation: FilesystemOperation::Append,
            } if error_path == path
        ));
    }

    #[test]
    #[tracing_test::traced_test]
    fn runtime_connection_error_logs_source_but_returns_redacted_reason() {
        const SOURCE_MARKER: &str = "connection-source-marker";
        let error = map_runtime_connection_error(LibSqlRuntimeError::Connection {
            operation: "open test database",
            source: libsql::Error::SqliteFailure(14, SOURCE_MARKER.to_string()),
        });

        let FilesystemError::BackendInfrastructure { reason, .. } = error else {
            panic!("connection failures must map to backend infrastructure errors");
        };
        assert!(
            !reason.contains(SOURCE_MARKER),
            "public filesystem errors must keep the libSQL source redacted"
        );
        assert!(
            logs_contain(SOURCE_MARKER),
            "debug diagnostics must retain the underlying libSQL source"
        );
    }

    #[test]
    #[tracing_test::traced_test]
    fn runtime_writer_error_logs_source_but_returns_redacted_reason() {
        const SOURCE_MARKER: &str = "writer-source-marker";
        let error = map_runtime_write_connection_error(
            VirtualPath::new("/resources/deltas/log").unwrap(),
            FilesystemOperation::Append,
            LibSqlRuntimeError::Connection {
                operation: "checkout writer",
                source: libsql::Error::SqliteFailure(14, SOURCE_MARKER.to_string()),
            },
        );

        let FilesystemError::BackendInfrastructure { reason, .. } = error else {
            panic!("writer connection failures must map to backend infrastructure errors");
        };
        assert!(
            !reason.contains(SOURCE_MARKER),
            "public filesystem errors must keep the libSQL source redacted"
        );
        assert!(
            logs_contain(SOURCE_MARKER),
            "debug diagnostics must retain the underlying libSQL source"
        );
    }

    /// Break caught: deleting the entry row before a later event-log cleanup
    /// fails would expose a partially applied filesystem delete.
    #[tokio::test]
    async fn delete_rolls_back_all_tables_when_event_cleanup_fails() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("atomic-delete-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem::new(Arc::clone(&db)).expect("filesystem runtime");
        fs.run_migrations().await.unwrap();

        let path = VirtualPath::new("/resources/atomic/delete").unwrap();
        fs.write_file(&path, b"entry").await.unwrap();
        fs.append(&path, b"event".to_vec()).await.unwrap();

        let connection = db.connect().unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_event_delete \
                 BEFORE DELETE ON root_filesystem_events \
                 WHEN OLD.path = '/resources/atomic/delete' \
                 BEGIN \
                   SELECT RAISE(ABORT, 'synthetic event cleanup failure'); \
                 END;",
            )
            .await
            .unwrap();

        let result = fs.delete(&path).await;
        assert!(matches!(result, Err(FilesystemError::Backend { .. })));
        assert!(
            fs.get(&path).await.unwrap().is_some(),
            "a failed multi-table delete must restore the entry row"
        );
        assert_eq!(
            fs.tail(&path, SeqNo::ZERO).await.unwrap().len(),
            1,
            "a failed multi-table delete must preserve append events"
        );
    }

    /// Break caught: checking for descendants before entering the writer lane
    /// lets a child appear between validation and the parent-file write.
    #[tokio::test]
    async fn write_file_rechecks_directory_conflict_inside_writer_lane() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("writer-lane-precondition-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let runtime = Arc::new(LibSqlRuntime::new(Arc::clone(&db)).expect("libSQL runtime"));
        let fs = Arc::new(LibSqlRootFilesystem::from_runtime(Arc::clone(&runtime)));
        fs.run_migrations().await.unwrap();

        let held_writer = runtime.write().await.unwrap();
        let parent = VirtualPath::new("/resources/parent").unwrap();
        let write_fs = Arc::clone(&fs);
        let write_parent = parent.clone();
        let mut parent_write =
            tokio::spawn(async move { write_fs.write_file(&write_parent, b"parent").await });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let external = db.connect().unwrap();
        external
            .execute(
                "INSERT INTO root_filesystem_entries \
                 (path, contents, is_dir, content_type, kind, indexed, version) \
                 VALUES (?1, X'', 0, 'application/octet-stream', NULL, '{}', 1)",
                libsql::params!["/resources/parent/child"],
            )
            .await
            .unwrap();
        drop(held_writer);

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), &mut parent_write)
            .await
            .expect("parent write completes")
            .expect("parent write task");
        assert!(
            matches!(result, Err(FilesystemError::Backend { .. })),
            "the parent file must be rejected after a child appears: {result:?}"
        );
        assert!(
            fs.get(&parent).await.unwrap().is_none(),
            "the rejected parent file must not coexist with its child"
        );
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
        let conn = fs.read_connection().await.unwrap();
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
        let conn = fs.read_connection().await.unwrap();

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

    /// Deterministic, single-task regression pin for the atomicity fix
    /// (commit 1792aebb2 / PR #5749 round 4): `delete_if_version`'s
    /// zero-rows diagnosis must reuse the SAME connection the conditional
    /// DELETE ran on, not check out a second one. Round-B review finding:
    /// the concurrency storm test in `tests/concurrent_cas_storm.rs`
    /// doesn't actually discriminate this — every racer shares one
    /// pre-fetched version and nothing recreates the path mid-round, so
    /// it passes with or without the fix. This test does discriminate it,
    /// with no concurrency required: the shared runtime's writer lane has
    /// exactly one connection, so `delete_if_version` checks it out and hits
    /// the stale-version (0-rows) branch. If diagnosis tried to acquire a
    /// second writer lease, it would deadlock against the first; reusing the
    /// passed-in connection completes immediately.
    #[tokio::test]
    async fn delete_if_version_diagnosis_reuses_the_delete_connection_under_a_size_one_pool() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("delete-single-conn-test.db");
        let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let fs = LibSqlRootFilesystem::new(db).expect("filesystem runtime");
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

    #[tokio::test]
    async fn cancelling_pooled_delete_transaction_releases_external_writer_lock() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("delete-cancellation-test.db");
        let db = Arc::new(
            libsql::Builder::new_local(db_path.clone())
                .build()
                .await
                .unwrap(),
        );
        let fs = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)).expect("filesystem runtime"));
        fs.run_migrations().await.unwrap();

        let path = VirtualPath::new("/resources/cancelled-delete").unwrap();
        let version = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let unrelated_path = VirtualPath::new("/resources/unrelated-delete").unwrap();
        let unrelated_version = fs
            .put(
                &unrelated_path,
                Entry::bytes(vec![2]),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
        let (other_fs, _other_dir) = fresh_backend().await;
        let other_version = other_fs
            .put(&path, Entry::bytes(vec![3]), CasExpectation::Absent)
            .await
            .unwrap();

        let (begun_tx, begun_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel();
        install_delete_if_version_cancellation_gate(&fs, &path, begun_tx, release_rx);

        fs.delete_if_version(&unrelated_path, unrelated_version)
            .await
            .expect("a different path must not consume the target cancellation gate");
        other_fs
            .delete_if_version(&path, other_version)
            .await
            .expect("the same path on a different runtime must not consume the target gate");

        let delete_fs = Arc::clone(&fs);
        let delete_path = path.clone();
        let delete =
            tokio::spawn(async move { delete_fs.delete_if_version(&delete_path, version).await });
        begun_rx
            .await
            .expect("delete reaches the point after BEGIN IMMEDIATE");
        delete.abort();
        assert!(
            delete
                .await
                .expect_err("cancelled delete task")
                .is_cancelled(),
            "delete task must be cancelled while its transaction is open"
        );

        let independent_db = libsql::Builder::new_local(db_path).build().await.unwrap();
        let independent = independent_db.connect().unwrap();
        let mut configured = independent
            .query("PRAGMA busy_timeout = 25", ())
            .await
            .unwrap();
        while configured.next().await.unwrap().is_some() {}
        drop(configured);

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            independent.execute(
                "INSERT INTO root_filesystem_entries \
                 (path, contents, is_dir, content_type, kind, indexed, version) \
                 VALUES (?1, X'', 0, 'application/octet-stream', NULL, '{}', 1)",
                libsql::params!["/resources/independent-writer"],
            ),
        )
        .await
        .expect("independent writer must not hang behind a cancelled pooled transaction")
        .expect("independent writer must acquire the SQLite writer lock");
    }

    /// Break caught: dropping the filesystem transaction must release the
    /// size-one writer lane before the same task performs its next write.
    #[tokio::test]
    async fn dropping_storage_transaction_releases_writer_lane_synchronously() {
        let (fs, _dir) = fresh_backend().await;
        let fs = Arc::new(fs);
        let prefix = VirtualPath::new("/resources").unwrap();
        let abandoned_path = VirtualPath::new("/resources/abandoned").unwrap();
        let next_path = VirtualPath::new("/resources/next").unwrap();
        let task_fs = Arc::clone(&fs);
        let task_prefix = prefix.clone();
        let task_abandoned_path = abandoned_path.clone();
        let task_next_path = next_path.clone();
        tokio::spawn(async move {
            let mut transaction = task_fs.begin(&task_prefix).await.unwrap();
            transaction
                .put(
                    &task_abandoned_path,
                    Entry::bytes(b"uncommitted".to_vec()),
                    CasExpectation::Absent,
                )
                .await
                .unwrap();
            drop(transaction);

            let mut writer_checkout = Box::pin(task_fs.runtime.write());
            let first_poll = std::future::poll_fn(|context| {
                std::task::Poll::Ready(std::future::Future::poll(writer_checkout.as_mut(), context))
            })
            .await;
            match first_poll {
                std::task::Poll::Ready(Ok(writer)) => drop(writer),
                std::task::Poll::Ready(Err(error)) => {
                    panic!("dropping the transaction must clear writer ownership: {error}")
                }
                std::task::Poll::Pending => {
                    drop(
                        writer_checkout
                            .await
                            .expect("a recycled writer connection must become available"),
                    );
                }
            }

            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                task_fs.put(
                    &task_next_path,
                    Entry::bytes(b"committed".to_vec()),
                    CasExpectation::Absent,
                ),
            )
            .await
            .expect("the next write must not wait behind a detached rollback")
            .expect("the same task must reacquire the writer lane after dropping a transaction");
        })
        .await
        .expect("writer task");

        assert!(
            fs.get(&abandoned_path).await.unwrap().is_none(),
            "dropping an active transaction must roll back its staged write"
        );
        assert!(
            fs.get(&next_path).await.unwrap().is_some(),
            "the writer lane must remain usable after cancellation cleanup"
        );
    }
}
