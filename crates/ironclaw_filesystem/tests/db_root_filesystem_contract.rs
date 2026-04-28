#![cfg(any(feature = "libsql", feature = "postgres"))]

use ironclaw_filesystem::RootFilesystem;

#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation, LibSqlRootFilesystem};
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
