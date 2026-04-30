use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::{
    ChunkConfig, InMemoryMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentIndexer, MemoryDocumentPath,
    MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest, RepositoryMemoryBackend,
    chunk_document,
};

#[tokio::test]
async fn backend_filesystem_adapter_routes_file_operations_with_scoped_context() {
    let backend = Arc::new(RecordingBackend::new());
    let filesystem = MemoryBackendFilesystemAdapter::new(backend.clone());
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/notes/a.md",
    )
    .unwrap();

    filesystem.write_file(&path, b"plugin note").await.unwrap();

    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"plugin note");
    let entries = filesystem
        .list_dir(
            &VirtualPath::new(
                "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/notes",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a.md");

    let seen = backend.seen_contexts.lock().unwrap();
    assert!(seen.iter().all(|ctx| ctx.scope().tenant_id() == "tenant-a"));
    assert!(seen.iter().all(|ctx| ctx.scope().user_id() == "alice"));
    assert!(
        seen.iter()
            .all(|ctx| ctx.scope().project_id() == Some("project-1"))
    );
}

#[tokio::test]
async fn backend_filesystem_adapter_fails_closed_when_file_documents_unsupported() {
    let backend = Arc::new(UnsupportedFileBackend::default());
    let filesystem = MemoryBackendFilesystemAdapter::new(backend.clone());
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes.md",
    )
    .unwrap();

    let err = filesystem
        .write_file(&path, b"must not write")
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("memory backend does not support file documents")
    );
    assert!(!backend.was_called());
}

#[tokio::test]
async fn repository_memory_backend_keeps_builtin_repository_as_default_plugin() {
    let repository = Arc::new(InMemoryMemoryDocumentRepository::new());
    let backend = Arc::new(RepositoryMemoryBackend::new(repository.clone()));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/MEMORY.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"remember via plugin boundary")
        .await
        .unwrap();

    let document_path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();
    assert_eq!(
        repository
            .read_document(&document_path)
            .await
            .unwrap()
            .unwrap(),
        b"remember via plugin boundary"
    );
}

#[tokio::test]
async fn repository_memory_backend_search_fails_closed_until_provider_is_supplied() {
    let repository = Arc::new(InMemoryMemoryDocumentRepository::new());
    let backend = RepositoryMemoryBackend::new(repository);
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    let err = backend
        .search(&context, MemorySearchRequest::new("needle").unwrap())
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("memory backend does not support search")
    );
}

#[tokio::test]
async fn repository_memory_backend_reports_write_success_when_indexer_fails_after_persist() {
    let repository = Arc::new(InMemoryMemoryDocumentRepository::new());
    let backend =
        RepositoryMemoryBackend::new(repository.clone()).with_indexer(Arc::new(FailingIndexer));
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();

    backend
        .write_document(&context, &path, b"persist despite stale derived index")
        .await
        .unwrap();

    assert_eq!(
        repository.read_document(&path).await.unwrap().unwrap(),
        b"persist despite stale derived index"
    );
}

#[test]
fn chunk_document_handles_zero_chunk_size_without_hanging() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let chunks = chunk_document(
            "alpha beta gamma",
            ChunkConfig {
                chunk_size: 0,
                overlap_percent: 0.0,
                min_chunk_size: 1,
            },
        );
        let _ = tx.send(chunks);
    });

    let chunks = rx
        .recv_timeout(Duration::from_millis(200))
        .expect("zero-sized chunk config must not hang chunking");
    assert!(!chunks.is_empty());
}

struct RecordingBackend {
    repository: InMemoryMemoryDocumentRepository,
    seen_contexts: Mutex<Vec<MemoryContext>>,
}

impl RecordingBackend {
    fn new() -> Self {
        Self {
            repository: InMemoryMemoryDocumentRepository::new(),
            seen_contexts: Mutex::new(Vec::new()),
        }
    }

    fn remember_context(&self, context: &MemoryContext) {
        self.seen_contexts.lock().unwrap().push(context.clone());
    }
}

#[async_trait]
impl MemoryBackend for RecordingBackend {
    fn capabilities(&self) -> MemoryBackendCapabilities {
        MemoryBackendCapabilities {
            file_documents: true,
            ..MemoryBackendCapabilities::default()
        }
    }

    async fn read_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        self.remember_context(context);
        self.repository.read_document(path).await
    }

    async fn write_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        self.remember_context(context);
        self.repository.write_document(path, bytes).await
    }

    async fn list_documents(
        &self,
        context: &MemoryContext,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        self.remember_context(context);
        self.repository.list_documents(scope).await
    }
}

struct FailingIndexer;

#[async_trait]
impl MemoryDocumentIndexer for FailingIndexer {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: VirtualPath::new(format!(
                "/memory/tenants/{}/users/{}/agents/{}/projects/{}/{}",
                path.tenant_id(),
                path.user_id(),
                path.agent_id().unwrap_or("_none"),
                path.project_id().unwrap_or("_none"),
                path.relative_path()
            ))
            .unwrap(),
            operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
            reason: "index unavailable".to_string(),
        })
    }
}

#[derive(Default)]
struct UnsupportedFileBackend {
    called: Mutex<bool>,
}

impl UnsupportedFileBackend {
    fn was_called(&self) -> bool {
        *self.called.lock().unwrap()
    }
}

#[async_trait]
impl MemoryBackend for UnsupportedFileBackend {
    fn capabilities(&self) -> MemoryBackendCapabilities {
        MemoryBackendCapabilities::default()
    }

    async fn write_document(
        &self,
        _context: &MemoryContext,
        _path: &MemoryDocumentPath,
        _bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        *self.called.lock().unwrap() = true;
        Ok(())
    }
}
