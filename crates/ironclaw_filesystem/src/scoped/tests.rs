//! Caller-level tests for the operation gates added with the unified storage
//! surface. The matrix below exercises each `MountPermissions` axis against
//! each new op and asserts that the permission denial happens at the
//! `ScopedFilesystem` boundary before any backend dispatch.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    ScopedPath, TenantId, UserId, VirtualPath,
};

use super::*;
use crate::in_memory::InMemoryBackend;
use crate::{
    BackendId, BackendKind, CasExpectation, CompositeRootFilesystem, ContentKind, Entry,
    FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind, IndexName, IndexPolicy,
    IndexSpec, MountDescriptor, Page, RecordKind, SeqNo, StorageClass, TxnCapability,
};

fn test_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant_test").unwrap(),
        user_id: UserId::new("user_test").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn expect_err<T>(result: Result<T, FilesystemError>) -> FilesystemError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(err) => err,
    }
}

#[test]
fn scoped_path_class_buckets_known_segments_and_redacts_unknowns() {
    let cases = [
        ("/workspace/project/file.txt", PathClass::Workspace),
        ("/memory/profile.json", PathClass::Memory),
        ("/artifacts/run/output.json", PathClass::Artifacts),
        ("/turns/state.json", PathClass::Turns),
        ("/resources/snapshot.json", PathClass::Resources),
        ("/users/alice/private.txt", PathClass::Other),
        ("/tenants/acme/users/alice/secrets", PathClass::Other),
    ];

    for (raw, expected) in cases {
        let path = ScopedPath::new(raw).unwrap();
        assert_eq!(scoped_path_class(&path), expected);
    }
}

#[test]
fn scoped_path_detail_labels_known_snapshots_without_exposing_paths() {
    let cases = [
        ("/turns/state.json", "turn_state_snapshot"),
        ("/resources/snapshot.json", "resource_governor_snapshot"),
        ("/resources/budget-gates.json", "budget_gate_snapshot"),
        ("/resources/other.json", "unknown"),
    ];

    for (raw, expected) in cases {
        let path = ScopedPath::new(raw).unwrap();
        assert_eq!(scoped_path_detail(&path), expected);
    }
}

fn scoped_in_memory(permissions: MountPermissions) -> ScopedFilesystem<InMemoryBackend> {
    ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/engine/scoped_test").unwrap(),
            permissions,
        )])
        .unwrap(),
    )
}

fn descriptor_for(
    virtual_root: &str,
    backend: &InMemoryBackend,
    backend_id: &str,
) -> MountDescriptor {
    MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root).unwrap(),
        backend_id: BackendId::new(backend_id).unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::StructuredRecord,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: backend.capabilities(),
    }
}

fn no_op(read: bool, write: bool, list: bool, delete: bool) -> MountPermissions {
    MountPermissions {
        read,
        write,
        list,
        delete,
        execute: false,
    }
}

fn record_with_scope(scope: &str) -> Entry {
    Entry::record(
        RecordKind::new("test_kind").unwrap(),
        &serde_json::json!({}),
    )
    .unwrap()
    .with_indexed(
        IndexKey::new("scope").unwrap(),
        crate::IndexValue::Text(scope.into()),
    )
}

#[tokio::test]
async fn describe_path_uses_composite_mount_backend_capabilities() {
    let turn_backend = Arc::new(InMemoryBackend::new());
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        descriptor_for("/engine/tenants", turn_backend.as_ref(), "turns"),
        Arc::clone(&turn_backend),
    )
    .unwrap();
    let root = Arc::new(root);
    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::clone(&root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/engine/tenants/t1/users/u1/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    );

    let root_capabilities = root.capabilities();
    assert_eq!(root_capabilities.txn(), TxnCapability::None);

    let virtual_path = scoped
        .resolve(
            &test_scope(),
            &ScopedPath::new("/turns/state.json").unwrap(),
        )
        .unwrap();
    let placement = root.describe_path(&virtual_path).await.unwrap();
    assert_eq!(placement.capabilities.txn(), TxnCapability::Cas);
    assert_eq!(
        placement.path,
        VirtualPath::new("/engine/tenants/t1/users/u1/turns/state.json").unwrap()
    );
}

#[tokio::test]
async fn describe_path_returns_mount_not_found_for_unmapped_path() {
    let backend = Arc::new(InMemoryBackend::new());
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        descriptor_for("/engine/tenants", backend.as_ref(), "turns"),
        backend,
    )
    .unwrap();
    let root = Arc::new(root);
    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::clone(&root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/missing").unwrap(),
            VirtualPath::new("/engine/unmounted").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    );

    let virtual_path = scoped
        .resolve(
            &test_scope(),
            &ScopedPath::new("/missing/state.json").unwrap(),
        )
        .unwrap();
    let err = root.describe_path(&virtual_path).await.unwrap_err();
    assert!(matches!(err, FilesystemError::MountNotFound { .. }));
}

