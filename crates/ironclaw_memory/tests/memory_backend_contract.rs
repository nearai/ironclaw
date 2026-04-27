use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::{
    InMemoryMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentPath, MemoryDocumentRepository,
    MemoryDocumentScope, MemorySearchRequest, RepositoryMemoryBackend,
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
