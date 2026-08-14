// arch-exempt: large_file, backend parity contracts stay in one shared behavioral suite, plan #5274
use std::time::Duration;

use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_filesystem::{
    AtomicSubtreeEntry, Capability, CasExpectation, Entry, FileType, FilesystemError,
    FilesystemOperation, Filter, InMemoryBackend, IndexKey, IndexKind, IndexName, IndexSpec,
    IndexValue, LibSqlRootFilesystem, OrderedPage, Page, RecordKind, SeqNo, SortDirection,
};
use ironclaw_host_api::path::VirtualPath;

#[tokio::test]
async fn libsql_create_subtree_atomic_publishes_the_complete_batch() {
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-1").unwrap();
    let first =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-1/0-alpha.txt").unwrap();
    let second =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-1/1-beta.txt").unwrap();

    let versions = filesystem
        .create_subtree_atomic(
            &prefix,
            vec![
                AtomicSubtreeEntry {
                    path: first.clone(),
                    entry: Entry::bytes(b"alpha".to_vec()),
                },
                AtomicSubtreeEntry {
                    path: second.clone(),
                    entry: Entry::bytes(b"beta".to_vec()),
                },
            ],
        )
        .await
        .unwrap();

    assert_eq!(versions.len(), 2);
    assert_eq!(filesystem.read_file(&first).await.unwrap(), b"alpha");
    assert_eq!(filesystem.read_file(&second).await.unwrap(), b"beta");
}

#[tokio::test]
async fn libsql_create_subtree_atomic_rejects_conflicts_without_overwrite() {
    let filesystem = libsql_root().await;
    let prefix =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-conflict").unwrap();
    let file =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-conflict/0.txt").unwrap();
    filesystem
        .create_subtree_atomic(
            &prefix,
            vec![AtomicSubtreeEntry {
                path: file.clone(),
                entry: Entry::bytes(b"original".to_vec()),
            }],
        )
        .await
        .unwrap();

    let error = filesystem
        .create_subtree_atomic(
            &prefix,
            vec![AtomicSubtreeEntry {
                path: file.clone(),
                entry: Entry::bytes(b"replacement".to_vec()),
            }],
        )
        .await
        .unwrap_err();

    assert!(matches!(error, FilesystemError::VersionMismatch { .. }));
    assert_eq!(filesystem.read_file(&file).await.unwrap(), b"original");
}

#[tokio::test]
async fn libsql_create_subtree_atomic_rejects_invalid_batch_without_partial_write() {
    let filesystem = libsql_root().await;
    let prefix =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-invalid").unwrap();
    let valid =
        VirtualPath::new("/engine/tenants/t1/users/u1/attachments/message-invalid/0.txt").unwrap();
    let outside = VirtualPath::new("/engine/tenants/t1/users/u2/escaped.txt").unwrap();

    let error = filesystem
        .create_subtree_atomic(
            &prefix,
            vec![
                AtomicSubtreeEntry {
                    path: valid.clone(),
                    entry: Entry::bytes(b"valid".to_vec()),
                },
                AtomicSubtreeEntry {
                    path: outside,
                    entry: Entry::bytes(b"escaped".to_vec()),
                },
            ],
        )
        .await
        .unwrap_err();

    assert!(matches!(error, FilesystemError::PathOutsideMount { .. }));
    assert!(filesystem.get(&valid).await.unwrap().is_none());
}

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
#[tokio::test]
async fn libsql_root_filesystem_overwrites_existing_file() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/memory/tenants/t1/users/u1/facts.md").unwrap();

    filesystem.write_file(&path, b"first").await.unwrap();
    filesystem.write_file(&path, b"second").await.unwrap();

    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"second");
    assert_eq!(filesystem.stat(&path).await.unwrap().len, 6);
}
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
#[test]
fn postgres_root_filesystem_implements_root_filesystem_contract() {
    fn assert_root<T: RootFilesystem>() {}
    assert_root::<PostgresRootFilesystem>();
}
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
    let filesystem = LibSqlRootFilesystem::new(locked_db).expect("filesystem runtime");
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
struct TestLibSqlRootFilesystem {
    filesystem: LibSqlRootFilesystem,
    _dir: tempfile::TempDir,
}
impl std::ops::Deref for TestLibSqlRootFilesystem {
    type Target = LibSqlRootFilesystem;

    fn deref(&self) -> &Self::Target {
        &self.filesystem
    }
}
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

/// Mirrors `postgres_put_cas_version_on_missing_path_reports_no_found_version`:
/// a `Version` CAS write against a path with no existing row must fail
/// closed with `found: None` rather than panicking or reporting a stale
/// version, so callers can distinguish "never written" from "someone
/// else won the race" on the version-mismatch error.
#[tokio::test]
async fn libsql_native_put_cas_version_on_missing_path_reports_no_found_version() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/L3-missing").unwrap();
    let err = filesystem
        .put(
            &path,
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
async fn libsql_delete_if_version_deletes_current_and_rejects_stale_or_missing() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/CAS-DEL").unwrap();

    // Missing path → NotFound (already gone, benign), never VersionMismatch.
    let err = filesystem
        .delete_if_version(&path, ironclaw_filesystem::RecordVersion::from_backend(1))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::NotFound {
            operation: FilesystemOperation::Delete,
            ..
        }
    ));

    // Simulates the state a concurrent writer would leave behind (this is a
    // sequential script, not a real race — see concurrent_cas_storm.rs for
    // genuine parallel coverage): the v1 the deleter read is bumped to v2
    // before the delete lands → the stale delete loses with the observed
    // version and the entry survives at v2.
    let v1 = filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();
    let log_seq = filesystem.append(&path, b"kept".to_vec()).await.unwrap();
    let v2 = filesystem
        .put(&path, Entry::bytes(vec![2]), CasExpectation::Version(v1))
        .await
        .unwrap();
    let err = filesystem.delete_if_version(&path, v1).await.unwrap_err();
    match err {
        FilesystemError::VersionMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, Some(v1));
            assert_eq!(found, Some(v2));
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
    let got = filesystem.get(&path).await.unwrap().unwrap();
    assert_eq!(got.version, v2);
    assert_eq!(got.entry.body, vec![2]);

    // Correct version deletes exactly the entry; single-key, so the event
    // log at the same path survives (blind `delete` sweeps it).
    filesystem.delete_if_version(&path, v2).await.unwrap();
    assert!(filesystem.get(&path).await.unwrap().is_none());
    let log = filesystem.tail(&path, SeqNo::ZERO).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].seq, log_seq);
}

/// Review fix (PR #5749): an `expected_version` beyond `i64::MAX` must
/// surface `CorruptRecordVersion` (audit finding F6's overflow guard) rather
/// than being silently truncated into a bind parameter that could never
/// match — and the guard must fire before any DELETE runs, so the entry
/// survives untouched.
#[tokio::test]
async fn libsql_delete_if_version_rejects_out_of_range_expected_version() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/CAS-DEL-OVERFLOW").unwrap();
    let v1 = filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();

    let err = filesystem
        .delete_if_version(
            &path,
            ironclaw_filesystem::RecordVersion::from_backend(u64::MAX),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, FilesystemError::CorruptRecordVersion { .. }),
        "expected CorruptRecordVersion, got {err:?}"
    );

    let got = filesystem.get(&path).await.unwrap().unwrap();
    assert_eq!(got.version, v1);
    assert_eq!(got.entry.body, vec![1]);
}

/// Round-B review: the ABA hazard `delete_if_version`'s trait doc warns
/// about (version tokens are not generation-stable) was previously pinned
/// only for the in-memory backend. libSQL shares the same "version
/// restarts at 1 after a full delete" precondition — pin it here too so a
/// future change to libSQL's version-assignment (e.g. a sequence that
/// doesn't reset) can't silently invalidate the trait doc's warning for
/// this backend without a test noticing.
#[tokio::test]
async fn libsql_delete_if_version_is_vulnerable_to_aba_across_delete_recreate_cycles() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/CAS-DEL-ABA").unwrap();

    let v1_first = filesystem
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();
    filesystem.delete_if_version(&path, v1_first).await.unwrap();
    assert!(filesystem.get(&path).await.unwrap().is_none());

    let v1_second = filesystem
        .put(&path, Entry::bytes(vec![2]), CasExpectation::Absent)
        .await
        .unwrap();
    assert_eq!(
        v1_first, v1_second,
        "version must restart after a full delete, or this ABA hazard doesn't apply"
    );

    // The stale `v1_first` token wrongly authorizes deleting the second
    // incarnation's live data — documented hazard, not a regression.
    filesystem.delete_if_version(&path, v1_first).await.unwrap();
    assert!(
        filesystem.get(&path).await.unwrap().is_none(),
        "stale version token wrongly matched and deleted the second incarnation"
    );
}