#[tokio::test]
async fn describe_path_returns_contract_error_for_missing_alias() {
    let backend = Arc::new(InMemoryBackend::new());
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        descriptor_for("/engine/tenants", backend.as_ref(), "turns"),
        backend,
    )
    .unwrap();
    let root = Arc::new(root);
    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::clone(&root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/engine/workspace").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    );

    let err = scoped
        .resolve(
            &test_scope(),
            &ScopedPath::new("/turns/state.json").unwrap(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::Contract(HostApiError::InvalidMount { .. })
    ));
}

#[tokio::test]
async fn query_denies_when_read_missing_even_with_list() {
    let scoped = scoped_in_memory(no_op(false, false, true, false));
    let err = scoped
        .query(
            &test_scope(),
            &ScopedPath::new("/workspace").unwrap(),
            &Filter::All,
            Page::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Query,
            ..
        }
    ));
}

#[tokio::test]
async fn query_denies_when_list_missing_even_with_read() {
    let scoped = scoped_in_memory(no_op(true, false, false, false));
    let err = scoped
        .query(
            &test_scope(),
            &ScopedPath::new("/workspace").unwrap(),
            &Filter::All,
            Page::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Query,
            ..
        }
    ));
}

#[tokio::test]
async fn query_succeeds_with_read_and_list() {
    let scoped = scoped_in_memory(no_op(true, true, true, false));
    scoped
        .put(
            &test_scope(),
            &ScopedPath::new("/workspace/a").unwrap(),
            record_with_scope("acme"),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    let results = scoped
        .query(
            &test_scope(),
            &ScopedPath::new("/workspace").unwrap(),
            &Filter::All,
            Page::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn read_bytes_bounded_enforces_size_at_scoped_boundary() {
    let scoped = scoped_in_memory(no_op(true, true, false, false));
    scoped
        .write_bytes(
            &test_scope(),
            &ScopedPath::new("/workspace/large.txt").unwrap(),
            b"large body".to_vec(),
        )
        .await
        .unwrap();

    let body = scoped
        .read_bytes_bounded(
            &test_scope(),
            &ScopedPath::new("/workspace/large.txt").unwrap(),
            4,
        )
        .await
        .unwrap();
    assert_eq!(body, None);
}

#[tokio::test]
async fn read_bytes_bounded_denies_when_read_missing() {
    let scoped = scoped_in_memory(no_op(false, true, false, false));
    let err = scoped
        .read_bytes_bounded(
            &test_scope(),
            &ScopedPath::new("/workspace/large.txt").unwrap(),
            4,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::ReadFile,
            ..
        }
    ));
}

#[tokio::test]
async fn ensure_index_denies_when_write_missing() {
    let scoped = scoped_in_memory(no_op(true, false, true, false));
    let spec = IndexSpec::new(
        IndexName::new("by_scope").unwrap(),
        vec![IndexKey::new("scope").unwrap()],
        IndexKind::Exact,
    );
    let err = scoped
        .ensure_index(
            &test_scope(),
            &ScopedPath::new("/workspace").unwrap(),
            &spec,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::EnsureIndex,
            ..
        }
    ));
}

#[tokio::test]
async fn ensure_index_succeeds_with_write() {
    let scoped = scoped_in_memory(no_op(false, true, false, false));
    let spec = IndexSpec::new(
        IndexName::new("by_scope").unwrap(),
        vec![IndexKey::new("scope").unwrap()],
        IndexKind::Exact,
    );
    scoped
        .ensure_index(
            &test_scope(),
            &ScopedPath::new("/workspace").unwrap(),
            &spec,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn append_batch_denies_when_write_missing() {
    let scoped = scoped_in_memory(no_op(true, false, true, false));
    let err = scoped
        .append_batch(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            vec![b"x".to_vec()],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Append,
            ..
        }
    ));
}

#[tokio::test]
async fn append_batch_succeeds_with_write_and_returns_seqs_in_order() {
    let scoped = scoped_in_memory(no_op(false, true, false, false));
    let seqs = scoped
        .append_batch(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
        )
        .await
        .unwrap();
    assert_eq!(seqs.len(), 3);
    assert!(seqs[0] < seqs[1], "seqs must be monotonically increasing");
    assert!(seqs[1] < seqs[2], "seqs must be monotonically increasing");
}

#[tokio::test]
async fn append_event_denies_when_write_missing() {
    let scoped = scoped_in_memory(no_op(true, false, true, false));
    let err = scoped
        .append(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            b"x".to_vec(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Append,
            ..
        }
    ));
}

#[tokio::test]
async fn append_event_succeeds_with_write_and_returns_monotonic_seq() {
    let scoped = scoped_in_memory(no_op(false, true, false, false));
    let s1 = scoped
        .append(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            b"a".to_vec(),
        )
        .await
        .unwrap();
    let s2 = scoped
        .append(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            b"b".to_vec(),
        )
        .await
        .unwrap();
    assert!(s2 > s1);
}

#[tokio::test]
async fn tail_denies_when_read_missing() {
    let scoped = scoped_in_memory(no_op(false, true, true, false));
    let err = scoped
        .tail(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            SeqNo::ZERO,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Tail,
            ..
        }
    ));
}

