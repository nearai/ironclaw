use std::sync::Arc;

use ironclaw_filesystem::*;
use ironclaw_host_api::{HostPath, VirtualPath};
use tempfile::tempdir;

#[tokio::test]
async fn catalog_describes_paths_by_longest_matching_mount() {
    let mut root = CompositeRootFilesystem::new();
    let (broad_backend, _broad_dir) = empty_local_backend("/memory");
    let (private_backend, _private_dir) = empty_local_backend("/memory/private");

    root.mount(
        descriptor(
            "/memory",
            "memory-documents",
            BackendKind::MemoryDocuments,
            StorageClass::FileContent,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
        ),
        Arc::new(broad_backend),
    )
    .unwrap();
    root.mount(
        descriptor(
            "/memory/private",
            "private-memory-documents",
            BackendKind::MemoryDocuments,
            StorageClass::FileContent,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
        ),
        Arc::new(private_backend),
    )
    .unwrap();

    let placement = root
        .describe_path(&VirtualPath::new("/memory/private/SOUL.md").unwrap())
        .await
        .unwrap();

    assert_eq!(placement.path.as_str(), "/memory/private/SOUL.md");
    assert_eq!(placement.matched_root.as_str(), "/memory/private");
    assert_eq!(placement.backend_id.as_str(), "private-memory-documents");
    assert_eq!(placement.backend_kind, BackendKind::MemoryDocuments);
    assert_eq!(placement.content_kind, ContentKind::MemoryDocument);
    assert_eq!(placement.index_policy, IndexPolicy::FullTextAndVector);
    // Backend ops capabilities are independent of IndexPolicy — the catalog
    // policy hint drives upstream indexing services; the capability flags
    // describe what RootFilesystem ops the mounted backend actually serves.
    assert!(placement.capabilities.has(Capability::Read));
    assert!(placement.capabilities.has(Capability::Write));
}

#[tokio::test]
async fn composite_routes_filesystem_operations_to_matching_backend() {
    let memory_dir = tempdir().unwrap();
    let project_dir = tempdir().unwrap();
    std::fs::write(memory_dir.path().join("MEMORY.md"), b"remember this").unwrap();
    std::fs::write(project_dir.path().join("README.md"), b"project readme").unwrap();

    let mut memory_backend = LocalFilesystem::new();
    memory_backend
        .mount_local(
            VirtualPath::new("/memory").unwrap(),
            HostPath::from_path_buf(memory_dir.path().to_path_buf()),
        )
        .unwrap();
    let mut project_backend = LocalFilesystem::new();
    project_backend
        .mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(project_dir.path().to_path_buf()),
        )
        .unwrap();

    let mut root = CompositeRootFilesystem::new();
    root.mount(
        descriptor(
            "/memory",
            "memory-documents",
            BackendKind::MemoryDocuments,
            StorageClass::FileContent,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
        ),
        Arc::new(memory_backend),
    )
    .unwrap();
    root.mount(
        descriptor(
            "/projects",
            "project-files",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
        ),
        Arc::new(project_backend),
    )
    .unwrap();

    assert_eq!(
        root.read_file(&VirtualPath::new("/memory/MEMORY.md").unwrap())
            .await
            .unwrap(),
        b"remember this"
    );
    assert_eq!(
        root.read_file(&VirtualPath::new("/projects/README.md").unwrap())
            .await
            .unwrap(),
        b"project readme"
    );

    root.write_file(
        &VirtualPath::new("/memory/notes/new.md").unwrap(),
        b"new memory",
    )
    .await
    .unwrap();
    root.append_file(
        &VirtualPath::new("/memory/notes/new.md").unwrap(),
        b" appended",
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(memory_dir.path().join("notes/new.md")).unwrap(),
        b"new memory appended"
    );

    root.create_dir_all(&VirtualPath::new("/projects/generated/deep").unwrap())
        .await
        .unwrap();
    assert!(project_dir.path().join("generated/deep").is_dir());

    root.delete(&VirtualPath::new("/memory/notes/new.md").unwrap())
        .await
        .unwrap();
    assert!(!memory_dir.path().join("notes/new.md").exists());
}

#[tokio::test]
async fn catalog_mounts_are_sorted_for_stable_diagnostics() {
    let mut root = CompositeRootFilesystem::new();
    let (project_backend, _project_dir) = empty_local_backend("/projects");
    let (memory_backend, _memory_dir) = empty_local_backend("/memory");
    root.mount(
        descriptor(
            "/projects",
            "project-files",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
        ),
        Arc::new(project_backend),
    )
    .unwrap();
    root.mount(
        descriptor(
            "/memory",
            "memory-documents",
            BackendKind::MemoryDocuments,
            StorageClass::FileContent,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
        ),
        Arc::new(memory_backend),
    )
    .unwrap();

    let roots: Vec<String> = root
        .mounts()
        .await
        .unwrap()
        .into_iter()
        .map(|mount| mount.virtual_root.as_str().to_string())
        .collect();

    assert_eq!(roots, vec!["/memory", "/projects"]);
}

#[tokio::test]
async fn duplicate_composite_mount_roots_fail_closed() {
    let mut root = CompositeRootFilesystem::new();
    let (memory_backend, _memory_dir) = empty_local_backend("/memory");
    let (other_backend, _other_dir) = empty_local_backend("/memory");
    root.mount(
        descriptor(
            "/memory",
            "memory-documents",
            BackendKind::MemoryDocuments,
            StorageClass::FileContent,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
        ),
        Arc::new(memory_backend),
    )
    .unwrap();

    let err = root
        .mount(
            descriptor(
                "/memory",
                "other-memory-documents",
                BackendKind::MemoryDocuments,
                StorageClass::FileContent,
                ContentKind::MemoryDocument,
                IndexPolicy::FullTextAndVector,
            ),
            Arc::new(other_backend),
        )
        .unwrap_err();

    assert!(matches!(err, FilesystemError::MountConflict { .. }));
}

#[tokio::test]
async fn missing_composite_mount_fails_without_backend_side_effects() {
    let root = CompositeRootFilesystem::new();
    let err = root
        .read_file(&VirtualPath::new("/memory/MEMORY.md").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::MountNotFound { .. }));
}

fn empty_local_backend(virtual_root: &str) -> (LocalFilesystem, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let mut backend = LocalFilesystem::new();
    backend
        .mount_local(
            VirtualPath::new(virtual_root).unwrap(),
            HostPath::from_path_buf(dir.path().to_path_buf()),
        )
        .unwrap();
    (backend, dir)
}

fn descriptor(
    virtual_root: &str,
    backend_id: &str,
    backend_kind: BackendKind,
    storage_class: StorageClass,
    content_kind: ContentKind,
    index_policy: IndexPolicy,
) -> MountDescriptor {
    MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root).unwrap(),
        backend_id: BackendId::new(backend_id).unwrap(),
        backend_kind,
        storage_class,
        content_kind,
        index_policy,
        // IndexPolicy (catalog hint about how upstream services index path
        // content) is intentionally separate from `Capability::IndexFts` /
        // `Capability::IndexVector` (backend op support for `ensure_index`
        // / `query` on indexed projections). Test mounts use a LocalFilesystem
        // which doesn't ship those record-plane ops, so the descriptor
        // doesn't claim them — IndexPolicy on the descriptor still drives
        // upstream behavior independently.
        capabilities: BackendCapabilities::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with(Capability::Append)
            .with(Capability::List)
            .with(Capability::Stat),
    }
}