/// Round-C review (PR #5749): no test drove `delete_if_version` against an
/// explicit directory row (`is_dir = TRUE`, via `create_dir_all`) to confirm
/// the `is_dir = 0` scoping — shared with `put`'s Version arm and
/// `current_version_libsql` — actually excludes it, the way
/// `libsql_put_rejects_existing_directory`-style tests already pin for
/// `put`. A directory-only path must diagnose as `NotFound` (no file-plane
/// row at that path), never match/delete the directory row.
#[tokio::test]
async fn libsql_delete_if_version_excludes_explicit_directory_row() {
    let filesystem = libsql_root().await;
    let dir = VirtualPath::new("/secrets/leases/CAS-DEL-DIR").unwrap();
    filesystem.create_dir_all(&dir).await.unwrap();

    let err = filesystem
        .delete_if_version(&dir, ironclaw_filesystem::RecordVersion::from_backend(1))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            FilesystemError::NotFound {
                operation: FilesystemOperation::Delete,
                ..
            }
        ),
        "delete_if_version must not match an explicit directory row \
         (is_dir = TRUE), got: {err:?}"
    );
}
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
#[tokio::test]
async fn libsql_get_returns_none_for_missing_path() {
    let filesystem = libsql_root().await;
    let path = VirtualPath::new("/secrets/leases/missing").unwrap();
    assert!(filesystem.get(&path).await.unwrap().is_none());
}
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

