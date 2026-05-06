use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::*;

#[test]
fn memory_scope_rejects_segments_that_cannot_round_trip_or_collide_in_owner_keys() {
    assert!(MemoryDocumentScope::new("tenant:admin", "alice", None).is_err());
    assert!(MemoryDocumentScope::new(".", "alice", None).is_err());
    assert!(MemoryDocumentScope::new("tenant-a", "..", None).is_err());
}

#[tokio::test]
async fn in_memory_repository_rejects_file_directory_prefix_conflicts() {
    let repo = InMemoryMemoryDocumentRepository::new();
    let file = MemoryDocumentPath::new("tenant-a", "alice", None, "notes").unwrap();
    let child = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/a.md").unwrap();

    repo.write_document(&file, b"plain file").await.unwrap();
    let err = repo.write_document(&child, b"child").await.unwrap_err();
    assert!(err.to_string().contains("existing file ancestor"));

    let repo = InMemoryMemoryDocumentRepository::new();
    repo.write_document(&child, b"child").await.unwrap();
    let err = repo.write_document(&file, b"plain file").await.unwrap_err();
    assert!(err.to_string().contains("existing directory"));
}

#[tokio::test]
async fn memory_filesystem_maps_virtual_paths_to_repository_keys() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo.clone());
    let key =
        MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "notes/a.md").unwrap();

    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/notes/a.md",
        )
        .unwrap(),
        b"memory note",
    )
    .await
    .unwrap();

    assert_eq!(
        repo.read_document(&key).await.unwrap().unwrap(),
        b"memory note"
    );
    assert_eq!(
        fs.read_file(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/notes/a.md"
            )
            .unwrap()
        )
        .await
        .unwrap(),
        b"memory note"
    );
}

#[tokio::test]
async fn memory_filesystem_accepts_transitional_no_agent_paths_as_absent_agent_scope() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo.clone());
    let key =
        MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "notes/a.md").unwrap();

    fs.write_file(
        &VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/project-1/notes/a.md")
            .unwrap(),
        b"legacy no-agent path",
    )
    .await
    .unwrap();

    assert_eq!(key.agent_id(), None);
    assert_eq!(
        repo.read_document(&key).await.unwrap().unwrap(),
        b"legacy no-agent path"
    );
}

#[tokio::test]
async fn memory_filesystem_maps_agent_scoped_virtual_paths_to_repository_keys() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo.clone());
    let key = MemoryDocumentPath::new_with_agent(
        "tenant-a",
        "alice",
        Some("agent-1"),
        Some("project-1"),
        "notes/a.md",
    )
    .unwrap();

    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/agent-1/projects/project-1/notes/a.md",
        )
        .unwrap(),
        b"agent-scoped memory note",
    )
    .await
    .unwrap();

    assert_eq!(key.agent_id(), Some("agent-1"));
    assert_eq!(
        repo.read_document(&key).await.unwrap().unwrap(),
        b"agent-scoped memory note"
    );
}

#[tokio::test]
async fn memory_filesystem_preserves_agent_isolation_and_absent_agent_scope() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);

    for (path, bytes) in [
        (
            "/memory/tenants/tenant-a/users/alice/agents/agent-1/projects/project-1/MEMORY.md",
            b"agent 1".as_slice(),
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/agent-2/projects/project-1/MEMORY.md",
            b"agent 2".as_slice(),
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/MEMORY.md",
            b"no agent".as_slice(),
        ),
    ] {
        fs.write_file(&VirtualPath::new(path).unwrap(), bytes)
            .await
            .unwrap();
    }

    for (path, expected) in [
        (
            "/memory/tenants/tenant-a/users/alice/agents/agent-1/projects/project-1/MEMORY.md",
            b"agent 1".as_slice(),
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/agent-2/projects/project-1/MEMORY.md",
            b"agent 2".as_slice(),
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/MEMORY.md",
            b"no agent".as_slice(),
        ),
    ] {
        assert_eq!(
            fs.read_file(&VirtualPath::new(path).unwrap())
                .await
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn memory_filesystem_preserves_tenant_user_project_isolation() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);

    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/MEMORY.md",
        )
        .unwrap(),
        b"alice tenant-a project-1",
    )
    .await
    .unwrap();
    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/bob/agents/_none/projects/project-1/MEMORY.md",
        )
        .unwrap(),
        b"bob tenant-a project-1",
    )
    .await
    .unwrap();
    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-b/users/alice/agents/_none/projects/project-1/MEMORY.md",
        )
        .unwrap(),
        b"alice tenant-b project-1",
    )
    .await
    .unwrap();
    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-2/MEMORY.md",
        )
        .unwrap(),
        b"alice tenant-a project-2",
    )
    .await
    .unwrap();

    assert_eq!(
        fs.read_file(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/MEMORY.md"
            )
            .unwrap(),
        )
        .await
        .unwrap(),
        b"alice tenant-a project-1"
    );
    assert_eq!(
        fs.read_file(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/bob/agents/_none/projects/project-1/MEMORY.md"
            )
            .unwrap(),
        )
        .await
        .unwrap(),
        b"bob tenant-a project-1"
    );
    assert_eq!(
        fs.read_file(
            &VirtualPath::new(
                "/memory/tenants/tenant-b/users/alice/agents/_none/projects/project-1/MEMORY.md"
            )
            .unwrap(),
        )
        .await
        .unwrap(),
        b"alice tenant-b project-1"
    );
    assert_eq!(
        fs.read_file(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-2/MEMORY.md"
            )
            .unwrap(),
        )
        .await
        .unwrap(),
        b"alice tenant-a project-2"
    );
}