#[tokio::test]
async fn tail_succeeds_with_read_and_write() {
    let scoped = scoped_in_memory(no_op(true, true, false, false));
    let s1 = scoped
        .append(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            b"hello".to_vec(),
        )
        .await
        .unwrap();
    let events = scoped
        .tail(
            &test_scope(),
            &ScopedPath::new("/workspace/log").unwrap(),
            SeqNo::ZERO,
        )
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, s1);
}

#[tokio::test]
async fn begin_denies_when_write_missing() {
    let scoped = scoped_in_memory(no_op(true, false, true, false));
    let err = expect_err(
        scoped
            .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
            .await,
    );
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::BeginTxn,
            ..
        }
    ));
}

#[tokio::test]
async fn begin_with_write_propagates_backend_unsupported() {
    let scoped = scoped_in_memory(no_op(false, true, false, false));
    let err = expect_err(
        scoped
            .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
            .await,
    );
    assert!(
        matches!(
            err,
            FilesystemError::Unsupported {
                operation: FilesystemOperation::BeginTxn,
                ..
            }
        ),
        "expected Unsupported (gate let it through), got {err:?}"
    );
}

#[derive(Default)]
struct TxnStubBackend;

#[async_trait]
impl RootFilesystem for TxnStubBackend {
    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<crate::DirEntry>, FilesystemError> {
        Ok(Vec::new())
    }

    async fn stat(&self, path: &VirtualPath) -> Result<crate::FileStat, FilesystemError> {
        Ok(crate::FileStat {
            path: path.clone(),
            file_type: crate::FileType::Directory,
            len: 0,
            modified: None,
            sensitive: false,
        })
    }

    async fn begin(&self, _path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        Ok(Box::new(StubTxn::default()))
    }
}

#[derive(Default)]
struct StubTxn {
    seen_put: Option<VirtualPath>,
    seen_get: Option<VirtualPath>,
    seen_delete: Option<VirtualPath>,
}

#[async_trait]
impl StorageTxn for StubTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.seen_put = Some(path.clone());
        Ok(RecordVersion::from_backend(1))
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.seen_get = Some(path.clone());
        Ok(None)
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.seen_delete = Some(path.clone());
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), FilesystemError> {
        Ok(())
    }

    async fn rollback(self: Box<Self>) {}
}

fn scoped_txn_stub(permissions: MountPermissions) -> ScopedFilesystem<TxnStubBackend> {
    ScopedFilesystem::with_fixed_view(
        Arc::new(TxnStubBackend),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/engine/scoped_txn").unwrap(),
            permissions,
        )])
        .unwrap(),
    )
}

#[tokio::test]
async fn scoped_txn_rejects_put_outside_mount_prefix() {
    let scoped = scoped_txn_stub(MountPermissions::read_write());
    let mut txn = scoped
        .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap();
    let escape = VirtualPath::new("/secrets/api_key").unwrap();
    let err = txn
        .put(&escape, Entry::bytes(b"leak".to_vec()), CasExpectation::Any)
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
}

#[tokio::test]
async fn scoped_txn_rejects_get_outside_mount_prefix() {
    let scoped = scoped_txn_stub(MountPermissions::read_write());
    let mut txn = scoped
        .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap();
    let escape = VirtualPath::new("/secrets/api_key").unwrap();
    let err = txn.get(&escape).await.unwrap_err();
    assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
}

#[tokio::test]
async fn scoped_txn_rejects_delete_outside_mount_prefix() {
    let scoped = scoped_txn_stub(MountPermissions {
        read: true,
        write: true,
        list: true,
        delete: true,
        execute: false,
    });
    let mut txn = scoped
        .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap();
    let escape = VirtualPath::new("/secrets/api_key").unwrap();
    let err = txn.delete(&escape).await.unwrap_err();
    assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
}

#[tokio::test]
async fn scoped_txn_allows_put_inside_mount_prefix() {
    let scoped = scoped_txn_stub(MountPermissions::read_write());
    let mut txn = scoped
        .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap();
    let inside = VirtualPath::new("/engine/scoped_txn/file").unwrap();
    txn.put(&inside, Entry::bytes(b"ok".to_vec()), CasExpectation::Any)
        .await
        .unwrap();
}

#[tokio::test]
async fn scoped_txn_per_op_acl_blocks_delete_without_delete_permission() {
    let scoped = scoped_txn_stub(MountPermissions::read_write());
    let mut txn = scoped
        .begin(&test_scope(), &ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap();
    let inside = VirtualPath::new("/engine/scoped_txn/file").unwrap();
    let err = txn.delete(&inside).await.unwrap_err();
    match err {
        FilesystemError::Backend {
            operation: FilesystemOperation::Delete,
            reason,
            ..
        } => {
            assert!(
                reason.contains("permission"),
                "expected permission-denial reason, got {reason}"
            );
        }
        other => panic!("expected Backend(permission), got {other:?}"),
    }
}