#[tokio::test]
async fn libsql_fts_treats_free_form_queries_as_plain_text() {
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory/plain-query").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let content = IndexKey::new("content").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("by_content_plain_query").unwrap(),
        vec![content.clone()],
        IndexKind::Fts,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    let entry = Entry::record(kind.clone(), &serde_json::json!({}))
        .unwrap()
        .with_indexed(
            content.clone(),
            IndexValue::Text("launch-code-plum-42 unlocks staging".into()),
        );
    filesystem
        .put(
            &VirtualPath::new("/memory/plain-query/a").unwrap(),
            entry,
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    // Negative document: shares only some terms with every multi-term query
    // below, so it can tell the FTS5 implicit-AND join apart from an OR join.
    // If terms were OR-joined (or a required term were wrongly stop-listed),
    // this partial match would leak into the results.
    let partial = Entry::record(kind, &serde_json::json!({}))
        .unwrap()
        .with_indexed(
            content.clone(),
            IndexValue::Text("banana-99 staging".into()),
        );
    filesystem
        .put(
            &VirtualPath::new("/memory/plain-query/partial").unwrap(),
            partial,
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    for query in [
        "What is the staging launch code?",
        "launch-code-plum-42?",
        "staging (unlocks)",
        "launch AND code",
        "launch OR code",
        "launch NOT code",
    ] {
        let results = filesystem
            .query(
                &prefix,
                &Filter::Fts {
                    key: content.clone(),
                    query: query.into(),
                },
                Page::default(),
            )
            .await
            .unwrap_or_else(|error| panic!("plain FTS query {query:?} failed: {error:?}"));
        assert_eq!(
            results.len(),
            1,
            "plain FTS query {query:?} must require every term (partial doc excluded)"
        );
        assert!(
            !results
                .iter()
                .any(|result| result.path.as_str().ends_with("/partial")),
            "plain FTS query {query:?} must not return the partial-match document"
        );
    }

    // The negative document is itself searchable: a single-term query for a
    // term it contains must return it, proving the exclusion above is
    // term-based rather than a total index failure.
    let partial_only = filesystem
        .query(
            &prefix,
            &Filter::Fts {
                key: content.clone(),
                query: "banana-99".into(),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(partial_only.len(), 1);
    assert!(partial_only[0].path.as_str().ends_with("/partial"));

    let punctuation_only = filesystem
        .query(
            &prefix,
            &Filter::Fts {
                key: content,
                query: "?!()".into(),
            },
            Page::default(),
        )
        .await
        .expect("punctuation-only FTS is a valid empty query");
    assert!(punctuation_only.is_empty());
}

#[tokio::test]
async fn libsql_repeated_fts_declaration_does_not_wait_for_the_writer() {
    // Regression for #7283: memory search re-declares its FTS index on the
    // query hot path. Once the cataloged declaration has committed, checking
    // it must remain available while an unrelated durable write owns libSQL's
    // single writer lease.
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/memory/repeated-fts").unwrap();
    let content = IndexKey::new("content").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("by_content_repeated").unwrap(),
        vec![content],
        IndexKind::Fts,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();

    let writer = filesystem.begin(&prefix).await.unwrap();
    let redeclaration = tokio::time::timeout(
        Duration::from_secs(1),
        filesystem.ensure_index(&prefix, &spec),
    )
    .await;
    writer.rollback().await;

    assert!(
        matches!(redeclaration, Ok(Ok(()))),
        "an existing FTS declaration must not require the writer: {redeclaration:?}"
    );
}

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
#[tokio::test]
async fn libsql_ordered_query_uses_composite_index_and_keyset_cursor() {
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/threads/index").unwrap();
    let activity = IndexKey::new("activity").unwrap();
    let thread_id = IndexKey::new("thread_id").unwrap();
    filesystem
        .ensure_index(
            &prefix,
            &IndexSpec::new(
                IndexName::new("thread_activity").unwrap(),
                vec![activity.clone(), thread_id.clone()],
                IndexKind::Exact,
            ),
        )
        .await
        .unwrap();
    let kind = RecordKind::new("thread_index").unwrap();
    for (id, rank) in [("b", "001"), ("a", "001"), ("c", "002")] {
        let entry = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(activity.clone(), IndexValue::Text(rank.into()))
            .with_indexed(thread_id.clone(), IndexValue::Text(id.into()));
        filesystem
            .put(
                &VirtualPath::new(format!("/threads/index/{id}")).unwrap(),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    let first = filesystem
        .query_ordered(
            &prefix,
            &Filter::All,
            &ironclaw_filesystem::OrderedPage::new(
                IndexName::new("thread_activity").unwrap(),
                activity.clone(),
                thread_id.clone(),
                SortDirection::Ascending,
                2,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|row| row.entry.indexed[&thread_id].clone())
            .collect::<Vec<_>>(),
        vec![IndexValue::Text("a".into()), IndexValue::Text("b".into())]
    );
    let second = filesystem
        .query_ordered(
            &prefix,
            &Filter::All,
            &ironclaw_filesystem::OrderedPage::new(
                IndexName::new("thread_activity").unwrap(),
                activity.clone(),
                thread_id.clone(),
                SortDirection::Ascending,
                2,
            )
            .after(ironclaw_filesystem::OrderedQueryCursor {
                value: IndexValue::Text("001".into()),
                tie_breaker: IndexValue::Text("b".into()),
            }),
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(
        second[0].entry.indexed[&thread_id],
        IndexValue::Text("c".into())
    );

    let descending = filesystem
        .query_ordered(
            &prefix,
            &Filter::All,
            &ironclaw_filesystem::OrderedPage::new(
                IndexName::new("thread_activity").unwrap(),
                activity,
                thread_id.clone(),
                SortDirection::Descending,
                2,
            )
            .after(ironclaw_filesystem::OrderedQueryCursor {
                value: IndexValue::Text("001".into()),
                tie_breaker: IndexValue::Text("b".into()),
            }),
        )
        .await
        .unwrap();
    assert_eq!(descending.len(), 1);
    assert_eq!(
        descending[0].entry.indexed[&thread_id],
        IndexValue::Text("a".into())
    );
}

#[tokio::test]
async fn libsql_root_ordered_query_includes_normal_descendants() {
    let filesystem = libsql_root().await;
    // `VirtualPath` intentionally does not expose the bare virtual root, so use
    // a declared top-level root to exercise the same descendant-pattern edge.
    let root = VirtualPath::new("/engine").unwrap();
    let rank = IndexKey::new("rank").unwrap();
    let item_id = IndexKey::new("item_id").unwrap();
    filesystem
        .ensure_index(
            &root,
            &IndexSpec::new(
                IndexName::new("root_items").unwrap(),
                vec![rank.clone(), item_id.clone()],
                IndexKind::Exact,
            ),
        )
        .await
        .unwrap();
    filesystem
        .put(
            &VirtualPath::new("/engine/root-query/item-a").unwrap(),
            Entry::record(
                RecordKind::new("root_item").unwrap(),
                &serde_json::json!({}),
            )
            .unwrap()
            .with_indexed(rank.clone(), IndexValue::Text("001".into()))
            .with_indexed(item_id.clone(), IndexValue::Text("item-a".into())),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    let rows = filesystem
        .query_ordered(
            &root,
            &Filter::All,
            &ironclaw_filesystem::OrderedPage::new(
                IndexName::new("root_items").unwrap(),
                rank,
                item_id,
                SortDirection::Ascending,
                10,
            ),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path.as_str(), "/engine/root-query/item-a");
}

#[tokio::test]
async fn libsql_scopes_ordered_index_names_by_prefix() {
    let filesystem = libsql_root().await;
    filesystem
        .ensure_index(
            &VirtualPath::new("/engine/prefix-a").unwrap(),
            &IndexSpec::new(
                IndexName::new("shared_name").unwrap(),
                vec![
                    IndexKey::new("rank_a").unwrap(),
                    IndexKey::new("id_a").unwrap(),
                ],
                IndexKind::Exact,
            ),
        )
        .await
        .unwrap();
    filesystem
        .ensure_index(
            &VirtualPath::new("/engine/prefix-b").unwrap(),
            &IndexSpec::new(
                IndexName::new("shared_name").unwrap(),
                vec![
                    IndexKey::new("rank_b").unwrap(),
                    IndexKey::new("id_b").unwrap(),
                ],
                IndexKind::Exact,
            ),
        )
        .await
        .expect("same index name with a different prefix is independent");
}

/// The projection's UPDATE and DELETE branches were replaced wholesale
/// with the static trigger set, but the ordered-index contracts only insert and
/// query. A stale row left behind after an indexed key changes, or after the
/// source row is deleted, would go unnoticed.
async fn static_projection_update_and_delete_contract<F: RootFilesystem>(
    filesystem: &F,
    base: &str,
) {
    let rank = IndexKey::new("rank").unwrap();
    let item_id = IndexKey::new("item_id").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("mutating_items_v1").unwrap(),
        vec![rank.clone(), item_id.clone()],
        IndexKind::Exact,
    );
    filesystem
        .ensure_index(&VirtualPath::new(base.to_string()).unwrap(), &spec)
        .await
        .unwrap();
    let path = VirtualPath::new(format!("{base}/item")).unwrap();
    let write = async |rank_value: &str, expectation: CasExpectation| {
        filesystem
            .put(
                &path,
                Entry::record(
                    RecordKind::new("mutating_item").unwrap(),
                    &serde_json::json!({}),
                )
                .unwrap()
                .with_indexed(rank.clone(), IndexValue::Text(rank_value.to_string()))
                .with_indexed(item_id.clone(), IndexValue::Text("item".to_string())),
                expectation,
            )
            .await
            .unwrap()
    };
    let query = async || {
        filesystem
            .query_ordered(
                &VirtualPath::new(base.to_string()).unwrap(),
                &Filter::All,
                &OrderedPage::new(
                    IndexName::new("mutating_items_v1").unwrap(),
                    rank.clone(),
                    item_id.clone(),
                    SortDirection::Ascending,
                    16,
                ),
            )
            .await
            .unwrap()
            .len()
    };

    let version = write("a", CasExpectation::Absent).await;
    assert_eq!(query().await, 1, "the insert projects one row");

    // Replacing the indexed key must leave exactly one row, not two.
    write("b", CasExpectation::Version(version)).await;
    assert_eq!(
        query().await,
        1,
        "an indexed-key update must replace the projection row, not add one"
    );

    filesystem.delete(&path).await.unwrap();
    assert_eq!(
        query().await,
        0,
        "deleting the source row must remove its projection row"
    );
}

#[tokio::test]
async fn libsql_static_projection_update_and_delete_remove_stale_rows() {
    let filesystem = libsql_root().await;
    static_projection_update_and_delete_contract(&*filesystem, "/engine/mutating").await;
}

#[tokio::test]
async fn in_memory_static_projection_update_and_delete_remove_stale_rows() {
    let filesystem = InMemoryBackend::new();
    static_projection_update_and_delete_contract(&filesystem, "/engine/mutating").await;
}

#[tokio::test]
async fn libsql_ordered_index_declaration_never_backfills_existing_rows() {
    let filesystem = libsql_root().await;
    let prefix = VirtualPath::new("/processes/materialized/process").unwrap();
    let status = IndexKey::new("status").unwrap();
    let process_id = IndexKey::new("process_id").unwrap();
    let kind = RecordKind::new("process").unwrap();
    let old = Entry::record(kind.clone(), &serde_json::json!({}))
        .unwrap()
        .with_indexed(status.clone(), IndexValue::Text("queued".into()))
        .with_indexed(process_id.clone(), IndexValue::Text("old".into()));
    filesystem
        .put(
            &VirtualPath::new("/processes/materialized/process/old").unwrap(),
            old,
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    let spec = IndexSpec::new(
        IndexName::new("process_queue_declaration_contract").unwrap(),
        vec![status.clone(), process_id.clone()],
        IndexKind::Exact,
    );
    filesystem.ensure_index(&prefix, &spec).await.unwrap();
    let page = ironclaw_filesystem::OrderedPage::new(
        spec.name.clone(),
        status.clone(),
        process_id.clone(),
        SortDirection::Ascending,
        10,
    );
    assert!(
        filesystem
            .query_ordered(&prefix, &Filter::All, &page)
            .await
            .unwrap()
            .is_empty(),
        "declaration must not hide a request-time table scan as automatic backfill"
    );

    let new = Entry::record(kind, &serde_json::json!({}))
        .unwrap()
        .with_indexed(status, IndexValue::Text("queued".into()))
        .with_indexed(process_id.clone(), IndexValue::Text("new".into()));
    filesystem
        .put(
            &VirtualPath::new("/processes/materialized/process/new").unwrap(),
            new,
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    let rows = filesystem
        .query_ordered(&prefix, &Filter::All, &page)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].entry.indexed[&process_id],
        IndexValue::Text("new".into())
    );
}

/// Shared cross-backend body for `Filter::FtsRanked` (#7185).
///
/// The scenario is conversational recall: a fact is stored as one sentence and
/// asked for in differently-worded question that shares only SOME of its
/// content words. `Filter::Fts` requires every content term, so it finds
/// nothing — the assertion this test opens with, which is what made memory
/// recall fail in practice. `Filter::FtsRanked` matches on any term and orders
/// by relevance, so the sentence sharing three query terms comes back ahead of
/// the one sharing a single term, and the unrelated record stays out.
///
/// The AND-semantics assertion is load-bearing: without it a ranked-OR test
/// would also pass under the old behavior and prove nothing.
async fn ranked_fts_contract<F: RootFilesystem>(filesystem: &F, base: &str) {
    let content = IndexKey::new("content").unwrap();
    let kind = RecordKind::new("chunk").unwrap();
    let root = VirtualPath::new(base.to_string()).unwrap();
    let spec = IndexSpec::new(
        IndexName::new("ranked_fts_content_v1").unwrap(),
        vec![content.clone()],
        IndexKind::Fts,
    );
    filesystem.ensure_index(&root, &spec).await.unwrap();

    for (leaf, body) in [
        (
            "a",
            "Sarah prefers the standup meeting scheduled early on Thursday mornings",
        ),
        ("b", "Sarah keeps a spare umbrella at her desk"),
        ("c", "Deployment runbook for the staging cluster"),
    ] {
        filesystem
            .put(
                &VirtualPath::new(format!("{base}/{leaf}")).unwrap(),
                Entry::record(kind.clone(), &serde_json::json!({}))
                    .unwrap()
                    .with_indexed(content.clone(), IndexValue::Text(body.into())),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    // Content terms: sarah, like, standup, scheduled. Record `a` carries three
    // of them but not `like`, so every-term matching returns nothing.
    let question = "when does Sarah like her standup scheduled";

    let and_results = filesystem
        .query(
            &root,
            &Filter::Fts {
                key: content.clone(),
                query: question.into(),
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert!(
        and_results.is_empty(),
        "Filter::Fts requires every content term, so a paraphrased question must miss; got {:?}",
        and_results
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>()
    );

    let ranked = filesystem
        .query(
            &root,
            &Filter::FtsRanked {
                key: content.clone(),
                query: question.into(),
                limit: 10,
            },
            Page::default(),
        )
        .await
        .unwrap();
    let ranked_paths: Vec<&str> = ranked.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        ranked_paths,
        vec![format!("{base}/a").as_str(), format!("{base}/b").as_str()],
        "ranked OR must return both partial matches, most relevant first, and exclude the \
         unrelated record"
    );

    // `limit` truncates after ranking, so the top-1 request keeps the best hit.
    let top_one = filesystem
        .query(
            &root,
            &Filter::FtsRanked {
                key: content.clone(),
                query: question.into(),
                limit: 1,
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        top_one.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        vec![format!("{base}/a").as_str()]
    );

    // A query with no content terms matches nothing rather than everything.
    let stop_words_only = filesystem
        .query(
            &root,
            &Filter::FtsRanked {
                key: content.clone(),
                query: "and the of to".into(),
                limit: 10,
            },
            Page::default(),
        )
        .await
        .unwrap();
    assert!(stop_words_only.is_empty());

    // ...and it stays "nothing" on a prefix where no FTS index was ever
    // declared. Every case above declares the index first, so this is the one
    // combination that can expose an ordering difference INSIDE a backend:
    // libSQL resolves the FTS table before reading the query, PostgreSQL reads
    // the query and never consults an index. The answer to "does a query with
    // no content terms match anything" does not depend on an index, so it must
    // not depend on which backend is bound.
    let undeclared = VirtualPath::new(format!("{base}-undeclared")).unwrap();
    let stop_words_only_undeclared = filesystem
        .query(
            &undeclared,
            &Filter::FtsRanked {
                key: content.clone(),
                query: "and the of to".into(),
                limit: 10,
            },
            Page::default(),
        )
        .await;
    assert!(
        matches!(&stop_words_only_undeclared, Ok(entries) if entries.is_empty()),
        "a content-term-free ranked query must be an empty result on every backend even where no \
         FTS index is declared, got {stop_words_only_undeclared:?}"
    );

    // Nesting a ranking filter inside a compound would discard the ordering.
    let nested = filesystem
        .query(
            &root,
            &Filter::And(vec![Filter::FtsRanked {
                key: content,
                query: question.into(),
                limit: 10,
            }]),
            Page::default(),
        )
        .await;
    assert!(
        matches!(nested, Err(FilesystemError::Unsupported { .. })),
        "nested FtsRanked must be Unsupported on every backend, got {nested:?}"
    );
}

#[tokio::test]
async fn libsql_ranked_fts_finds_paraphrased_recall_in_relevance_order() {
    let filesystem = libsql_root().await;
    ranked_fts_contract(&*filesystem, "/memory/ranked-fts").await;
}

#[tokio::test]
async fn in_memory_ranked_fts_finds_paraphrased_recall_in_relevance_order() {
    let filesystem = ironclaw_filesystem::InMemoryBackend::new();
    ranked_fts_contract(&filesystem, "/memory/ranked-fts").await;
}

/// Shared cross-backend body: an ordered-index spec declared on an ancestor
/// prefix serves queries at child paths, and those queries stay scoped to the
/// queried child subtree.
///
/// Projection rows are keyed by spec name and path only, with no record of the
/// prefix that declared them, so a backend resolving an ancestor spec must
/// still constrain results to the queried subtree. Without that constraint a
/// child query returns a sibling's rows — a scope leak, not just a parity
/// difference.
async fn ancestor_declared_ordered_index_contract<F: RootFilesystem>(filesystem: &F, base: &str) {
    let rank = IndexKey::new("rank").unwrap();
    let item_id = IndexKey::new("item_id").unwrap();
    let spec = IndexSpec::new(
        IndexName::new("ancestor_declared_items_v1").unwrap(),
        vec![rank.clone(), item_id.clone()],
        IndexKind::Exact,
    );
    let root = VirtualPath::new(base.to_string()).unwrap();
    filesystem.ensure_index(&root, &spec).await.unwrap();

    for (child, leaf) in [("alpha", "a-1"), ("alpha", "a-2"), ("beta", "b-1")] {
        filesystem
            .put(
                &VirtualPath::new(format!("{base}/{child}/{leaf}")).unwrap(),
                Entry::record(
                    RecordKind::new("ancestor_item").unwrap(),
                    &serde_json::json!({}),
                )
                .unwrap()
                .with_indexed(rank.clone(), IndexValue::Text(leaf.to_string()))
                .with_indexed(item_id.clone(), IndexValue::Text(leaf.to_string())),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    let page = || {
        ironclaw_filesystem::OrderedPage::new(
            spec.name.clone(),
            rank.clone(),
            item_id.clone(),
            SortDirection::Ascending,
            10,
        )
    };
    let paths_at = async |prefix: String| -> Vec<String> {
        filesystem
            .query_ordered(&VirtualPath::new(prefix).unwrap(), &Filter::All, &page())
            .await
            .unwrap()
            .iter()
            .map(|row| row.path.as_str().to_string())
            .collect()
    };

    // Declared on the ancestor, queried on a child: the write path projected
    // into the ancestor declaration, and resolution found it from below.
    assert_eq!(
        paths_at(format!("{base}/alpha")).await,
        vec![format!("{base}/alpha/a-1"), format!("{base}/alpha/a-2")],
        "a child query must see the rows in its own subtree"
    );
    assert_eq!(
        paths_at(format!("{base}/beta")).await,
        vec![format!("{base}/beta/b-1")],
        "a sibling subtree must not leak into a child query resolved from an ancestor spec"
    );
    // The declaring prefix itself still spans the whole subtree.
    assert_eq!(paths_at(base.to_string()).await.len(), 3);
}

/// Shared cross-backend body for the existing-deployment path: a narrower spec
/// declared before the root one keeps serving its own subtree once the root
/// declaration is added, and a subtree that never had its own declaration is
/// served by the root spec.
///
/// Declarations never backfill, so resolution must prefer the more specific
/// spec — its projection is the one already holding rows written before the
/// root declaration existed.
async fn narrow_then_root_ordered_index_contract<F: RootFilesystem>(filesystem: &F, base: &str) {
    let rank = IndexKey::new("rank").unwrap();
    let item_id = IndexKey::new("item_id").unwrap();
    let spec = || {
        IndexSpec::new(
            IndexName::new("migrating_items_v1").unwrap(),
            vec![rank.clone(), item_id.clone()],
            IndexKind::Exact,
        )
    };
    let write = async |path: String, leaf: &str| {
        filesystem
            .put(
                &VirtualPath::new(path).unwrap(),
                Entry::record(
                    RecordKind::new("migrating_item").unwrap(),
                    &serde_json::json!({}),
                )
                .unwrap()
                .with_indexed(rank.clone(), IndexValue::Text(leaf.to_string()))
                .with_indexed(item_id.clone(), IndexValue::Text(leaf.to_string())),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    };

    // The deployment as it stands before the fix: a per-thread declaration,
    // and a row written under it.
    filesystem
        .ensure_index(
            &VirtualPath::new(format!("{base}/threads/t-1")).unwrap(),
            &spec(),
        )
        .await
        .unwrap();
    write(format!("{base}/threads/t-1/m-1"), "m-1").await;

    // The upgrade: the root declaration lands while the narrow one remains.
    filesystem
        .ensure_index(&VirtualPath::new(base.to_string()).unwrap(), &spec())
        .await
        .unwrap();
    write(format!("{base}/threads/t-1/m-2"), "m-2").await;
    // A thread created after the upgrade never gets its own declaration.
    write(format!("{base}/threads/t-2/m-1"), "m-1").await;

    let paths_at = async |prefix: String| -> Vec<String> {
        filesystem
            .query_ordered(
                &VirtualPath::new(prefix).unwrap(),
                &Filter::All,
                &ironclaw_filesystem::OrderedPage::new(
                    IndexName::new("migrating_items_v1").unwrap(),
                    rank.clone(),
                    item_id.clone(),
                    SortDirection::Ascending,
                    10,
                ),
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.path.as_str().to_string())
            .collect()
    };

    assert_eq!(
        paths_at(format!("{base}/threads/t-1")).await,
        vec![
            format!("{base}/threads/t-1/m-1"),
            format!("{base}/threads/t-1/m-2"),
        ],
        "rows written before the root declaration must stay queryable after it"
    );
    assert_eq!(
        paths_at(format!("{base}/threads/t-2")).await,
        vec![format!("{base}/threads/t-2/m-1")],
        "a subtree with no declaration of its own is served by the root spec"
    );
}

#[tokio::test]
async fn libsql_ordered_index_declared_at_ancestor_serves_scoped_descendant_queries() {
    let filesystem = libsql_root().await;
    ancestor_declared_ordered_index_contract(&*filesystem, "/engine/ancestor-declared").await;
}

#[tokio::test]
async fn libsql_ordered_index_narrow_declaration_survives_root_declaration() {
    let filesystem = libsql_root().await;
    narrow_then_root_ordered_index_contract(&*filesystem, "/engine/narrow-then-root").await;
}

#[tokio::test]
async fn in_memory_ordered_index_declared_at_ancestor_serves_scoped_descendant_queries() {
    let filesystem = ironclaw_filesystem::InMemoryBackend::new();
    ancestor_declared_ordered_index_contract(&filesystem, "/engine/ancestor-declared").await;
}

#[tokio::test]
async fn in_memory_ordered_index_narrow_declaration_survives_root_declaration() {
    let filesystem = ironclaw_filesystem::InMemoryBackend::new();
    narrow_then_root_ordered_index_contract(&filesystem, "/engine/narrow-then-root").await;
}

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
#[tokio::test]
async fn libsql_append_batch_is_one_statement_with_contiguous_ordered_seqs() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/engine").unwrap();

    // Seed a single append so the batch must continue the sequence.
    let s0 = filesystem.append(&log, b"seed".to_vec()).await.unwrap();

    let payloads: Vec<Vec<u8>> = (0..21u8).map(|n| vec![n]).collect();
    let seqs = filesystem
        .append_batch(&log, payloads.clone())
        .await
        .unwrap();
    assert_eq!(seqs.len(), 21);
    assert!(seqs[0] > s0, "batch seqs continue past the seeded append");
    // Contiguous + monotonic in payload order.
    for window in seqs.windows(2) {
        assert!(window[0] < window[1]);
    }

    // Order + content preserved through the single multi-row INSERT.
    let all = filesystem.tail(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(all.len(), 22);
    assert_eq!(all[0].payload, b"seed".to_vec());
    for (offset, payload) in payloads.iter().enumerate() {
        assert_eq!(&all[offset + 1].payload, payload);
        assert_eq!(all[offset + 1].seq, seqs[offset]);
    }

    // Empty batch is a no-op.
    assert!(
        filesystem
            .append_batch(&log, Vec::new())
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn libsql_append_batch_spanning_multiple_statements_commits_atomically_in_order() {
    // 600 > the 256-row chunk size, so this exercises the multi-statement
    // transactional path: every chunk must commit and the seqs stay contiguous
    // and ordered across chunk boundaries.
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/multichunk").unwrap();

    let payloads: Vec<Vec<u8>> = (0..600u32).map(|n| n.to_le_bytes().to_vec()).collect();
    let seqs = filesystem
        .append_batch(&log, payloads.clone())
        .await
        .unwrap();
    assert_eq!(seqs.len(), 600);
    for window in seqs.windows(2) {
        assert!(window[0] < window[1], "seqs are ordered across chunks");
    }

    let all = filesystem.tail(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(all.len(), 600);
    for (offset, payload) in payloads.iter().enumerate() {
        assert_eq!(
            &all[offset].payload, payload,
            "order preserved across chunks"
        );
        assert_eq!(all[offset].seq, seqs[offset]);
    }
}
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
#[tokio::test]
async fn libsql_head_seq_returns_none_for_empty_path() {
    let filesystem = libsql_root().await;
    let log = VirtualPath::new("/events/empty-head").unwrap();
    let head = filesystem.head_seq(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(head, None);
}
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
#[tokio::test]
async fn libsql_capabilities_advertise_events() {
    let filesystem = libsql_root().await;
    assert!(filesystem.capabilities().has(Capability::Events));
}
#[tokio::test]
async fn libsql_create_dir_all_concurrent_shared_prefixes_waits_for_writer() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem =
        std::sync::Arc::new(LibSqlRootFilesystem::new(db).expect("filesystem runtime"));
    filesystem.run_migrations().await.unwrap();

    let mut tasks = Vec::new();
    for sample in 0..32 {
        let filesystem = std::sync::Arc::clone(&filesystem);
        tasks.push(tokio::spawn(async move {
            let path = VirtualPath::new(format!(
                "/engine/tenants/latency/users/libsql/runs/shared-c8/sample-{sample}/d0/d1/d2"
            ))
            .unwrap();
            filesystem.create_dir_all(&path).await
        }));
    }

    for task in tasks {
        task.await.unwrap().unwrap();
    }

    let shared_prefix =
        VirtualPath::new("/engine/tenants/latency/users/libsql/runs/shared-c8").unwrap();
    assert_eq!(
        filesystem.stat(&shared_prefix).await.unwrap().file_type,
        FileType::Directory
    );
}
#[tokio::test]
async fn libsql_put_concurrent_distinct_children_waits_for_writer() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem =
        std::sync::Arc::new(LibSqlRootFilesystem::new(db).expect("filesystem runtime"));
    filesystem.run_migrations().await.unwrap();
    let parent = VirtualPath::new("/engine/tenants/latency/users/libsql/runs/shared-put").unwrap();
    filesystem.create_dir_all(&parent).await.unwrap();

    let mut tasks = Vec::new();
    for sample in 0..32 {
        let filesystem = std::sync::Arc::clone(&filesystem);
        tasks.push(tokio::spawn(async move {
            let path = VirtualPath::new(format!(
                "/engine/tenants/latency/users/libsql/runs/shared-put/record-{sample}"
            ))
            .unwrap();
            filesystem
                .put(&path, Entry::bytes(vec![sample as u8]), CasExpectation::Any)
                .await
        }));
    }

    for task in tasks {
        task.await.unwrap().unwrap();
    }

    let last =
        VirtualPath::new("/engine/tenants/latency/users/libsql/runs/shared-put/record-31").unwrap();
    assert_eq!(
        filesystem.get(&last).await.unwrap().unwrap().entry.body,
        vec![31]
    );
}
async fn libsql_root() -> TestLibSqlRootFilesystem {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = LibSqlRootFilesystem::new(db).expect("filesystem runtime");
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
mod postgres_tests {
    use super::*;
    use ironclaw_filesystem::{
        AtomicSubtreeEntry, Capability, CasExpectation, Entry, FileType, FilesystemError,
        FilesystemOperation, Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page,
        PostgresRootFilesystem, RecordKind, SeqNo, TxnCapability,
    };
    use ironclaw_host_api::path::VirtualPath;

    /// One container per test binary, started on first use.
    ///
    /// Kept alive for the process: dropping the handle stops the database out
    /// from under every later test. Each test still namespaces by a unique
    /// path prefix, so sharing one instance is safe.
    static POSTGRES_CONTAINER: tokio::sync::OnceCell<Option<ContainerUrl>> =
        tokio::sync::OnceCell::const_new();

    struct ContainerUrl {
        url: String,
        _container: testcontainers_modules::testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres,
        >,
    }

    /// The database these tests run against.
    ///
    /// An explicit URL wins, so a local run can point at an existing server.
    /// Otherwise a container is provisioned, which is what lets these tests
    /// actually run in CI lanes that have Docker but set no database URL —
    /// previously every case here skipped there, leaving the PostgreSQL
    /// projection code untested and uncovered.
    async fn postgres_url() -> Option<String> {
        if let Ok(url) = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
        {
            return Some(url);
        }
        POSTGRES_CONTAINER
            .get_or_init(|| async {
                use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};
                let image = testcontainers_modules::postgres::Postgres::default()
                    .with_db_name("ironclaw_test")
                    .with_user("postgres")
                    .with_password("postgres")
                    .with_tag("16-alpine");
                let container = match image.start().await {
                    Ok(container) => container,
                    Err(error) => {
                        eprintln!(
                            "skipping Postgres filesystem contract tests: \
                             docker/testcontainers unavailable ({error})"
                        );
                        return None;
                    }
                };
                let host = container.get_host().await.ok()?;
                let port = container.get_host_port_ipv4(5432).await.ok()?;
                Some(ContainerUrl {
                    url: format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test"),
                    _container: container,
                })
            })
            .await
            .as_ref()
            .map(|container| container.url.clone())
    }

    async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
        if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
            return None;
        }
        let url = postgres_url().await?;
        let config = url.parse::<tokio_postgres::Config>().ok()?;
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .ok()
    }

    /// Installs the projection triggers once per binary, before tests race.
    ///
    /// Declaring an ordered index installs those triggers, and that takes an
    /// ACCESS EXCLUSIVE lock on `root_filesystem_entries`. Against a *fresh*
    /// database every test would otherwise race the install while its
    /// neighbours write the same table, and a writer already holding a row lock
    /// there while it waits on the installer's advisory lock is a deadlock —
    /// PostgreSQL breaks it by killing one side, which surfaces as `BackendBusy`
    /// in whichever test lost rather than in the one doing the DDL. Running it
    /// once leaves every later declaration on the no-DDL fast path. A warm
    /// database hides this completely, so it reproduces only against a fresh
    /// one, which is what CI provisions and a repeat local run does not.
    ///
    /// The work borrows the first caller's filesystem instead of caching a pool:
    /// every `#[tokio::test]` has its own runtime, and `tokio_postgres` drives
    /// each connection from a task on the runtime that created it, so a pool
    /// shared across tests dies as `connection closed` the moment its origin
    /// test finishes.
    static PROJECTION_INSTALLED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    async fn install_projection_once(filesystem: &PostgresRootFilesystem) {
        PROJECTION_INSTALLED
            .get_or_init(|| async {
                // A unique prefix that is an ancestor of nothing any test
                // writes, so the declaration projects no rows for them.
                //
                // Every step panics rather than returning: reaching here means
                // a database was resolved, so a failure is a broken setup, not
                // an unconfigured one. Completing the `OnceCell` quietly with no
                // triggers installed would let the whole PostgreSQL projection
                // suite pass while testing nothing -- and the changed-coverage
                // exemptions for those residual lines are justified by these
                // tests running.
                let path = VirtualPath::new(format!(
                    "/secrets/leases/pgwarmup_{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .expect("warm-up prefix is a valid virtual path");
                let name = IndexName::new("warmup_probe").expect("warm-up index name");
                let key = IndexKey::new("rank").expect("warm-up index key");
                filesystem
                    .ensure_index(&path, &IndexSpec::new(name, vec![key], IndexKind::Exact))
                    .await
                    .expect("warm-up declaration installs the static projection triggers");
            })
            .await;
    }

    /// A private database for a test that mutates or asserts global schema.
    ///
    /// The projection triggers live on `root_filesystem_entries`, so installing
    /// or sweeping them takes an ACCESS EXCLUSIVE lock on that whole table, and
    /// the `pg_trigger` assertions read state every other test shares. Sharing
    /// one database, such a test blocks unrelated concurrent writers — which
    /// surfaces as a `BackendBusy` failure in whichever test happens to be
    /// mid-write, far from the cause — and races its own assertions against
    /// their declarations. Path prefixes isolate rows; they cannot isolate
    /// schema, so a schema-level test gets a schema-level scope.
    ///
    /// This is the ancestor of the crate's shared `test-support` provisioner
    /// (`src/postgres_isolation.rs`), which the event-store and
    /// product-workflow-ledger suites use. This suite deliberately keeps its
    /// own older variant: its URL resolution goes through `postgres_url()`
    /// (container startup + the `IRONCLAW_SKIP_POSTGRES_TESTS` opt-out), its
    /// names are uuid-based rather than epoch-based, and its sweep runs per
    /// provisioning call without the shared seam's age gate — migrating it is
    /// a behavior change to this suite's scaffolding, not a de-duplication,
    /// and is left as a candidate follow-up.
    struct IsolatedDatabase {
        filesystem: PostgresRootFilesystem,
        prefix: String,
        client: tokio_postgres::Client,
        admin: tokio_postgres::Client,
        name: String,
    }

    impl IsolatedDatabase {
        /// Drop the database on the way out of a passing test.
        ///
        /// This is the tidy path, not a guarantee: a failing assertion unwinds
        /// straight past it, so a red run can leave its database behind. That
        /// is what the sweep in `postgres_isolated_root` collects, rather than
        /// wrapping the test body in `catch_unwind` — the cleanup is a courtesy
        /// to whoever points these tests at their own server, and it is not
        /// worth contorting the test to make it absolute.
        ///
        /// `FORCE` closes the pool's connections, which are not guaranteed to
        /// have gone away by the time the handles drop.
        async fn cleanup(self) {
            let Self {
                filesystem,
                client,
                admin,
                name,
                ..
            } = self;
            drop(filesystem);
            drop(client);
            let _ = admin
                .execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), &[])
                .await;
        }
    }

    async fn postgres_isolated_root() -> Option<IsolatedDatabase> {
        // Same opt-out every other PostgreSQL case honours through
        // `postgres_pool`, checked before anything provisions a container or
        // issues DDL.
        if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
            return None;
        }
        // Past the skip flag and a resolvable URL, every failure below is a
        // broken environment rather than an unconfigured one, so it panics.
        // Returning `None` instead would make the legacy-trigger sweep -- the
        // only coverage for that path, and the basis for waiving its residual
        // lines in the changed-coverage exemptions -- report success while
        // never running. A role that cannot CREATE DATABASE would silence the
        // test and the gate together.
        let config = postgres_url()
            .await?
            .parse::<tokio_postgres::Config>()
            .expect("resolved postgres url parses as a connection config");
        let (admin, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect to the resolved postgres server");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        // Collect databases a previously failed run unwound past. No `FORCE`:
        // a database another run still holds open refuses to drop, which is
        // exactly the outcome we want when two binaries share a server.
        if let Ok(stale) = admin
            .query(
                "SELECT datname FROM pg_database WHERE datname LIKE 'rfs_isolated_%'",
                &[],
            )
            .await
        {
            for row in stale {
                let name = row.get::<_, String>(0);
                let _ = admin
                    .execute(&format!("DROP DATABASE IF EXISTS {name}"), &[])
                    .await;
            }
        }

        // Identifiers cannot be bind parameters in DDL. `uuid::simple` is hex
        // and the swept names come from `pg_database`, so both interpolations
        // are server-supplied or generated, never caller input.
        let name = format!("rfs_isolated_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute(&format!("CREATE DATABASE {name}"), &[])
            .await
            .expect("create the isolated database (the role needs CREATEDB)");

        let mut isolated = config.clone();
        isolated.dbname(&name);
        let manager = deadpool_postgres::Manager::new(isolated.clone(), tokio_postgres::NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .expect("build a pool against the isolated database");
        let filesystem = PostgresRootFilesystem::new(pool);
        filesystem
            .run_migrations()
            .await
            .expect("migrate the isolated database");
        let (client, connection) = isolated
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect to the isolated database");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        Some(IsolatedDatabase {
            filesystem,
            prefix: format!("/secrets/leases/pgtest_{}", uuid::Uuid::new_v4().simple()),
            client,
            admin,
            name,
        })
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
        install_projection_once(&fs).await;
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
    async fn postgres_ordered_index_declared_at_ancestor_serves_scoped_descendant_queries() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        super::ancestor_declared_ordered_index_contract(&fs, &prefix).await;
    }

    #[tokio::test]
    async fn postgres_ranked_fts_finds_paraphrased_recall_in_relevance_order() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        super::ranked_fts_contract(&fs, &prefix).await;
    }

    #[tokio::test]
    async fn postgres_ordered_index_narrow_declaration_survives_root_declaration() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        super::narrow_then_root_ordered_index_contract(&fs, &prefix).await;
    }

    #[tokio::test]
    async fn postgres_create_subtree_atomic_publishes_the_complete_batch() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let batch_prefix = vpath(&prefix, "attachments/message-1");
        let first = vpath(&prefix, "attachments/message-1/0-alpha.txt");
        let second = vpath(&prefix, "attachments/message-1/1-beta.txt");

        let versions = fs
            .create_subtree_atomic(
                &batch_prefix,
                vec![
                    AtomicSubtreeEntry {
                        path: first.clone(),
                        entry: Entry::bytes(b"alpha".to_vec()),
                    },
                    AtomicSubtreeEntry {
                        path: second.clone(),
                        entry: Entry::bytes(b"beta".to_vec()),
                    },
                ],
            )
            .await
            .unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(fs.read_file(&first).await.unwrap(), b"alpha");
        assert_eq!(fs.read_file(&second).await.unwrap(), b"beta");
    }

    #[tokio::test]
    async fn postgres_create_subtree_atomic_rejects_conflicts_without_overwrite() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let batch_prefix = vpath(&prefix, "attachments/message-conflict");
        let file = vpath(&prefix, "attachments/message-conflict/0.txt");
        fs.create_subtree_atomic(
            &batch_prefix,
            vec![AtomicSubtreeEntry {
                path: file.clone(),
                entry: Entry::bytes(b"original".to_vec()),
            }],
        )
        .await
        .unwrap();

        let error = fs
            .create_subtree_atomic(
                &batch_prefix,
                vec![AtomicSubtreeEntry {
                    path: file.clone(),
                    entry: Entry::bytes(b"replacement".to_vec()),
                }],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, FilesystemError::VersionMismatch { .. }));
        assert_eq!(fs.read_file(&file).await.unwrap(), b"original");
    }

    #[tokio::test]
    async fn postgres_create_subtree_atomic_rejects_invalid_batch_without_partial_write() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let batch_prefix = vpath(&prefix, "attachments/message-invalid");
        let valid = vpath(&prefix, "attachments/message-invalid/0.txt");
        let outside = VirtualPath::new(format!("{prefix}-outside/escaped.txt")).unwrap();

        let error = fs
            .create_subtree_atomic(
                &batch_prefix,
                vec![
                    AtomicSubtreeEntry {
                        path: valid.clone(),
                        entry: Entry::bytes(b"valid".to_vec()),
                    },
                    AtomicSubtreeEntry {
                        path: outside,
                        entry: Entry::bytes(b"escaped".to_vec()),
                    },
                ],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, FilesystemError::PathOutsideMount { .. }));
        assert!(fs.get(&valid).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn postgres_append_batch_is_one_statement_with_contiguous_ordered_seqs() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        // Event-plane writes go under a per-test prefix; the events table keys
        // on the full path, so isolation holds against a shared DB.
        let log = VirtualPath::new(format!("{prefix}/events")).unwrap();

        let s0 = fs.append(&log, b"seed".to_vec()).await.unwrap();

        let payloads: Vec<Vec<u8>> = (0..21u8).map(|n| vec![n]).collect();
        let seqs = fs.append_batch(&log, payloads.clone()).await.unwrap();
        assert_eq!(seqs.len(), 21);
        assert!(seqs[0] > s0);
        for window in seqs.windows(2) {
            assert!(window[0] < window[1]);
        }

        let all = fs.tail(&log, SeqNo::ZERO).await.unwrap();
        assert_eq!(all.len(), 22);
        assert_eq!(all[0].payload, b"seed".to_vec());
        for (offset, payload) in payloads.iter().enumerate() {
            assert_eq!(&all[offset + 1].payload, payload);
            assert_eq!(all[offset + 1].seq, seqs[offset]);
        }

        assert!(fs.append_batch(&log, Vec::new()).await.unwrap().is_empty());
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
    async fn postgres_delete_if_version_deletes_current_and_rejects_stale_or_missing() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "cas_delete");

        // Missing path → NotFound (already gone, benign), never VersionMismatch.
        let err = fs
            .delete_if_version(&path, ironclaw_filesystem::RecordVersion::from_backend(1))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::NotFound {
                operation: FilesystemOperation::Delete,
                ..
            }
        ));

        // Simulates the state a concurrent writer would leave behind (this
        // is a sequential script, not a real race — see
        // concurrent_cas_storm.rs for genuine parallel coverage): the v1
        // the deleter read is bumped to v2 before the delete lands → the
        // stale delete loses with the observed version and the entry
        // survives at v2.
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let log_seq = fs.append(&path, b"kept".to_vec()).await.unwrap();
        let v2 = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Version(v1))
            .await
            .unwrap();
        let err = fs.delete_if_version(&path, v1).await.unwrap_err();
        match err {
            FilesystemError::VersionMismatch {
                expected, found, ..
            } => {
                assert_eq!(expected, Some(v1));
                assert_eq!(found, Some(v2));
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
        let got = fs.get(&path).await.unwrap().unwrap();
        assert_eq!(got.version, v2);
        assert_eq!(got.entry.body, vec![2]);

        // Correct version deletes exactly the entry; single-key, so the
        // event log at the same path survives (blind `delete` sweeps it).
        fs.delete_if_version(&path, v2).await.unwrap();
        assert!(fs.get(&path).await.unwrap().is_none());
        let log = fs.tail(&path, SeqNo::ZERO).await.unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].seq, log_seq);
    }

    /// Review fix (PR #5749): mirrors the libsql overflow guard. An
    /// `expected_version` beyond `i64::MAX` must surface
    /// `CorruptRecordVersion` before any DELETE runs, so the entry survives
    /// untouched — never a silently-truncated bind parameter that could
    /// never match.
    #[tokio::test]
    async fn postgres_delete_if_version_rejects_out_of_range_expected_version() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "cas_delete_overflow");
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();

        let err = fs
            .delete_if_version(
                &path,
                ironclaw_filesystem::RecordVersion::from_backend(u64::MAX),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, FilesystemError::CorruptRecordVersion { .. }),
            "expected CorruptRecordVersion, got {err:?}"
        );

        let got = fs.get(&path).await.unwrap().unwrap();
        assert_eq!(got.version, v1);
        assert_eq!(got.entry.body, vec![1]);
    }

    /// Round-B review: mirrors the libSQL/in-memory ABA-hazard pin. Postgres
    /// shares the same "version restarts at 1 after a full delete"
    /// precondition (see `put`'s Absent-insert path); pin it here too so a
    /// future change to Postgres's version-assignment can't silently
    /// invalidate the trait doc's ABA warning for this backend.
    #[tokio::test]
    async fn postgres_delete_if_version_is_vulnerable_to_aba_across_delete_recreate_cycles() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let path = vpath(&prefix, "cas_delete_aba");

        let v1_first = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        fs.delete_if_version(&path, v1_first).await.unwrap();
        assert!(fs.get(&path).await.unwrap().is_none());

        let v1_second = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Absent)
            .await
            .unwrap();
        assert_eq!(
            v1_first, v1_second,
            "version must restart after a full delete, or this ABA hazard doesn't apply"
        );

        // The stale `v1_first` token wrongly authorizes deleting the second
        // incarnation's live data — documented hazard, not a regression.
        fs.delete_if_version(&path, v1_first).await.unwrap();
        assert!(
            fs.get(&path).await.unwrap().is_none(),
            "stale version token wrongly matched and deleted the second incarnation"
        );
    }

    /// Round-C review (PR #5749): mirrors `postgres_put_rejects_existing_directory`
    /// but for `delete_if_version` — no test drove it against an explicit
    /// directory row to confirm the `is_dir = FALSE` scoping (shared with
    /// `DELETE_IF_VERSION_ATOMIC_SQL`'s `locked`/`deleted` CTEs and
    /// `postgres_current_version_with_client`) actually excludes it. A
    /// directory-only path must diagnose as `NotFound`, never match/delete
    /// the directory row.
    #[tokio::test]
    async fn postgres_delete_if_version_excludes_explicit_directory_row() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let dir = vpath(&prefix, "cas_delete_dir");
        fs.create_dir_all(&dir).await.unwrap();

        let err = fs
            .delete_if_version(&dir, ironclaw_filesystem::RecordVersion::from_backend(1))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                FilesystemError::NotFound {
                    operation: FilesystemOperation::Delete,
                    ..
                }
            ),
            "delete_if_version must not match an explicit directory row \
             (is_dir = TRUE), got: {err:?}"
        );
    }

    #[tokio::test]
    async fn postgres_delete_if_version_reports_mismatch_under_concurrent_update_race() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let racer_pool = match postgres_pool().await {
            Some(pool) => pool,
            None => {
                return;
            }
        };
        let path = vpath(&prefix, "cas_delete_update_race");
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();

        let racer_client = racer_pool
            .get()
            .await
            .expect("racer connection must be available on the same reachable Postgres");
        let path_str = path.as_str().to_string();
        let (updated_tx, updated_rx) = tokio::sync::oneshot::channel::<()>();

        let racer = tokio::spawn(async move {
            racer_client.batch_execute("BEGIN").await.unwrap();
            racer_client
                .execute(
                    "UPDATE root_filesystem_entries \
                     SET version = 999, updated_at = NOW() \
                     WHERE path = $1 AND is_dir = FALSE",
                    &[&path_str],
                )
                .await
                .unwrap();
            // The uncommitted UPDATE holds the tuple lock. A concurrent
            // `delete_if_version` must wait, then report the updated row as
            // stale rather than collapsing every waited-on version change to
            // NotFound.
            let _ = updated_tx.send(());
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            racer_client.batch_execute("COMMIT").await.unwrap();
        });

        updated_rx
            .await
            .expect("racer must signal after its UPDATE runs");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let result = fs.delete_if_version(&path, v1).await;

        racer.await.expect("racer task must not panic");

        match result {
            Err(FilesystemError::VersionMismatch {
                expected, found, ..
            }) => {
                assert_eq!(expected, Some(v1));
                assert_eq!(
                    found,
                    Some(ironclaw_filesystem::RecordVersion::from_backend(999))
                );
            }
            other => panic!("expected VersionMismatch against updated row, got: {other:?}"),
        }
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
    async fn postgres_create_dir_all_conflict_rolls_back_inserted_prefixes() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let parent = vpath(&prefix, "mkdir_conflict");
        let blocking_file = vpath(&prefix, "mkdir_conflict/file");
        let child_under_file = vpath(&prefix, "mkdir_conflict/file/child");

        fs.put(
            &blocking_file,
            Entry::bytes(b"already a file".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

        let err = fs
            .create_dir_all(&child_under_file)
            .await
            .expect_err("existing file prefix must reject create_dir_all");
        match err {
            FilesystemError::Backend {
                path,
                operation,
                reason,
            } => {
                assert_eq!(path, blocking_file);
                assert_eq!(operation, FilesystemOperation::CreateDirAll);
                assert!(
                    reason.contains("file exists where directory is required"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("expected create_dir_all Backend error, got: {other:?}"),
        }
        assert_eq!(
            fs.get(&blocking_file).await.unwrap().unwrap().entry.body,
            b"already a file"
        );

        fs.delete(&blocking_file).await.unwrap();
        assert!(
            matches!(
                fs.stat(&parent).await,
                Err(FilesystemError::NotFound {
                    operation: FilesystemOperation::Stat,
                    ..
                })
            ),
            "failed create_dir_all must roll back explicit directory rows inserted before the conflict"
        );
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
    async fn postgres_transaction_reserves_i64_sequence_ranges_atomically() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let sequence_path = vpath(&prefix, "sequence-range");
        let mut transaction = fs.begin(&prefix_path).await.unwrap();

        let first = transaction
            .reserve_sequence_range(&sequence_path, 3)
            .await
            .unwrap();
        let second = transaction
            .reserve_sequence_range(&sequence_path, 2)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(first, SeqNo::from_backend(3));
        assert_eq!(second, SeqNo::from_backend(5));
        assert_eq!(
            fs.reserve_sequence(&sequence_path).await.unwrap(),
            SeqNo::from_backend(6)
        );
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
    async fn postgres_concurrent_ordered_index_declaration_is_idempotent() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let fs = std::sync::Arc::new(fs);
        let prefix = std::sync::Arc::new(VirtualPath::new(prefix).unwrap());
        let spec = std::sync::Arc::new(IndexSpec::new(
            IndexName::new("concurrent_ordered_projection_v1").unwrap(),
            vec![
                IndexKey::new("scope").unwrap(),
                IndexKey::new("sequence").unwrap(),
            ],
            IndexKind::Exact,
        ));
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let fs = std::sync::Arc::clone(&fs);
            let prefix = std::sync::Arc::clone(&prefix);
            let spec = std::sync::Arc::clone(&spec);
            let barrier = std::sync::Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                fs.ensure_index(prefix.as_ref(), spec.as_ref()).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
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
    async fn postgres_ordered_query_uses_keyset_cursor() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let activity = IndexKey::new("activity").unwrap();
        let thread_id = IndexKey::new("thread_id").unwrap();
        fs.ensure_index(
            &prefix_path,
            &IndexSpec::new(
                IndexName::new("thread_activity").unwrap(),
                vec![activity.clone(), thread_id.clone()],
                IndexKind::Exact,
            ),
        )
        .await
        .unwrap();
        let kind = RecordKind::new("thread_index").unwrap();
        for (id, rank) in [("b", "001"), ("a", "001"), ("c", "002")] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(activity.clone(), IndexValue::Text(rank.into()))
                .with_indexed(thread_id.clone(), IndexValue::Text(id.into()));
            fs.put(&vpath(&prefix, id), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let rows = fs
            .query_ordered(
                &prefix_path,
                &Filter::All,
                &ironclaw_filesystem::OrderedPage::new(
                    IndexName::new("thread_activity").unwrap(),
                    activity,
                    thread_id.clone(),
                    SortDirection::Ascending,
                    2,
                )
                .after(ironclaw_filesystem::OrderedQueryCursor {
                    value: IndexValue::Text("001".into()),
                    tie_breaker: IndexValue::Text("b".into()),
                }),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].entry.indexed[&thread_id],
            IndexValue::Text("c".into())
        );

        let descending = fs
            .query_ordered(
                &prefix_path,
                &Filter::All,
                &ironclaw_filesystem::OrderedPage::new(
                    IndexName::new("thread_activity").unwrap(),
                    IndexKey::new("activity").unwrap(),
                    thread_id.clone(),
                    SortDirection::Descending,
                    2,
                )
                .after(ironclaw_filesystem::OrderedQueryCursor {
                    value: IndexValue::Text("001".into()),
                    tie_breaker: IndexValue::Text("b".into()),
                }),
            )
            .await
            .unwrap();
        assert_eq!(descending.len(), 1);
        assert_eq!(
            descending[0].entry.indexed[&thread_id],
            IndexValue::Text("a".into())
        );
    }

    #[tokio::test]
    async fn postgres_ordered_index_projects_rows_under_long_prefixes() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let unique = prefix.rsplit('_').next().unwrap();
        let index_name = IndexName::new(format!("long_prefix_projection_{unique}")).unwrap();
        let status = IndexKey::new("status").unwrap();
        let process_id = IndexKey::new("process_id").unwrap();
        let tenant_prefix = VirtualPath::new(format!(
            "{prefix}/tenants/tenant/agents/agent/projects/project/users/user/turns/materialized/process"
        ))
        .unwrap();
        fs.ensure_index(
            &tenant_prefix,
            &IndexSpec::new(
                index_name.clone(),
                vec![status.clone(), process_id.clone()],
                IndexKind::Exact,
            ),
        )
        .await
        .unwrap();

        let row_path = VirtualPath::new(format!("{}/run-1", tenant_prefix.as_str())).unwrap();
        let row = Entry::record(RecordKind::new("process").unwrap(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(status.clone(), IndexValue::Text("queued".into()))
            .with_indexed(process_id.clone(), IndexValue::Text("run-1".into()));
        fs.put(&row_path, row, CasExpectation::Absent)
            .await
            .unwrap();

        let rows = fs
            .query_ordered(
                &tenant_prefix,
                &Filter::All,
                &ironclaw_filesystem::OrderedPage::new(
                    index_name,
                    status,
                    process_id,
                    SortDirection::Ascending,
                    10,
                ),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, row_path);
    }

    #[tokio::test]
    async fn postgres_scopes_ordered_index_names_by_prefix() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        for (suffix, rank, id) in [
            ("prefix-a", "rank_a", "id_a"),
            ("prefix-b", "rank_b", "id_b"),
        ] {
            fs.ensure_index(
                &VirtualPath::new(format!("{prefix}/{suffix}")).unwrap(),
                &IndexSpec::new(
                    IndexName::new("shared_name").unwrap(),
                    vec![IndexKey::new(rank).unwrap(), IndexKey::new(id).unwrap()],
                    IndexKind::Exact,
                ),
            )
            .await
            .expect("same index name with a different prefix is independent");
        }
    }

    /// libSQL pins the legacy-trigger sweep, PostgreSQL did not. Its
    /// discovery, identifier validation, and drop path are distinct code, so a
    /// sweep that silently misses would leave the per-declaration trigger
    /// growth in place on every upgraded database while all other projection
    /// tests still pass.
    #[tokio::test]
    async fn postgres_static_projection_sweeps_legacy_triggers() {
        // Its own database: this case rewrites and then asserts the trigger set
        // of a shared table, which no path prefix can isolate.
        let Some(isolated) = postgres_isolated_root().await else {
            return;
        };
        let IsolatedDatabase {
            ref filesystem,
            ref prefix,
            ref client,
            ..
        } = isolated;
        // Seed a survivor shaped like the per-declaration generation, over the
        // test's own connection: production types must not carry test seams.
        client
            .batch_execute(
                "CREATE OR REPLACE FUNCTION idx_rfs_legacy_probe_fn() RETURNS trigger                  LANGUAGE plpgsql AS $legacy$ BEGIN                    DELETE FROM root_filesystem_ordered_index_rows WHERE path = NEW.path;                    RETURN NEW; END; $legacy$;                  DROP TRIGGER IF EXISTS idx_rfs_legacy_probe ON root_filesystem_entries;                  CREATE TRIGGER idx_rfs_legacy_probe AFTER INSERT ON root_filesystem_entries                    FOR EACH ROW EXECUTE FUNCTION idx_rfs_legacy_probe_fn();",
            )
            .await
            .expect("seed legacy trigger");

        filesystem
            .ensure_index(
                &vpath(prefix, "sweep"),
                &IndexSpec::new(
                    IndexName::new("sweep_probe_v1").unwrap(),
                    vec![IndexKey::new("rank").unwrap()],
                    IndexKind::Exact,
                ),
            )
            .await
            .expect("declaration installs the static set and sweeps legacy triggers");

        let remaining = client
            .query(
                "SELECT t.tgname FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
                 WHERE c.relname = 'root_filesystem_entries' AND NOT t.tgisinternal \
                 ORDER BY t.tgname",
                &[],
            )
            .await
            .expect("list triggers")
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>();
        assert!(
            !remaining.iter().any(|name| name.starts_with("idx_rfs_")),
            "legacy per-declaration triggers must be swept, saw {remaining:?}"
        );
        // The whole set, not just the insert trigger: an install that dropped
        // the update or delete trigger would still project new rows while
        // silently leaving stale ones behind on replacement and deletion.
        for required in [
            "rfs_ordered_projection_v3_ai",
            "rfs_ordered_projection_v3_au",
            "rfs_ordered_projection_v3_ad",
        ] {
            assert!(
                remaining.iter().any(|name| name == required),
                "the static set must install {required}, saw {remaining:?}"
            );
        }
        // And the behavior those triggers exist for, end to end.
        super::static_projection_update_and_delete_contract(filesystem, prefix).await;
        isolated.cleanup().await;
    }

    #[tokio::test]
    async fn postgres_ordered_index_declaration_never_backfills_existing_rows() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        let prefix_path = VirtualPath::new(&prefix).unwrap();
        let status = IndexKey::new("status").unwrap();
        let process_id = IndexKey::new("process_id").unwrap();
        let kind = RecordKind::new("process").unwrap();
        let old = Entry::record(kind.clone(), &serde_json::json!({}))
            .unwrap()
            .with_indexed(status.clone(), IndexValue::Text("queued".into()))
            .with_indexed(process_id.clone(), IndexValue::Text("old".into()));
        fs.put(&vpath(&prefix, "old"), old, CasExpectation::Absent)
            .await
            .unwrap();

        let unique = prefix.rsplit('_').next().unwrap();
        let spec = IndexSpec::new(
            IndexName::new(format!("no_backfill_{unique}")).unwrap(),
            vec![status.clone(), process_id.clone()],
            IndexKind::Exact,
        );
        fs.ensure_index(&prefix_path, &spec).await.unwrap();
        let page = ironclaw_filesystem::OrderedPage::new(
            spec.name.clone(),
            status.clone(),
            process_id.clone(),
            SortDirection::Ascending,
            10,
        );
        assert!(
            fs.query_ordered(&prefix_path, &Filter::All, &page)
                .await
                .unwrap()
                .is_empty(),
            "declaration must not hide a request-time table scan as automatic backfill"
        );

        let new = Entry::record(kind, &serde_json::json!({}))
            .unwrap()
            .with_indexed(status, IndexValue::Text("queued".into()))
            .with_indexed(process_id.clone(), IndexValue::Text("new".into()));
        fs.put(&vpath(&prefix, "new"), new, CasExpectation::Absent)
            .await
            .unwrap();
        let rows = fs
            .query_ordered(&prefix_path, &Filter::All, &page)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].entry.indexed[&process_id],
            IndexValue::Text("new".into())
        );
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
                    key: content.clone(),
                    query: "brown".into(),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        let natural_language = fs
            .query(
                &prefix_path,
                &Filter::Fts {
                    key: content,
                    query: "What is the brown fox?".into(),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(natural_language.len(), 1);
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
        // Scope the read-back to THIS test's GIN FTS index. Parallel postgres
        // tests share `current_schema()` and every one creates `idx_rfs_*`
        // indexes, so `ORDER BY indexname DESC LIMIT 1` alone can return another
        // test's index. The declaring prefix is uuid-unique and embedded in the
        // partial-index predicate, so match on it and require GIN (the FTS kind).
        let row = client
            .query_one(
                "SELECT indexdef FROM pg_indexes \
                 WHERE schemaname = current_schema() \
                   AND tablename = 'root_filesystem_entries' \
                   AND indexname LIKE 'idx_rfs_%' \
                   AND indexdef ILIKE '%using gin%' \
                   AND strpos(indexdef, $1) > 0 \
                 ORDER BY indexname DESC LIMIT 1",
                &[&prefix],
            )
            .await
            .expect("the GIN FTS index for this prefix must be visible");
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
// arch-exempt: large_file, fallible libSQL runtime construction only adjusts existing contract setup, plan #6175
