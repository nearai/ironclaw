#![cfg(any(feature = "libsql", feature = "postgres"))]

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::{
    MemoryDocumentFilesystem, MemoryDocumentPath, MemoryDocumentRepository, MemoryDocumentScope,
};

#[cfg(feature = "libsql")]
use ironclaw_memory::LibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
use ironclaw_memory::PostgresMemoryDocumentRepository;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_persists_documents_across_instances() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();
    repository
        .write_document(&path, b"remember this")
        .await
        .unwrap();

    let reopened = LibSqlMemoryDocumentRepository::new(db);
    assert_eq!(
        reopened.read_document(&path).await.unwrap().unwrap(),
        b"remember this"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_isolates_tenant_user_and_project_scopes() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    for (path, content) in [
        (
            MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-a alice project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-a", "bob", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-a bob project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-b", "alice", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-b alice project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-a", "alice", Some("project-2"), "MEMORY.md").unwrap(),
            b"tenant-a alice project-2".as_slice(),
        ),
    ] {
        repository.write_document(&path, content).await.unwrap();
    }

    let visible = repository
        .list_documents(&MemoryDocumentScope::new("tenant-a", "alice", Some("project-1")).unwrap())
        .await
        .unwrap();

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].tenant_id(), "tenant-a");
    assert_eq!(visible[0].user_id(), "alice");
    assert_eq!(visible[0].project_id(), Some("project-1"));
    assert_eq!(visible[0].relative_path(), "MEMORY.md");
    assert_eq!(
        repository
            .read_document(&visible[0])
            .await
            .unwrap()
            .unwrap(),
        b"tenant-a alice project-1"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_document_filesystem_reads_and_writes_through_db_repository() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let filesystem = MemoryDocumentFilesystem::new(repository);
    let path =
        VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/project-1/notes/a.md")
            .unwrap();

    filesystem
        .write_file(&path, b"filesystem db note")
        .await
        .unwrap();

    assert_eq!(
        filesystem.read_file(&path).await.unwrap(),
        b"filesystem db note"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_lists_none_project_documents_under_top_level_projects_directory()
{
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let filesystem = MemoryDocumentFilesystem::new(repository);
    let document = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/projects/_none/projects/alpha/notes.md",
    )
    .unwrap();

    filesystem
        .write_file(&document, b"unscoped project note")
        .await
        .unwrap();

    let entries = filesystem
        .list_dir(&VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/_none").unwrap())
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.name == "projects"));
    assert_eq!(
        filesystem.read_file(&document).await.unwrap(),
        b"unscoped project note"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_rejects_file_directory_prefix_conflicts() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();
    let file = MemoryDocumentPath::new("tenant-a", "alice", None, "notes").unwrap();
    let child = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/a.md").unwrap();

    repository
        .write_document(&file, b"plain file")
        .await
        .unwrap();
    let err = repository
        .write_document(&child, b"child")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing file ancestor"));

    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();
    repository.write_document(&child, b"child").await.unwrap();
    let err = repository
        .write_document(&file, b"plain file")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing directory"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_upserts_duplicate_document_paths() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();
    let (first, second) = tokio::join!(
        repository.write_document(&path, b"first"),
        repository.write_document(&path, b"second")
    );
    first.unwrap();
    second.unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT COUNT(*), content FROM memory_documents WHERE user_id = ?1 AND path = ?2 GROUP BY content",
            libsql::params!["tenant:tenant-a:user:alice:project:_none", "MEMORY.md"],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("one memory document row");
    let count: i64 = row.get(0).unwrap();
    let content: String = row.get(1).unwrap();
    assert_eq!(count, 1);
    assert!(content == "first" || content == "second");
    assert!(rows.next().await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_stores_text_in_existing_memory_documents_shape() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path =
        MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "notes/a.md").unwrap();
    repository
        .write_document(&path, b"db backed note")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT user_id, agent_id, path, content FROM memory_documents",
            (),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("memory document row");
    let user_id: String = row.get(0).unwrap();
    let agent_id: Option<String> = row.get(1).unwrap();
    let db_path: String = row.get(2).unwrap();
    let content: String = row.get(3).unwrap();

    assert_eq!(user_id, "tenant:tenant-a:user:alice:project:project-1");
    assert_eq!(agent_id, None);
    assert_eq!(db_path, "notes/a.md");
    assert_eq!(content, "db backed note");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_rejects_non_utf8_documents() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "binary.bin").unwrap();
    let err = repository
        .write_document(&path, &[0xff, 0xfe, 0xfd])
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::Backend { .. }));
    assert!(
        err.to_string()
            .contains("memory document content must be UTF-8")
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_memory_repository_implements_memory_repository_contract() {
    fn assert_repository<T: MemoryDocumentRepository>() {}
    assert_repository::<PostgresMemoryDocumentRepository>();
}

#[cfg(feature = "libsql")]
async fn libsql_db() -> (std::sync::Arc<libsql::Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("memory.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    (db, dir)
}

#[allow(dead_code)]
struct _TraitObjectCheck;

#[async_trait]
impl MemoryDocumentRepository for _TraitObjectCheck {
    async fn read_document(
        &self,
        _path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        Ok(None)
    }

    async fn write_document(
        &self,
        _path: &MemoryDocumentPath,
        _bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        Ok(())
    }

    async fn list_documents(
        &self,
        _scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        Ok(Vec::new())
    }
}
