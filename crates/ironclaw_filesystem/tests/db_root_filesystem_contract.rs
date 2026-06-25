#![cfg(any(feature = "libsql", feature = "postgres"))]

use ironclaw_filesystem::RootFilesystem;

#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::{
    Capability, CasExpectation, Entry, FileType, FilesystemError, FilesystemOperation, Filter,
    IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, LibSqlRootFilesystem, Page, RecordKind,
    SeqNo,
};
#[cfg(feature = "libsql")]
use ironclaw_host_api::VirtualPath;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_reads_writes_and_stats_files() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/engine/tenants/t1/users/u1/file.txt").unwrap();

    filesystem.write_file(&path, b"hello db fs").await.unwrap();

    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"hello db fs");
    let stat = filesystem.stat(&path).await.unwrap();
    assert_eq!(stat.path, path);
    assert_eq!(stat.file_type, FileType::File);
    assert_eq!(stat.len, 11);
    assert!(stat.modified.is_some());
    assert!(!stat.sensitive);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_lists_direct_children_sorted_with_virtual_paths() {
    let filesystem = libsql_root().await;
    filesystem
        .write_file(
            &VirtualPath::new("/engine/tenants/t1/users/u1/zeta.txt").unwrap(),
            b"z",
        )
        .await
        .unwrap();
    filesystem
        .write_file(
            &VirtualPath::new("/engine/tenants/t1/users/u1/alpha.txt").unwrap(),
            b"a",
        )
        .await
        .unwrap();
    filesystem
        .write_file(
            &VirtualPath::new("/engine/tenants/t1/users/u1/nested/file.txt").unwrap(),
            b"nested",
        )
        .await
        .unwrap();

    let entries = filesystem
        .list_dir(&VirtualPath::new("/engine/tenants/t1/users/u1").unwrap())
        .await
        .unwrap();

    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["alpha.txt", "nested", "zeta.txt"]);

    let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/engine/tenants/t1/users/u1/alpha.txt",
            "/engine/tenants/t1/users/u1/nested",
            "/engine/tenants/t1/users/u1/zeta.txt",
        ]
    );
    assert_eq!(entries[1].file_type, FileType::Directory);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_appends_deletes_and_creates_directories() {
    let filesystem = libsql_root().await;
    let dir = VirtualPath::new("/engine/tenants/t1/users/u1/logs").unwrap();
    let path = VirtualPath::new("/engine/tenants/t1/users/u1/logs/events.jsonl").unwrap();

    filesystem.create_dir_all(&dir).await.unwrap();
    assert_eq!(
        filesystem.stat(&dir).await.unwrap().file_type,
        FileType::Directory
    );
    assert!(filesystem.list_dir(&dir).await.unwrap().is_empty());

    filesystem.append_file(&path, b"one\n").await.unwrap();
    filesystem.append_file(&path, b"two\n").await.unwrap();
    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"one\ntwo\n");

    filesystem.delete(&path).await.unwrap();
    let err = filesystem.read_file(&path).await.unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::NotFound {
            operation: FilesystemOperation::ReadFile,
            ..
        }
    ));

    let err = filesystem.delete(&path).await.unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::NotFound {
            operation: FilesystemOperation::Delete,
            ..
        }
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_overwrites_existing_file() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/memory/tenants/t1/users/u1/facts.md").unwrap();

    filesystem.write_file(&path, b"first").await.unwrap();
    filesystem.write_file(&path, b"second").await.unwrap();

    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"second");
    assert_eq!(filesystem.stat(&path).await.unwrap().len, 6);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_write_file_rejects_existing_directory() {
    let filesystem = libsql_root().await;
    let dir = VirtualPath::new("/engine/tenants/t1/users/u1/logs").unwrap();
    let child = VirtualPath::new("/engine/tenants/t1/users/u1/logs/events.jsonl").unwrap();

    filesystem.create_dir_all(&dir).await.unwrap();
    filesystem.write_file(&child, b"one\n").await.unwrap();
    let err = filesystem.write_file(&dir, b"not a dir").await.unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::Backend {
            operation: FilesystemOperation::WriteFile,
            ..
        }
    ));
    assert_eq!(
        filesystem.stat(&dir).await.unwrap().file_type,
        FileType::Directory
    );
    assert_eq!(filesystem.read_file(&child).await.unwrap(), b"one\n");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_write_file_rejects_implicit_directory() {
    let filesystem = libsql_root().await;
    let dir = VirtualPath::new("/engine/tenants/t1/users/u1/nested").unwrap();
    let child = VirtualPath::new("/engine/tenants/t1/users/u1/nested/file.txt").unwrap();

    filesystem.write_file(&child, b"child").await.unwrap();
    let err = filesystem.write_file(&dir, b"not a dir").await.unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::Backend {
            operation: FilesystemOperation::WriteFile,
            ..
        }
    ));
    assert_eq!(
        filesystem.stat(&dir).await.unwrap().file_type,
        FileType::Directory
    );
    assert_eq!(filesystem.read_file(&child).await.unwrap(), b"child");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_append_file_rejects_implicit_directory() {
    let filesystem = libsql_root().await;
    let dir = VirtualPath::new("/engine/tenants/t1/users/u1/append-nested").unwrap();
    let child = VirtualPath::new("/engine/tenants/t1/users/u1/append-nested/file.txt").unwrap();

    filesystem.write_file(&child, b"child").await.unwrap();
    let err = filesystem
        .append_file(&dir, b"not a dir")
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::Backend {
            operation: FilesystemOperation::AppendFile,
            ..
        }
    ));
    assert_eq!(
        filesystem.stat(&dir).await.unwrap().file_type,
        FileType::Directory
    );
    assert_eq!(filesystem.read_file(&child).await.unwrap(), b"child");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_fails_closed_for_missing_paths_without_host_paths() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/projects/missing.txt").unwrap();

    let err = filesystem.read_file(&path).await.unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::NotFound {
            operation: FilesystemOperation::ReadFile,
            ..
        }
    ));
    let display = err.to_string();
    assert!(display.contains("/projects/missing.txt"));
    assert!(!display.contains("/tmp"));
    assert!(!display.contains(".db"));
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_root_filesystem_implements_root_filesystem_contract() {
    fn assert_root<T: RootFilesystem>() {}
    assert_root::<PostgresRootFilesystem>();
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_root_filesystem_migration_failure_surfaces_infrastructure_variant() {
    // Audit finding F1: backend connect/migration paths used to wrap
    // every infrastructure error in `FilesystemError::Backend` with a
    // fabricated `/engine` path. The path was always a lie — there is
    // no caller-supplied path in scope at migration time. Verify the
    // new `BackendInfrastructure` variant is what surfaces when the
    // backend's bootstrap path fails.
    //
    // Trigger a real migration failure by pre-populating the DB with a
    // table whose schema collides with what the migration expects to
    // add (`is_dir` column with an incompatible non-default-able CHECK
    // constraint that conflicts with the `ALTER` the migration runs).
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let raw_db = std::sync::Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let conn = raw_db.connect().unwrap();
    // Pre-create a table that prevents `CREATE TABLE root_filesystem_entries`
    // from being clean: the migration's CREATE IF NOT EXISTS is fine, but
    // the subsequent `ALTER TABLE ... ADD COLUMN is_dir INTEGER NOT NULL`
    // requires a default. Pre-existing rows without that column will
    // satisfy the default; but inserting an incompatible row first makes
    // the column add fail.
    conn.execute(
        "CREATE TABLE root_filesystem_entries (path TEXT PRIMARY KEY, contents BLOB NOT NULL DEFAULT X'')",
        (),
    )
    .await
    .unwrap();
    // Lock the file by removing write permissions so the migration's
    // ALTER paths fail outright. On platforms where chmod is honoured
    // (unix), this surfaces a libsql write error from the migration.
    drop(conn);
    drop(raw_db);
    let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o444);
        std::fs::set_permissions(&db_path, perms).unwrap();
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(true);
        std::fs::set_permissions(&db_path, perms).unwrap();
    }

    let locked_db =
        std::sync::Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let filesystem = LibSqlRootFilesystem::new(locked_db);
    let err = filesystem.run_migrations().await.unwrap_err();
    assert!(
        matches!(err, FilesystemError::BackendInfrastructure { .. }),
        "expected BackendInfrastructure, got {err:?}"
    );
    // Display must NOT mention the fictional `/engine` placeholder
    // (previous behavior leaked it everywhere).
    let display = err.to_string();
    assert!(
        !display.contains("/engine"),
        "infrastructure error must not fabricate a virtual path: {display}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_page_offset_overflow_surfaces_typed_error() {
    // Audit finding F6: `page.offset as i64` previously truncate-wrapped
    // values ≥ 2^63 into a negative SQLite OFFSET, which produced a
    // cryptic backend error or (worse) silently returned an empty page.
    // Surface a typed `Backend` error naming the operation and value.
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/engine/tenants/t1/users/u1/file.txt").unwrap();
    filesystem.write_file(&path, b"hello").await.unwrap();

    let err = filesystem
        .query(
            &VirtualPath::new("/engine/tenants/t1/users/u1").unwrap(),
            &Filter::All,
            Page {
                offset: u64::MAX,
                limit: 1,
            },
        )
        .await
        .unwrap_err();
    match &err {
        FilesystemError::Backend {
            operation, reason, ..
        } => {
            assert_eq!(*operation, FilesystemOperation::Query);
            assert!(
                reason.contains("page offset"),
                "expected reason to name the overflow, got {reason}"
            );
        }
        other => panic!("expected Backend error, got {other:?}"),
    }
}

