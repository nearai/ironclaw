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
    assert_eq!(
        root.read_file_bounded(&VirtualPath::new("/memory/MEMORY.md").unwrap(), 13)
            .await
            .unwrap(),
        Some(b"remember this".to_vec())
    );
    assert_eq!(
        root.read_file_bounded(&VirtualPath::new("/projects/README.md").unwrap(), 14)
            .await
            .unwrap(),
        Some(b"project readme".to_vec())
    );
    assert_eq!(
        root.read_file_bounded(&VirtualPath::new("/projects/README.md").unwrap(), 12)
            .await
            .unwrap(),
        None
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

    let err = root
        .read_file_bounded(&VirtualPath::new("/memory/MEMORY.md").unwrap(), 1024)
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::MountNotFound { .. }));
}

#[tokio::test]
async fn composite_routes_append_batch_to_matching_backend() {
    // Two InMemoryBackend mounts: broad at /events, more-specific at /events/engine.
    // append_batch to /events/engine/... must route to the /events/engine backend
    // (longest prefix), return N monotonic SeqNos for N payloads, and leave the
    // /events backend empty.
    let broad = Arc::new(InMemoryBackend::new());
    let specific = Arc::new(InMemoryBackend::new());

    let mut root = CompositeRootFilesystem::new();
    root.mount(
        event_log_descriptor("/events", "broad-events"),
        Arc::clone(&broad),
    )
    .unwrap();
    root.mount(
        event_log_descriptor("/events/engine", "specific-events"),
        Arc::clone(&specific),
    )
    .unwrap();

    let log = VirtualPath::new("/events/engine/log.jsonl").unwrap();
    let payloads: Vec<Vec<u8>> = vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()];

    // 1. Longest-prefix routing: append_batch dispatches to the /events/engine mount.
    let seqs = root.append_batch(&log, payloads.clone()).await.unwrap();
    assert_eq!(seqs.len(), 3);

    // 2. Ordered seqs: returned SeqNos are strictly monotonic in payload order.
    assert!(seqs[0] < seqs[1] && seqs[1] < seqs[2]);

    // The /events/engine backend holds all three records in payload order.
    let records = specific.tail(&log, SeqNo::ZERO).await.unwrap();
    assert_eq!(records.len(), 3);
    for (i, payload) in payloads.iter().enumerate() {
        assert_eq!(records[i].payload, *payload);
        assert_eq!(records[i].seq, seqs[i]);
    }

    // The broad /events backend received nothing.
    assert!(
        broad.tail(&log, SeqNo::ZERO).await.unwrap().is_empty(),
        "broad /events mount must not receive /events/engine appends"
    );
}

#[tokio::test]
async fn composite_append_batch_returns_mount_not_found() {
    // A composite with one mount at /events.  An append_batch to /logs/…
    // (outside all mounts) must return MountNotFound and leave the /events
    // backend completely empty — no side effects.
    let backend = Arc::new(InMemoryBackend::new());

    let mut root = CompositeRootFilesystem::new();
    root.mount(
        event_log_descriptor("/events", "events-backend"),
        Arc::clone(&backend),
    )
    .unwrap();

    // Path under a valid virtual root (/memory) that has no matching mount.
    let unmapped = VirtualPath::new("/memory/system.jsonl").unwrap();
    let payloads: Vec<Vec<u8>> = vec![b"payload".to_vec()];

    let err = root.append_batch(&unmapped, payloads).await.unwrap_err();

    assert!(
        matches!(err, FilesystemError::MountNotFound { .. }),
        "expected MountNotFound, got {err:?}"
    );

    // The mounted /events backend must not have received any write.
    let records = backend
        .tail(&VirtualPath::new("/events/log.jsonl").unwrap(), SeqNo::ZERO)
        .await
        .unwrap();
    assert!(
        records.is_empty(),
        "/events backend must not receive writes for an unmapped path"
    );
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

fn event_log_descriptor(virtual_root: &str, backend_id: &str) -> MountDescriptor {
    MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root).unwrap(),
        backend_id: BackendId::new(backend_id).unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::SystemState,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: BackendCapabilities::in_memory_full(),
    }
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