#[tokio::test]
async fn memory_filesystem_lists_direct_children_from_document_paths() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);

    for (path, bytes) in [
        (
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/SOUL.md",
            b"soul".as_slice(),
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/a.md",
            b"a",
        ),
        (
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/deep/b.md",
            b"b",
        ),
    ] {
        fs.write_file(&VirtualPath::new(path).unwrap(), bytes)
            .await
            .unwrap();
    }

    let entries = fs
        .list_dir(
            &VirtualPath::new("/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none")
                .unwrap(),
        )
        .await
        .unwrap();

    let summary: Vec<_> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.file_type, entry.path.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "SOUL.md",
                FileType::File,
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/SOUL.md",
            ),
            (
                "notes",
                FileType::Directory,
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes",
            ),
        ]
    );
}

#[tokio::test]
async fn memory_filesystem_stats_exact_files_and_inferred_directories() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);

    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/a.md",
        )
        .unwrap(),
        b"abc",
    )
    .await
    .unwrap();

    let file = fs
        .stat(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/a.md",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file.file_type, FileType::File);
    assert_eq!(file.len, 3);

    let directory = fs
        .stat(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(directory.file_type, FileType::Directory);
    assert_eq!(directory.len, 0);

    let err = fs
        .stat(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/missing.md",
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::Backend { .. }));
    assert!(
        err.to_string()
            .contains("/memory/tenants/tenant-a/users/alice")
    );
}

#[tokio::test]
async fn memory_filesystem_invokes_indexer_after_writes() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let indexer = Arc::new(RecordingIndexer::default());
    let fs = MemoryDocumentFilesystem::new(repo).with_indexer(indexer.clone());

    fs.write_file(
        &VirtualPath::new(
            "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/AGENTS.md",
        )
        .unwrap(),
        b"agent instructions",
    )
    .await
    .unwrap();

    let indexed = indexer.paths.lock().unwrap().clone();
    assert_eq!(indexed.len(), 1);
    assert_eq!(indexed[0].tenant_id(), "tenant-a");
    assert_eq!(indexed[0].user_id(), "alice");
    assert_eq!(indexed[0].project_id(), Some("project-1"));
    assert_eq!(indexed[0].relative_path(), "AGENTS.md");
}

#[tokio::test]
async fn memory_filesystem_rejects_file_directory_prefix_conflicts() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);
    let file_path =
        VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/_none/notes").unwrap();
    let child_path =
        VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/_none/notes/a.md").unwrap();

    fs.write_file(&file_path, b"plain file").await.unwrap();
    let err = fs.write_file(&child_path, b"child").await.unwrap_err();
    assert!(matches!(err, FilesystemError::Backend { .. }));
    assert!(err.to_string().contains("existing file ancestor"));

    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);
    fs.write_file(&child_path, b"child").await.unwrap();
    let err = fs.write_file(&file_path, b"plain file").await.unwrap_err();
    assert!(matches!(err, FilesystemError::Backend { .. }));
    assert!(err.to_string().contains("existing directory"));
}

#[tokio::test]
async fn memory_filesystem_reports_success_when_best_effort_indexer_fails_after_persist() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let indexer = Arc::new(FailingIndexer);
    let fs = MemoryDocumentFilesystem::new(repo.clone()).with_indexer(indexer);
    let path =
        VirtualPath::new("/memory/tenants/tenant-a/users/alice/projects/_none/MEMORY.md").unwrap();
    let key = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();

    fs.write_file(&path, b"persisted despite derived index failure")
        .await
        .unwrap();

    assert_eq!(
        repo.read_document(&key).await.unwrap().unwrap(),
        b"persisted despite derived index failure"
    );
}

#[tokio::test]
async fn memory_filesystem_rejects_non_document_memory_paths() {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let fs = MemoryDocumentFilesystem::new(repo);

    let err = fs
        .write_file(
            &VirtualPath::new("/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none")
                .unwrap(),
            b"not a document",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::Backend { .. }));
}

#[derive(Default)]
struct RecordingIndexer {
    paths: Mutex<Vec<MemoryDocumentPath>>,
}

#[async_trait]
impl MemoryDocumentIndexer for RecordingIndexer {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        self.paths.lock().unwrap().push(path.clone());
        Ok(())
    }
}

struct FailingIndexer;

#[async_trait]
impl MemoryDocumentIndexer for FailingIndexer {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: VirtualPath::new(format!(
                "/memory/tenants/{}/users/{}/projects/{}/{}",
                path.tenant_id(),
                path.user_id(),
                path.project_id().unwrap_or("_none"),
                path.relative_path()
            ))
            .unwrap(),
            operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
            reason: "index unavailable".to_string(),
        })
    }
}