#[cfg(feature = "libsql")]
struct TestLibSqlRootFilesystem {
    filesystem: LibSqlRootFilesystem,
    _dir: tempfile::TempDir,
}

#[cfg(feature = "libsql")]
impl std::ops::Deref for TestLibSqlRootFilesystem {
    type Target = LibSqlRootFilesystem;

    fn deref(&self) -> &Self::Target {
        &self.filesystem
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_native_put_get_round_trip_with_record_metadata() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/L1").unwrap();

    let kind = RecordKind::new("credential_lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    let status_key = IndexKey::new("status").unwrap();
    let entry = Entry::record(kind.clone(), &serde_json::json!({"hidden": true}))
        .unwrap()
        .with_indexed(scope_key.clone(), IndexValue::Text("acme".into()))
        .with_indexed(status_key.clone(), IndexValue::Text("active".into()));

    let version1 = filesystem
        .put(&path, entry, CasExpectation::Absent)
        .await
        .unwrap();
    assert_eq!(version1.get(), 1);

    let got = filesystem
        .get(&path)
        .await
        .unwrap()
        .expect("entry should be present");
    assert_eq!(got.version, version1);
    assert_eq!(got.entry.kind.as_ref(), Some(&kind));
    assert_eq!(got.entry.indexed.len(), 2);
    assert!(got.entry.indexed.contains_key(&scope_key));
    assert!(got.entry.indexed.contains_key(&status_key));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_native_put_cas_absent_rejects_existing_path() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/L2").unwrap();
    filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();
    let err = filesystem
        .put(&path, Entry::bytes(vec![2]), CasExpectation::Absent)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_native_put_cas_version_advances_and_rejects_stale() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/L3").unwrap();
    let v1 = filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();
    let v2 = filesystem
        .put(&path, Entry::bytes(vec![2]), CasExpectation::Version(v1))
        .await
        .unwrap();
    assert!(v2 > v1);
    // Stale version rejected.
    let err = filesystem
        .put(&path, Entry::bytes(vec![3]), CasExpectation::Version(v1))
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_native_put_cas_any_increments_existing_version() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/L4").unwrap();
    let v1 = filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();
    let v2 = filesystem
        .put(&path, Entry::bytes(vec![2]), CasExpectation::Any)
        .await
        .unwrap();
    assert_eq!(v2.get(), v1.get() + 1);
    let got = filesystem.get(&path).await.unwrap().unwrap();
    assert_eq!(got.version, v2);
    assert_eq!(got.entry.body, vec![2]);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_get_returns_none_for_missing_path() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/missing").unwrap();
    assert!(filesystem.get(&path).await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_write_file_after_put_resets_record_metadata_and_bumps_version() {
    // PR #3660 reviewer fix: legacy write_file/append_file used to update
    // only `contents`/`is_dir`/`updated_at`, leaving stale `kind`,
    // `indexed`, `content_type`, and `version` from a prior put. A
    // subsequent get() would then return a versioned-entry whose
    // metadata didn't match the bytes. The fix clears schema metadata
    // and bumps the version on every legacy write.
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/STALE").unwrap();
    let kind = RecordKind::new("credential_lease").unwrap();
    let scope = IndexKey::new("scope").unwrap();
    let record_entry = Entry::record(kind, &serde_json::json!({"k": 1}))
        .unwrap()
        .with_indexed(scope, IndexValue::Text("acme".into()));

    let v1 = filesystem
        .put(&path, record_entry, CasExpectation::Absent)
        .await
        .unwrap();

    // Legacy write overwrites the entry with opaque bytes.
    filesystem.write_file(&path, b"opaque").await.unwrap();

    let got = filesystem.get(&path).await.unwrap().unwrap();
    // Metadata cleared: kind=None, indexed empty. Version bumped from v1.
    assert!(got.entry.kind.is_none());
    assert!(got.entry.indexed.is_empty());
    assert_eq!(got.entry.body, b"opaque");
    assert!(got.version > v1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_ensure_index_is_idempotent_and_conflict_aware() {
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/secrets/leases").unwrap();
    let name = IndexName::new("by_scope_status").unwrap();
    let keys = vec![
        IndexKey::new("scope").unwrap(),
        IndexKey::new("status").unwrap(),
    ];
    let spec_exact = IndexSpec::new(name.clone(), keys.clone(), IndexKind::Exact);
    let spec_prefix = IndexSpec::new(name, keys, IndexKind::Prefix);

    filesystem.ensure_index(&prefix, &spec_exact).await.unwrap();
    // Re-declaring same spec is idempotent.
    filesystem.ensure_index(&prefix, &spec_exact).await.unwrap();
    // Declaring a different kind under the same name is a conflict.
    let err = filesystem
        .ensure_index(&prefix, &spec_prefix)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::IndexConflict { .. }));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_ensure_index_accepts_fts_kind_and_filter_matches_text() {
    // FTS5 vtable + sync triggers are created at declaration time, and
    // existing rows are backfilled. After the index is declared, a
    // Filter::Fts query against the same key finds matching documents.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let content = IndexKey::new("content").unwrap();
    // Insert before declaring the index so backfill kicks in.
    for (path, body) in [
        ("/memory/a", "the quick brown fox jumps"),
        ("/memory/b", "the lazy dog sleeps"),
        ("/memory/c", "a brown bear naps in the woods"),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(content.clone(), IndexValue::Text(body.into()));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    let spec = IndexSpec::new(
        IndexName::new("by_content").unwrap(),
        vec![content.clone()],
        IndexKind::Fts,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    // Redeclaration is idempotent.
    filesystem.ensure_index(&prefix, &spec).await.unwrap();

    let results = filesystem
        .query(
            &prefix,
            &Filter::Fts {
                key: content,
                query: "brown".into(),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_fts_filter_picks_up_inserts_through_triggers() {
    // After ensure_index, inserting a new row through put() updates the
    // FTS5 shadow table via the AFTER INSERT trigger.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory/triggered").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let content = IndexKey::new("content").unwrap();

    let spec = IndexSpec::new(
        IndexName::new("by_content_trig").unwrap(),
        vec![content.clone()],
        IndexKind::Fts,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();

    let entry = Entry::record(kind, &serde_json::json!({}))
        .unwrap()
        .with_indexed(content.clone(), IndexValue::Text("emerald city".into()));
    filesystem
        .put(
            &VirtualPath::new("/memory/triggered/x").unwrap(),
            entry,
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    let results = filesystem
        .query(
            &prefix,
            &Filter::Fts {
                key: content,
                query: "emerald".into(),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_ensure_index_fts_rejects_path_with_sql_metacharacters() {
    // Regression: the FTS5 sync triggers splice the mount-prefix path
    // directly into DDL string literals (no parameter binding available in
    // SQLite trigger bodies). VirtualPath rejects NUL/control/backslash/`..`
    // but currently allows `'`, `"`, `;`, etc. We refuse to emit DDL when
    // the path contains anything outside [A-Za-z0-9_/.-] so a path crafted
    // to escape the literal cannot reach a CREATE TRIGGER statement.
    let filesystem = libsql_root().await;
    let injection_path = VirtualPath::new("/memory/'; DROP TABLE root_filesystem_entries; --")
        .expect("VirtualPath::new accepts single-quote; DDL emitter must reject it");
    let content = IndexKey::new("content").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("by_content_inject").unwrap(),
        vec![content],
        IndexKind::Fts,
    );
    let err = filesystem
        .ensure_index(&injection_path, &spec)
        .await
        .unwrap_err();
    match err {
        FilesystemError::Backend {
            operation, reason, ..
        } => {
            assert_eq!(operation, FilesystemOperation::EnsureIndex);
            assert!(
                reason.contains("[A-Za-z0-9_/.-]"),
                "expected identifier-safe rejection, got: {reason}"
            );
        }
        other => panic!("expected Backend error, got: {other:?}"),
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_vector_index_round_trips_and_ranks_by_cosine() {
    // IndexKind::Vector is accepted at declaration; storage shape is
    // IndexValue::Bytes (LE-encoded f32s) in the indexed projection;
    // VectorNearest ranks the candidate set by cosine and returns top-k.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory/vec").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let embedding_key = IndexKey::new("embedding").unwrap();

    let spec = IndexSpec::new(
        IndexName::new("by_vec").unwrap(),
        vec![embedding_key.clone()],
        IndexKind::Vector { dim: 3 },
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    // Re-declaration is idempotent.
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    // A conflicting dim is rejected.
    let conflict = IndexSpec::new(
        IndexName::new("by_vec").unwrap(),
        vec![embedding_key.clone()],
        IndexKind::Vector { dim: 4 },
    );
    let err = filesystem
        .ensure_index(&prefix, &conflict)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::IndexConflict { .. }));

    let blob = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() };
    for (path, vec) in [
        ("/memory/vec/A", vec![1.0_f32, 0.0, 0.0]),
        ("/memory/vec/B", vec![0.9, 0.1, 0.0]),
        ("/memory/vec/C", vec![0.0, 0.0, 1.0]),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(embedding_key.clone(), IndexValue::Bytes(blob(&vec)));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    let results = filesystem
        .query(
            &prefix,
            &Filter::VectorNearest {
                key: embedding_key.clone(),
                embedding: vec![1.0, 0.0, 0.0],
                limit: 2,
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    // /memory/vec/A is closest (identical vector).
    assert_eq!(
        results[0].entry.indexed.get(&embedding_key),
        Some(&IndexValue::Bytes(blob(&[1.0, 0.0, 0.0])))
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_filters_on_indexed_projection() {
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    let status_key = IndexKey::new("status").unwrap();
    let prefix = VirtualPath::new("/secrets/leases").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("by_scope_status").unwrap(),
        vec![scope_key.clone(), status_key.clone()],
        IndexKind::Exact,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();

    for (path, scope, status) in [
        ("/secrets/leases/A", "acme", "active"),
        ("/secrets/leases/B", "acme", "revoked"),
        ("/secrets/leases/C", "globex", "active"),
        ("/secrets/leases/D", "acme", "active"),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()))
            .with_indexed(status_key.clone(), IndexValue::Text(status.into()));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    let results = filesystem
        .query(
            &prefix,
            &Filter::And(vec![
                Filter::Eq {
                    key: scope_key,
                    value: IndexValue::Text("acme".into()),
                },
                Filter::Eq {
                    key: status_key,
                    value: IndexValue::Text("active".into()),
                },
            ]),
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    let mut paths: Vec<String> = results
        .iter()
        .map(|v| String::from_utf8_lossy(&v.entry.body).into_owned())
        .collect();
    paths.sort();
    // Both matching rows have empty bodies; verify by re-reading the
    // indexed projection on each result.
    let acme_active_count = results
        .iter()
        .filter(|v| {
            v.entry.indexed.get(&IndexKey::new("scope").unwrap())
                == Some(&IndexValue::Text("acme".into()))
                && v.entry.indexed.get(&IndexKey::new("status").unwrap())
                    == Some(&IndexValue::Text("active".into()))
        })
        .count();
    assert_eq!(acme_active_count, 2);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_prefix_filter_matches_text_prefix() {
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    let prefix = VirtualPath::new("/secrets/leases").unwrap();

    for (path, scope) in [
        ("/secrets/leases/X", "tenant:acme/u/1"),
        ("/secrets/leases/Y", "tenant:acme/u/2"),
        ("/secrets/leases/Z", "tenant:globex/u/1"),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    let results = filesystem
        .query(
            &prefix,
            &Filter::PrefixOn {
                key: scope_key,
                value: IndexValue::Text("tenant:acme/".into()),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_or_empty_matches_nothing_and_all_matches_every_row() {
    // PR #3661 reviewer fix: empty `Or` was returning every row instead
    // of none, and `Filter::All` was being skipped in compound contexts.
    // After the translator change every node emits a non-empty fragment
    // (`All` -> `TRUE`, empty `And` -> `TRUE`, empty `Or` -> `FALSE`).
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    for (path, scope) in [
        ("/secrets/leases/A", "acme"),
        ("/secrets/leases/B", "globex"),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    let prefix = VirtualPath::new("/secrets/leases").unwrap();

    // `All` matches every row.
    let all = filesystem
        .query(&prefix, &Filter::All, Page::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Empty `Or` matches nothing.
    let none = filesystem
        .query(&prefix, &Filter::Or(Vec::new()), Page::default())
        .await
        .unwrap();
    assert!(none.is_empty());

    // Empty `And` matches everything (identity).
    let and_empty = filesystem
        .query(&prefix, &Filter::And(Vec::new()), Page::default())
        .await
        .unwrap();
    assert_eq!(and_empty.len(), 2);

    // `And([All])` is well-formed and matches everything.
    let and_all = filesystem
        .query(&prefix, &Filter::And(vec![Filter::All]), Page::default())
        .await
        .unwrap();
    assert_eq!(and_all.len(), 2);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_prefix_filter_literal_percent_is_not_a_wildcard() {
    // PR #3661 reviewer fix: a literal prefix containing `%` was being
    // passed to LIKE with its `%` left unescaped (because the prior
    // escape helper preserved trailing `%`). `tenant:%` would then match
    // anything starting with `tenant:` instead of literally `tenant:%`.
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    for (path, scope) in [
        ("/secrets/leases/P1", "tenant:%"),
        ("/secrets/leases/P2", "tenant:acme"),
        ("/secrets/leases/P3", "tenant:globex"),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    // Literal-prefix `tenant:%` should match only the row whose stored
    // scope literally starts with `tenant:%`, not the two `tenant:` rows.
    let results = filesystem
        .query(
            &VirtualPath::new("/secrets/leases").unwrap(),
            &Filter::PrefixOn {
                key: scope_key,
                value: IndexValue::Text("tenant:%".into()),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_range_on_bool_finds_matching_rows() {
    // Regression test for the libSQL Range/Bool bug: SQLite's `json_type`
    // returns the literal strings `"true"` / `"false"` for JSON booleans
    // (not `"boolean"`/`"integer"`). A prior `json_type = 'integer'`
    // guard never matched and silently dropped every bool row. The fix
    // recognises both string variants; this test locks it in so a future
    // refactor of `index_value_json_type_guard` can't regress.
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("flag").unwrap();
    let flag_key = IndexKey::new("enabled").unwrap();
    let prefix = VirtualPath::new("/secrets/leases/bool_range").unwrap();
    for (path, enabled) in [
        ("/secrets/leases/bool_range/T", true),
        ("/secrets/leases/bool_range/F", false),
    ] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(flag_key.clone(), IndexValue::Bool(enabled));
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    // Range covering the full bool space — both rows must match.
    let results = filesystem
        .query(
            &prefix,
            &Filter::Range {
                key: flag_key.clone(),
                lo: IndexValue::Bool(false),
                hi: IndexValue::Bool(true),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        results.len(),
        2,
        "libSQL Range on Bool must return both rows; prior bug dropped them"
    );

    // Single-value range — only `true` row matches.
    let only_true = filesystem
        .query(
            &prefix,
            &Filter::Range {
                key: flag_key,
                lo: IndexValue::Bool(true),
                hi: IndexValue::Bool(true),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(only_true.len(), 1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_range_rejects_mixed_variant_bounds() {
    // Mixed-variant bounds (e.g. I64 lo + Text hi) used to silently fall
    // through to a lexicographic-on-text comparison that returned the
    // wrong rows. After the discriminant guard they're rejected with
    // Unsupported, matching the in-memory backend's
    // `discriminant(lo) == discriminant(hi)` requirement.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/secrets/leases/mixed").unwrap();
    let err = filesystem
        .query(
            &prefix,
            &Filter::Range {
                key: IndexKey::new("k").unwrap(),
                lo: IndexValue::I64(0),
                hi: IndexValue::Text("z".into()),
            },
            Page::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            FilesystemError::Unsupported {
                operation: FilesystemOperation::Query,
                ..
            }
        ),
        "expected Unsupported for mixed-variant Range bounds, got {err:?}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_vector_nearest_stable_tie_break_on_equal_cosine() {
    // Regression test for the tie-breaker fix: equal-cosine candidates
    // used to truncate non-deterministically because the SQL backends
    // omitted the secondary path comparator. Two identical embeddings
    // under different paths must now sort by path ascending and the
    // top-1 truncation must always pick the lex-smaller path.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory/tie_break").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let embedding_key = IndexKey::new("embedding").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("by_vec_tie").unwrap(),
        vec![embedding_key.clone()],
        IndexKind::Vector { dim: 3 },
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    let blob: Vec<u8> = [1.0_f32, 0.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    for leaf in ["zz", "aa", "mm"] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(embedding_key.clone(), IndexValue::Bytes(blob.clone()));
        filesystem
            .put(
                &VirtualPath::new(format!("/memory/tie_break/{leaf}")).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    // Three identical embeddings; the top-1 truncation must always pick
    // `aa` (lex-smallest) because the tie-breaker sorts by path.
    let top_one = filesystem
        .query(
            &prefix,
            &Filter::VectorNearest {
                key: embedding_key.clone(),
                embedding: vec![1.0, 0.0, 0.0],
                limit: 1,
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(top_one.len(), 1);
    assert_eq!(top_one[0].path.as_str(), "/memory/tie_break/aa");

    // Top-2 picks `aa` then `mm` deterministically.
    let top_two = filesystem
        .query(
            &prefix,
            &Filter::VectorNearest {
                key: embedding_key,
                embedding: vec![1.0, 0.0, 0.0],
                limit: 2,
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(top_two.len(), 2);
    assert_eq!(top_two[0].path.as_str(), "/memory/tie_break/aa");
    assert_eq!(top_two[1].path.as_str(), "/memory/tie_break/mm");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_query_paginates_results() {
    let filesystem = libsql_root().await;
    let kind = RecordKind::new("lease").unwrap();
    let scope_key = IndexKey::new("scope").unwrap();
    let prefix = VirtualPath::new("/secrets/leases").unwrap();

    for i in 0..7 {
        let entry = Entry::record(kind.clone(), &serde_json::json!({"i": i}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text("acme".into()));
        let path = VirtualPath::new(format!("/secrets/leases/page-{i:02}")).unwrap();
        filesystem
            .put(&path, entry, CasExpectation::Absent)
            .await
            .unwrap();
    }

    let first = filesystem
        .query(
            &prefix,
            &Filter::Eq {
                key: scope_key.clone(),
                value: IndexValue::Text("acme".into()),
            },
            Page::new(0, 3),
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 3);

    let second = filesystem
        .query(
            &prefix,
            &Filter::Eq {
                key: scope_key,
                value: IndexValue::Text("acme".into()),
            },
            Page::new(3, 3),
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 3);
    // Pages must not overlap (ordered by path).
    for entry in &second {
        assert!(!first.iter().any(|f| f.entry.body == entry.entry.body));
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_append_and_tail_assigns_monotonic_seqno() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/engine").unwrap();

    let s1 = filesystem.append(&log, b"a".to_vec()).await.unwrap();
    let s2 = filesystem.append(&log, b"b".to_vec()).await.unwrap();
    let s3 = filesystem.append(&log, b"c".to_vec()).await.unwrap();
    assert!(s1 < s2 && s2 < s3);

    // tail-from-zero returns every record in order.
    let from_zero = filesystem.tail(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(from_zero.len(), 3);
    assert_eq!(from_zero[0].payload, b"a".to_vec());
    assert_eq!(from_zero[1].payload, b"b".to_vec());
    assert_eq!(from_zero[2].payload, b"c".to_vec());
    assert_eq!(from_zero[0].seq, s1);
    assert_eq!(from_zero[2].seq, s3);

    // tail-from-N skips earlier records (exclusive).
    let from_first = filesystem.tail(&log, s1).await.unwrap();
    assert_eq!(from_first.len(), 2);
    assert_eq!(from_first[0].seq, s2);
    assert_eq!(from_first[1].seq, s3);

    // tail-from-last returns nothing.
    let from_last = filesystem.tail(&log, s3).await.unwrap();
    assert!(from_last.is_empty());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_tail_bounded_limits_records_before_materialization() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/bounded").unwrap();

    let s1 = filesystem.append(&log, b"a".to_vec()).await.unwrap();
    let s2 = filesystem.append(&log, b"b".to_vec()).await.unwrap();
    let s3 = filesystem.append(&log, b"c".to_vec()).await.unwrap();

    let none = filesystem.tail_bounded(&log, SeqNo::ZERO, 0).await.unwrap();
    let first_two = filesystem.tail_bounded(&log, SeqNo::ZERO, 2).await.unwrap();
    let after_first = filesystem.tail_bounded(&log, s1, 1).await.unwrap();

    assert!(none.is_empty());
    assert_eq!(first_two.len(), 2);
    assert_eq!(first_two[0].seq, s1);
    assert_eq!(first_two[1].seq, s2);
    assert_eq!(after_first.len(), 1);
    assert_eq!(after_first[0].seq, s2);
    assert_eq!(filesystem.tail_bounded(&log, s3, 1).await.unwrap().len(), 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_head_seq_returns_none_for_empty_path() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/empty-head").unwrap();
    let head = filesystem.head_seq(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(head, None);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_head_seq_returns_max_seq_after_appends() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/head-log").unwrap();
    let s1 = filesystem.append(&log, b"a".to_vec()).await.unwrap();
    let s2 = filesystem.append(&log, b"b".to_vec()).await.unwrap();
    let s3 = filesystem.append(&log, b"c".to_vec()).await.unwrap();
    assert!(s1 < s2 && s2 < s3);

    let head = filesystem.head_seq(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(head, Some(s3));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_head_seq_returns_none_when_from_exceeds_all_seqs() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/head-exhausted").unwrap();
    filesystem.append(&log, b"a".to_vec()).await.unwrap();
    let last = filesystem.append(&log, b"b".to_vec()).await.unwrap();

    let head = filesystem.head_seq(&log, last).await.unwrap();
    assert_eq!(head, None);

    let beyond = SeqNo::from_backend(last.get() + 100);
    let head = filesystem.head_seq(&log, beyond).await.unwrap();
    assert_eq!(head, None);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_append_distinct_paths_share_global_seq_but_are_isolated_on_tail() {
    // Each path's tail returns only its own records, even though the
    // underlying `INTEGER PRIMARY KEY AUTOINCREMENT` assigns global seqs.
    // What matters at the trait surface is that `tail(path, from)` filters
    // by path and that seqs are monotonic per path.
    let filesystem = libsql_root().await;
    let a = VirtualPath::new("/events/engine/a").unwrap();
    let b = VirtualPath::new("/events/engine/b").unwrap();

    let a1 = filesystem.append(&a, b"a1".to_vec()).await.unwrap();
    let b1 = filesystem.append(&b, b"b1".to_vec()).await.unwrap();
    let a2 = filesystem.append(&a, b"a2".to_vec()).await.unwrap();

    let tail_a = filesystem.tail(&a, SeqNo::ZERO).await.unwrap();
    let tail_b = filesystem.tail(&b, SeqNo::ZERO).await.unwrap();

    assert_eq!(tail_a.len(), 2);
    assert_eq!(tail_a[0].seq, a1);
    assert_eq!(tail_a[1].seq, a2);
    assert_eq!(tail_a[0].payload, b"a1".to_vec());
    assert_eq!(tail_a[1].payload, b"a2".to_vec());

    assert_eq!(tail_b.len(), 1);
    assert_eq!(tail_b[0].seq, b1);
    assert_eq!(tail_b[0].payload, b"b1".to_vec());

    // Per-path seq is monotonic.
    assert!(a1 < a2);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_capabilities_advertise_events() {
    let filesystem = libsql_root().await;
    assert!(filesystem.capabilities().has(Capability::Events));
}

#[cfg(feature = "libsql")]
async fn libsql_root() -> TestLibSqlRootFilesystem {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = LibSqlRootFilesystem::new(db);
    filesystem.run_migrations().await.unwrap();
    TestLibSqlRootFilesystem {
        filesystem,
        _dir: db_dir,
    }
}

// ─── Postgres behavioral tests ────────────────────────────────────────────
//
// PR #3659 reviewer flagged that the libsql contract suite has no Postgres
// counterpart, even though the Postgres backend ships substantial new code.
// These tests mirror the libsql shape (put/get round-trip, CAS Absent /
// Version / Any, query with Filter shapes, ensure_index conflict +
// race-idempotence, Range numeric vs text comparison) and gracefully skip
// when no Postgres is reachable via `DATABASE_URL` /
// `IRONCLAW_FILESYSTEM_POSTGRES_URL`.

#[cfg(feature = "postgres")]
mod postgres_tests {
    use super::*;
    use ironclaw_filesystem::{
        Capability, CasExpectation, Entry, FileType, FilesystemError, FilesystemOperation, Filter,
        IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page, PostgresRootFilesystem,
        RecordKind, SeqNo, TxnCapability,
    };
    use ironclaw_host_api::VirtualPath;

    async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
        if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
            return None;
        }
        let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()?;
        let config = url.parse::<tokio_postgres::Config>().ok()?;
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .ok()
    }

    /// Build a fresh Postgres-backed filesystem with migrations applied.
    /// Returns `None` if no Postgres is reachable — caller must early-return
    /// so the test passes in environments without a DB. Each test uses a
    /// unique path prefix so concurrent runs against a shared DB don't
    /// interfere.
    async fn postgres_root() -> Option<(PostgresRootFilesystem, String)> {
        let pool = postgres_pool().await?;
        let fs = PostgresRootFilesystem::new(pool);
        fs.run_migrations().await.ok()?;
        // Unique per-test prefix under /secrets/leases (a known VirtualPath
        // root). Concurrent test runs against the same Postgres get
        // isolation via the prefix; cleanup happens by the next test's
        // delete on its own prefix or by the test DB being torn down
        // between runs.
        let prefix = format!("/secrets/leases/pgtest_{}", uuid::Uuid::new_v4().simple());
        Some((fs, prefix))
    }

    fn vpath(prefix: &str, leaf: &str) -> VirtualPath {
        VirtualPath::new(format!("{prefix}/{leaf}")).unwrap()
    }

    #[tokio::test]
    async fn postgres_native_put_get_round_trip_with_record_metadata() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "L1");
        let kind = RecordKind::new("credential_lease").unwrap();
        let scope_key = IndexKey::new("scope").unwrap();
        let status_key = IndexKey::new("status").unwrap();
        let entry = Entry::record(kind.clone(), &serde_json::json!({"hidden": true}))
            .unwrap()
            .with_indexed(scope_key.clone(), IndexValue::Text("acme".into()))
            .with_indexed(status_key.clone(), IndexValue::Text("active".into()));

        let version1 = fs.put(&path, entry, CasExpectation::Absent).await.unwrap();
        assert_eq!(version1.get(), 1);

        let got = fs
            .get(&path)
            .await
            .unwrap()
            .expect("entry should be present");
        assert_eq!(got.version, version1);
        assert_eq!(got.entry.kind.as_ref(), Some(&kind));
        assert_eq!(got.entry.indexed.len(), 2);
        assert!(got.entry.indexed.contains_key(&scope_key));
        assert!(got.entry.indexed.contains_key(&status_key));
    }

    #[tokio::test]
    async fn postgres_native_put_cas_absent_rejects_existing_path() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "L2");
        fs.put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let err = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Absent)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
    }

    #[tokio::test]
    async fn postgres_native_put_cas_version_advances_and_rejects_stale() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "L3");
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let v2 = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Version(v1))
            .await
            .unwrap();
        assert!(v2 > v1);
        let err = fs
            .put(&path, Entry::bytes(vec![3]), CasExpectation::Version(v1))
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
    }

    #[tokio::test]
    async fn postgres_put_cas_version_on_missing_path_reports_no_found_version() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let missing = vpath(&prefix, "cas_version_missing");
        let err = fs
            .put(
                &missing,
                Entry::bytes(vec![1]),
                CasExpectation::Version(ironclaw_filesystem::RecordVersion::from_backend(1)),
            )
            .await
            .expect_err("version CAS on a missing path must fail");
        match err {
            FilesystemError::VersionMismatch { found, .. } => {
                assert!(
                    found.is_none(),
                    "missing path should report no found version, got: {found:?}"
                );
            }
            other => panic!("expected VersionMismatch, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn postgres_native_put_cas_any_increments_existing_version() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "L4");
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let v2 = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Any)
            .await
            .unwrap();
        assert_eq!(v2.get(), v1.get() + 1);
        let got = fs.get(&path).await.unwrap().unwrap();
        assert_eq!(got.version, v2);
        assert_eq!(got.entry.body, vec![2]);
    }

    #[tokio::test]
    async fn postgres_put_cas_any_inserts_missing_path_and_returns_version() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let missing = vpath(&prefix, "cas_any_insert_missing");
        let v1 = fs
            .put(&missing, Entry::bytes(vec![7]), CasExpectation::Any)
            .await
            .expect("Any insert on a missing path must succeed");
        assert_eq!(v1, ironclaw_filesystem::RecordVersion::from_backend(1));
        let got = fs.get(&missing).await.unwrap().unwrap();
        assert_eq!(got.entry.body, vec![7]);
    }

    // CAS-put directory invariant (folded into the single write statement by
    // the round-trip fix). Mirrors the libsql `write_file` rejection tests but
    // drives `put` directly, which is the primitive the SQL fold changed.

    #[tokio::test]
    async fn postgres_put_rejects_implicit_directory() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        // Writing a child first makes `dir` an implicit directory.
        let dir = vpath(&prefix, "implicit");
        let child = vpath(&prefix, "implicit/leaf");
        fs.put(&child, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();

        // Every CAS arm must refuse to overwrite the implicit directory.
        for cas in [
            CasExpectation::Absent,
            CasExpectation::Any,
            CasExpectation::Version(ironclaw_filesystem::RecordVersion::from_backend(1)),
        ] {
            let err = fs
                .put(&dir, Entry::bytes(vec![2]), cas)
                .await
                .expect_err("put over an implicit directory must fail");
            assert!(
                matches!(
                    err,
                    FilesystemError::Backend {
                        operation: FilesystemOperation::WriteFile,
                        ..
                    }
                ),
                "expected directory-write Backend error, got: {err:?}"
            );
        }
        // The child is untouched.
        assert_eq!(fs.get(&child).await.unwrap().unwrap().entry.body, vec![1]);
    }

    #[tokio::test]
    async fn postgres_put_rejects_existing_directory() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let dir = VirtualPath::new(format!("{prefix}/explicit")).unwrap();
        // create_dir_all materializes explicit directory rows (is_dir = TRUE).
        // Keep `dir` childless so the exact-path explicit-directory guard
        // (ON CONFLICT / is_dir = FALSE) is what rejects the put, not the
        // descendant scan (covered by postgres_put_rejects_implicit_directory).
        fs.create_dir_all(&dir).await.unwrap();

        // Every CAS arm (distinct SQL: PUT_ABSENT_SQL / PUT_VERSION_SQL /
        // PUT_ANY_SQL) must refuse to overwrite the explicit directory.
        for cas in [
            CasExpectation::Absent,
            CasExpectation::Any,
            CasExpectation::Version(ironclaw_filesystem::RecordVersion::from_backend(1)),
        ] {
            let err = fs
                .put(&dir, Entry::bytes(vec![2]), cas)
                .await
                .expect_err("put over an explicit directory must fail");
            assert!(
                matches!(
                    err,
                    FilesystemError::Backend {
                        operation: FilesystemOperation::WriteFile,
                        ..
                    }
                ),
                "expected directory-write Backend error, got: {err:?}"
            );
        }
        assert_eq!(fs.stat(&dir).await.unwrap().file_type, FileType::Directory);
    }

    #[tokio::test]
    async fn postgres_transaction_rollback_discards_prior_put_after_later_cas_conflict() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        assert_eq!(fs.capabilities().txn(), TxnCapability::MultiKey);

        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let pending = vpath(&prefix, "txn_pending");
        let existing = vpath(&prefix, "txn_existing");
        fs.put(
            &existing,
            Entry::bytes(b"already committed".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

        let mut txn = fs.begin(&prefix_path).await.unwrap();
        txn.put(
            &pending,
            Entry::bytes(b"must roll back".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
        let err = txn
            .put(
                &existing,
                Entry::bytes(b"conflicting rewrite".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
        txn.rollback().await;

        assert!(fs.get(&pending).await.unwrap().is_none());
        let got = fs.get(&existing).await.unwrap().unwrap();
        assert_eq!(got.entry.body, b"already committed");
    }

    #[tokio::test]
    async fn postgres_get_returns_none_for_missing_path() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "missing");
        assert!(fs.get(&path).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn postgres_ensure_index_is_idempotent_and_conflict_aware() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(prefix).unwrap();
        let name = IndexName::new("by_scope_status").unwrap();
        let keys = vec![
            IndexKey::new("scope").unwrap(),
            IndexKey::new("status").unwrap(),
        ];
        let spec_exact = IndexSpec::new(name.clone(), keys.clone(), IndexKind::Exact);
        let spec_prefix = IndexSpec::new(name, keys, IndexKind::Prefix);

        fs.ensure_index(&prefix_path, &spec_exact).await.unwrap();
        // Idempotent re-declaration.
        fs.ensure_index(&prefix_path, &spec_exact).await.unwrap();
        // Conflicting kind under same name fails.
        let err = fs
            .ensure_index(&prefix_path, &spec_prefix)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::IndexConflict { .. }));
    }

    #[tokio::test]
    async fn postgres_query_filters_on_indexed_projection() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("lease").unwrap();
        let scope_key = IndexKey::new("scope").unwrap();
        let status_key = IndexKey::new("status").unwrap();

        for (leaf, scope, status) in [
            ("A", "acme", "active"),
            ("B", "acme", "revoked"),
            ("C", "globex", "active"),
            ("D", "acme", "active"),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()))
                .with_indexed(status_key.clone(), IndexValue::Text(status.into()));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }

        let results = fs
            .query(
                &prefix_path,
                &Filter::And(vec![
                    Filter::Eq {
                        key: scope_key,
                        value: IndexValue::Text("acme".into()),
                    },
                    Filter::Eq {
                        key: status_key,
                        value: IndexValue::Text("active".into()),
                    },
                ]),
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn postgres_query_range_on_i64_is_numeric_not_lexicographic() {
        // PR #3661 reviewer: Postgres Range on IndexValue::I64 used to be
        // lexicographic via `indexed->>'key' BETWEEN ...` on text. The fix
        // casts both sides to BIGINT so `2..10` includes `9` but not `99`.
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("widget").unwrap();
        let size = IndexKey::new("size").unwrap();
        for (leaf, n) in [
            ("W2", 2i64),
            ("W9", 9),
            ("W10", 10),
            ("W11", 11),
            ("W99", 99),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(size.clone(), IndexValue::I64(n));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        // Range 2..=10 should include {2, 9, 10} numerically. Lexicographic
        // comparison on '2' / '10' would miss `9` (since "9" > "10" as text).
        let results = fs
            .query(
                &prefix_path,
                &Filter::Range {
                    key: size,
                    lo: IndexValue::I64(2),
                    hi: IndexValue::I64(10),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn postgres_query_range_rejects_mixed_variant_bounds() {
        // Mixed-variant bounds used to silently lex-compare on text and
        // return the wrong rows. The discriminant guard now rejects them
        // with Unsupported on Postgres just like the in-memory and libSQL
        // backends, keeping cross-backend semantics aligned.
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let err = fs
            .query(
                &prefix_path,
                &Filter::Range {
                    key: IndexKey::new("k").unwrap(),
                    lo: IndexValue::I64(0),
                    hi: IndexValue::Text("z".into()),
                },
                Page::default(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }
            ),
            "expected Unsupported for mixed-variant Range bounds, got {err:?}"
        );
    }

    #[tokio::test]
    async fn postgres_vector_nearest_stable_tie_break_on_equal_cosine() {
        // Equal-cosine candidates must truncate deterministically.
        // Mirrors the libSQL tie-break test so cross-backend behavior
        // stays aligned with the in-memory reference (which has carried
        // the secondary `path.cmp` tie-breaker since the original
        // implementation).
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("chunk").unwrap();
        let embedding_key = IndexKey::new("embedding").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("by_vec_tie").unwrap(),
            vec![embedding_key.clone()],
            IndexKind::Vector { dim: 3 },
        );
        fs.ensure_index(&prefix_path, &spec).await.unwrap();
        let blob: Vec<u8> = [1.0_f32, 0.0, 0.0]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        for leaf in ["zz", "aa", "mm"] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(embedding_key.clone(), IndexValue::Bytes(blob.clone()));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let top_one = fs
            .query(
                &prefix_path,
                &Filter::VectorNearest {
                    key: embedding_key,
                    embedding: vec![1.0, 0.0, 0.0],
                    limit: 1,
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(top_one.len(), 1);
        // The lex-smallest path among the three identical embeddings wins.
        assert!(
            top_one[0].path.as_str().ends_with("/aa"),
            "expected /aa to win lex tie-break, got {}",
            top_one[0].path
        );
    }

    #[tokio::test]
    async fn postgres_query_prefix_filter_literal_percent_is_not_a_wildcard() {
        // PR #3661 reviewer: literal `tenant:%` must not become a wildcard.
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("lease").unwrap();
        let scope_key = IndexKey::new("scope").unwrap();
        for (leaf, scope) in [
            ("P1", "tenant:%"),
            ("P2", "tenant:acme"),
            ("P3", "tenant:globex"),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &prefix_path,
                &Filter::PrefixOn {
                    key: scope_key,
                    value: IndexValue::Text("tenant:%".into()),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn postgres_query_or_empty_matches_nothing_and_all_matches_every_row() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("lease").unwrap();
        let scope_key = IndexKey::new("scope").unwrap();
        for (leaf, scope) in [("A", "acme"), ("B", "globex")] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(scope_key.clone(), IndexValue::Text(scope.into()));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }

        assert_eq!(
            fs.query(&prefix_path, &Filter::All, Page::default())
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(
            fs.query(&prefix_path, &Filter::Or(Vec::new()), Page::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fs.query(&prefix_path, &Filter::And(Vec::new()), Page::default())
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn postgres_fts_index_filter_finds_documents_by_token() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("chunk").unwrap();
        let content = IndexKey::new("content").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("by_content").unwrap(),
            vec![content.clone()],
            IndexKind::Fts,
        );
        fs.ensure_index(&prefix_path, &spec).await.unwrap();
        // Redeclaration is idempotent.
        fs.ensure_index(&prefix_path, &spec).await.unwrap();
        for (leaf, body) in [
            ("a", "the quick brown fox jumps"),
            ("b", "the lazy dog sleeps"),
            ("c", "a brown bear naps"),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(content.clone(), IndexValue::Text(body.into()));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &prefix_path,
                &Filter::Fts {
                    key: content,
                    query: "brown".into(),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn postgres_fts_index_predicate_is_scoped_to_declaring_prefix() {
        // Audit finding F4: the Postgres GIN FTS index used to be
        // global over root_filesystem_entries. libsql FTS5 vtables are
        // declared per-mount-prefix, so cross-backend parity required a
        // partial index gated on `path LIKE '<prefix>/%' OR path =
        // '<prefix>'`. Verify the DDL emits the predicate by reading
        // it back from pg_indexes.
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let content = IndexKey::new("content").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("by_content_scoped").unwrap(),
            vec![content.clone()],
            IndexKind::Fts,
        );
        fs.ensure_index(&prefix_path, &spec).await.unwrap();

        // Use a fresh client; we don't have direct access to the pool
        // through `fs`, so re-derive it from the same env vars.
        let pool = postgres_pool().await.expect("pool available");
        let client = pool.get().await.unwrap();
        let row = client
            .query_one(
                "SELECT indexdef FROM pg_indexes \
                 WHERE schemaname = current_schema() \
                   AND tablename = 'root_filesystem_entries' \
                   AND indexname LIKE 'idx_rfs_%' \
                 ORDER BY indexname DESC LIMIT 1",
                &[],
            )
            .await
            .expect("at least one rfs index visible");
        let indexdef: String = row.get("indexdef");
        assert!(
            indexdef.contains(prefix.as_str()),
            "GIN FTS index DDL must include the declaring prefix as a partial-index \
             predicate, got: {indexdef}"
        );
        assert!(
            indexdef.contains("WHERE") || indexdef.to_lowercase().contains("where"),
            "GIN FTS index DDL must be a partial index, got: {indexdef}"
        );
    }

    #[tokio::test]
    async fn postgres_vector_index_ranks_by_cosine_brute_force() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let kind = RecordKind::new("chunk").unwrap();
        let embedding_key = IndexKey::new("embedding").unwrap();
        let spec = IndexSpec::new(
            IndexName::new("by_vec").unwrap(),
            vec![embedding_key.clone()],
            IndexKind::Vector { dim: 3 },
        );
        fs.ensure_index(&prefix_path, &spec).await.unwrap();
        // Re-declaration with a different dim is rejected.
        let conflict = IndexSpec::new(
            IndexName::new("by_vec").unwrap(),
            vec![embedding_key.clone()],
            IndexKind::Vector { dim: 4 },
        );
        let err = fs.ensure_index(&prefix_path, &conflict).await.unwrap_err();
        assert!(matches!(err, FilesystemError::IndexConflict { .. }));

        let blob = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() };
        for (leaf, vec) in [
            ("A", vec![1.0_f32, 0.0, 0.0]),
            ("B", vec![0.9, 0.1, 0.0]),
            ("C", vec![0.0, 0.0, 1.0]),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(embedding_key.clone(), IndexValue::Bytes(blob(&vec)));
            fs.put(&vpath(&prefix, leaf), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &prefix_path,
                &Filter::VectorNearest {
                    key: embedding_key.clone(),
                    embedding: vec![1.0, 0.0, 0.0],
                    limit: 2,
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        // First result must be the identical vector (A).
        assert_eq!(
            results[0].entry.indexed.get(&embedding_key),
            Some(&IndexValue::Bytes(blob(&[1.0, 0.0, 0.0])))
        );
    }

    #[tokio::test]
    async fn postgres_write_file_after_put_resets_record_metadata_and_bumps_version() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "STALE");
        let kind = RecordKind::new("credential_lease").unwrap();
        let scope = IndexKey::new("scope").unwrap();
        let record_entry = Entry::record(kind, &serde_json::json!({"k": 1}))
            .unwrap()
            .with_indexed(scope, IndexValue::Text("acme".into()));

        let v1 = fs
            .put(&path, record_entry, CasExpectation::Absent)
            .await
            .unwrap();

        // Legacy write must reset record metadata + bump version.
        #[allow(deprecated)]
        fs.write_file(&path, b"opaque").await.unwrap();

        let got = fs.get(&path).await.unwrap().unwrap();
        assert!(got.entry.kind.is_none());
        assert!(got.entry.indexed.is_empty());
        assert_eq!(got.entry.body, b"opaque");
        assert!(got.version > v1);
    }

    #[tokio::test]
    async fn postgres_append_and_tail_assigns_monotonic_seqno() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        // Per-test unique log path (under `/secrets/leases` as a known
        // VirtualPath root) so concurrent runs against a shared DB don't
        // see each other's events.
        let log = VirtualPath::new(format!("{prefix}/events_log")).unwrap();

        let s1 = fs.append(&log, b"a".to_vec()).await.unwrap();
        let s2 = fs.append(&log, b"b".to_vec()).await.unwrap();
        let s3 = fs.append(&log, b"c".to_vec()).await.unwrap();
        assert!(s1 < s2 && s2 < s3);

        // tail-from-zero returns every record in order, with correct payloads.
        let from_zero = fs.tail(&log, SeqNo::ZERO).await.unwrap();
        assert_eq!(from_zero.len(), 3);
        assert_eq!(from_zero[0].payload, b"a".to_vec());
        assert_eq!(from_zero[1].payload, b"b".to_vec());
        assert_eq!(from_zero[2].payload, b"c".to_vec());
        assert_eq!(from_zero[0].seq, s1);
        assert_eq!(from_zero[2].seq, s3);

        // tail-from-N skips earlier records (exclusive).
        let from_first = fs.tail(&log, s1).await.unwrap();
        assert_eq!(from_first.len(), 2);
        assert_eq!(from_first[0].seq, s2);
        assert_eq!(from_first[1].seq, s3);

        // tail-from-last returns nothing.
        assert!(fs.tail(&log, s3).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn postgres_tail_bounded_limits_records_before_materialization() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let log = VirtualPath::new(format!("{prefix}/events_bounded")).unwrap();

        let s1 = fs.append(&log, b"a".to_vec()).await.unwrap();
        let s2 = fs.append(&log, b"b".to_vec()).await.unwrap();
        let s3 = fs.append(&log, b"c".to_vec()).await.unwrap();

        let none = fs.tail_bounded(&log, SeqNo::ZERO, 0).await.unwrap();
        let first_two = fs.tail_bounded(&log, SeqNo::ZERO, 2).await.unwrap();
        let after_first = fs.tail_bounded(&log, s1, 1).await.unwrap();

        assert!(none.is_empty());
        assert_eq!(first_two.len(), 2);
        assert_eq!(first_two[0].seq, s1);
        assert_eq!(first_two[1].seq, s2);
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0].seq, s2);
        assert_eq!(fs.tail_bounded(&log, s3, 1).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn postgres_head_seq_returns_none_for_empty_path() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let log = VirtualPath::new(format!("{prefix}/head_empty")).unwrap();
        let head = fs.head_seq(&log, SeqNo::ZERO).await.unwrap();
        assert_eq!(head, None);
    }

    #[tokio::test]
    async fn postgres_head_seq_returns_max_seq_after_appends() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let log = VirtualPath::new(format!("{prefix}/head_log")).unwrap();
        let s1 = fs.append(&log, b"a".to_vec()).await.unwrap();
        let s2 = fs.append(&log, b"b".to_vec()).await.unwrap();
        let s3 = fs.append(&log, b"c".to_vec()).await.unwrap();
        assert!(s1 < s2 && s2 < s3);

        let head = fs.head_seq(&log, SeqNo::ZERO).await.unwrap();
        assert_eq!(head, Some(s3));
    }

    #[tokio::test]
    async fn postgres_head_seq_returns_none_when_from_exceeds_all_seqs() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let log = VirtualPath::new(format!("{prefix}/head_exhausted")).unwrap();
        fs.append(&log, b"a".to_vec()).await.unwrap();
        let last = fs.append(&log, b"b".to_vec()).await.unwrap();

        let head = fs.head_seq(&log, last).await.unwrap();
        assert_eq!(head, None);

        let beyond = SeqNo::from_backend(last.get() + 100);
        let head = fs.head_seq(&log, beyond).await.unwrap();
        assert_eq!(head, None);
    }

    #[tokio::test]
    async fn postgres_append_distinct_paths_are_isolated_on_tail() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let a = VirtualPath::new(format!("{prefix}/events_a")).unwrap();
        let b = VirtualPath::new(format!("{prefix}/events_b")).unwrap();

        let a1 = fs.append(&a, b"a1".to_vec()).await.unwrap();
        let _ = fs.append(&b, b"b1".to_vec()).await.unwrap();
        let a2 = fs.append(&a, b"a2".to_vec()).await.unwrap();

        let tail_a = fs.tail(&a, SeqNo::ZERO).await.unwrap();
        let tail_b = fs.tail(&b, SeqNo::ZERO).await.unwrap();

        assert_eq!(tail_a.len(), 2);
        assert_eq!(tail_a[0].seq, a1);
        assert_eq!(tail_a[1].seq, a2);
        assert_eq!(tail_a[0].payload, b"a1".to_vec());
        assert_eq!(tail_a[1].payload, b"a2".to_vec());

        assert_eq!(tail_b.len(), 1);
        assert_eq!(tail_b[0].payload, b"b1".to_vec());

        // Per-path seq is monotonic even though the BIGSERIAL is shared.
        assert!(a1 < a2);
    }

    #[tokio::test]
    async fn postgres_capabilities_advertise_events() {
        let Some((fs, _prefix)) = postgres_root().await else {
            return;
        };
        assert!(fs.capabilities().has(Capability::Events));
    }
}
