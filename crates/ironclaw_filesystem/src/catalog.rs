use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    BackendCapabilities, BackendId, BackendKind, BatchPut, Capability, CasExpectation, ContentKind,
    DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, Filter, IndexPolicy,
    IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo, StorageClass, VersionedEntry,
    path_prefix_matches,
};

/// Trusted catalog record for one virtual filesystem mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountDescriptor {
    pub virtual_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

/// Catalog answer for the backend that owns a virtual path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPlacement {
    pub path: VirtualPath,
    pub matched_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

impl PathPlacement {
    fn from_descriptor(path: VirtualPath, descriptor: &MountDescriptor) -> Self {
        Self {
            path,
            matched_root: descriptor.virtual_root.clone(),
            backend_id: descriptor.backend_id.clone(),
            backend_kind: descriptor.backend_kind.clone(),
            storage_class: descriptor.storage_class,
            content_kind: descriptor.content_kind,
            index_policy: descriptor.index_policy,
            capabilities: descriptor.capabilities,
        }
    }
}

/// Trusted catalog over virtual filesystem mount placement.
///
/// The catalog explains where a [`VirtualPath`] is placed; it does not grant
/// runtime access. Untrusted callers must still go through [`ScopedFilesystem`]
/// and a scoped [`MountView`].
#[async_trait]
pub trait FilesystemCatalog: Send + Sync {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError>;

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError>;
}

/// Root filesystem that composes multiple backend roots behind one virtual namespace.
pub struct CompositeRootFilesystem {
    mounts: Vec<CompositeMount>,
}

struct CompositeMount {
    descriptor: MountDescriptor,
    backend: Arc<dyn RootFilesystem>,
}

impl CompositeRootFilesystem {
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    pub fn mount<F>(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<F>,
    ) -> Result<(), FilesystemError>
    where
        F: RootFilesystem + 'static,
    {
        let backend: Arc<dyn RootFilesystem> = backend;
        self.mount_dyn(descriptor, backend)
    }

    pub fn mount_dyn(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<dyn RootFilesystem>,
    ) -> Result<(), FilesystemError> {
        if self
            .mounts
            .iter()
            .any(|mount| mount.descriptor.virtual_root.as_str() == descriptor.virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict {
                path: descriptor.virtual_root,
            });
        }
        // PR #3659 reviewer fix: validate the descriptor's advertised
        // capabilities against the backend's actual capabilities at
        // mount time. Catalog metadata that claims query/index/event
        // support over a backend that doesn't provide it would defeat
        // the PR's mount-time validation guarantee — fail closed instead.
        validate_mount_capabilities(&descriptor, backend.capabilities())?;
        self.mounts.push(CompositeMount {
            descriptor,
            backend,
        });
        Ok(())
    }

    fn matching_mount(&self, path: &VirtualPath) -> Result<&CompositeMount, FilesystemError> {
        self.mounts
            .iter()
            .filter(|mount| {
                path_prefix_matches(mount.descriptor.virtual_root.as_str(), path.as_str())
            })
            .max_by_key(|mount| mount.descriptor.virtual_root.as_str().len())
            .ok_or_else(|| FilesystemError::MountNotFound { path: path.clone() })
    }

    /// Resolve a multi-path batch to the single [`CompositeMount`] that owns
    /// every path, or fail closed.
    ///
    /// A batch may not straddle mounts: every path must resolve to the same
    /// mount as the first item, else the whole call fails `PathOutsideMount`.
    /// Mount identity is compared by pointer on the resolved `CompositeMount`,
    /// so one `matching_mount` resolve per path settles routing. Shared by
    /// [`put_batch`](RootFilesystem::put_batch) and
    /// [`get_batch`](RootFilesystem::get_batch) composite dispatch.
    ///
    /// An empty `items` slice returns [`FilesystemError::BackendInfrastructure`]
    /// (`op`) — no path is in scope to resolve. Callers whose empty case is a
    /// valid no-op (e.g. `get_batch`) must short-circuit *before* calling this.
    fn resolve_single_mount_batch<'a, T>(
        &'a self,
        items: &[T],
        path_of: impl Fn(&T) -> &VirtualPath,
        op: FilesystemOperation,
    ) -> Result<&'a CompositeMount, FilesystemError> {
        let Some(first) = items.first() else {
            return Err(FilesystemError::BackendInfrastructure {
                operation: op,
                reason: "empty batch".to_string(),
            });
        };
        let first_mount = self.matching_mount(path_of(first))?;
        for item in &items[1..] {
            let mount = self.matching_mount(path_of(item))?;
            if !std::ptr::eq(mount, first_mount) {
                return Err(FilesystemError::PathOutsideMount {
                    path: path_of(item).clone(),
                });
            }
        }
        Ok(first_mount)
    }
}

impl Default for CompositeRootFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

/// PR #3659 reviewer fix: reject a [`MountDescriptor`] whose advertised
/// capabilities claim more than the backend actually delivers on the
/// **new** capability axes (records, query, index, events, txn).
///
/// Scope deliberately limited to the new axes: the legacy bytes-plane
/// flags (`read`/`write`/`list`/`stat`/`delete`/`append`) have always
/// been descriptor-driven metadata, and many existing backends still
/// return `BackendCapabilities::default()` (all-false) from their
/// `capabilities()` accessor even though they implement
/// `read_file`/`write_file` natively. The mount-time validation
/// guarantee the reviewer asked for applies to the new capability
/// surface that this PR introduces; downstream catalog clients are the
/// authority for the legacy plane until each backend opts in to a more
/// accurate `capabilities()` override.
fn validate_mount_capabilities(
    descriptor: &MountDescriptor,
    backend: BackendCapabilities,
) -> Result<(), FilesystemError> {
    let declared = descriptor.capabilities;
    // Only validate the **new** capability axes — legacy bytes flags stay
    // descriptor-driven (see the function-level doc comment).
    const NEW_AXES: &[Capability] = &[
        Capability::Records,
        Capability::Query,
        Capability::IndexExact,
        Capability::IndexPrefix,
        Capability::IndexFts,
        Capability::IndexVector,
        Capability::Events,
        Capability::BatchPut,
    ];
    let mut shortfalls: Vec<Capability> = NEW_AXES
        .iter()
        .copied()
        .filter(|cap| declared.has(*cap) && !backend.has(*cap))
        .collect();
    let backend_txn = txn_capability_rank(backend.txn());
    let declared_txn = txn_capability_rank(declared.txn());
    let txn_shortfall = declared_txn > backend_txn;
    if shortfalls.is_empty() && !txn_shortfall {
        Ok(())
    } else {
        Err(FilesystemError::DescriptorOverclaims {
            path: descriptor.virtual_root.clone(),
            missing: std::mem::take(&mut shortfalls),
            txn_shortfall,
        })
    }
}

fn txn_capability_rank(value: crate::TxnCapability) -> u8 {
    match value {
        crate::TxnCapability::None => 0,
        crate::TxnCapability::Cas => 1,
        crate::TxnCapability::MultiKey => 2,
    }
}

#[async_trait]
impl FilesystemCatalog for CompositeRootFilesystem {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError> {
        let mount = self.matching_mount(path)?;
        Ok(PathPlacement::from_descriptor(
            path.clone(),
            &mount.descriptor,
        ))
    }

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError> {
        let mut mounts: Vec<_> = self
            .mounts
            .iter()
            .map(|mount| mount.descriptor.clone())
            .collect();
        mounts.sort_by(|left, right| left.virtual_root.as_str().cmp(right.virtual_root.as_str()));
        Ok(mounts)
    }
}

#[async_trait]
impl RootFilesystem for CompositeRootFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        // The composite is a router, not a backend in its own right. Callers
        // wanting per-path capabilities should consult [`describe_path`]
        // through the [`FilesystemCatalog`] impl.
        BackendCapabilities::default()
    }

    // ── Unified entry plane ──

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .put(path, entry, cas)
            .await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.matching_mount(path)?.backend.get(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .query(path, filter, page)
            .await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .ensure_index(path, spec)
            .await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        self.matching_mount(path)?.backend.begin(path).await
    }

    // A write batch may not straddle mounts (see `resolve_single_mount_batch`):
    // every path must resolve to the same mount as the first leg, else the whole
    // call fails `PathOutsideMount` and nothing is written.
    async fn put_batch(&self, puts: Vec<BatchPut>) -> Result<Vec<RecordVersion>, FilesystemError> {
        let mount =
            self.resolve_single_mount_batch(&puts, |p| &p.path, FilesystemOperation::PutBatch)?;
        mount.backend.put_batch(puts).await
    }

    // Read analogue of `put_batch`. Empty is a valid no-op (`Ok(vec![])`), so we
    // short-circuit before resolving a mount; otherwise the same single-mount
    // dispatcher routes every path to one backend `get_batch`.
    async fn get_batch(
        &self,
        paths: Vec<VirtualPath>,
    ) -> Result<Vec<Option<VersionedEntry>>, FilesystemError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mount =
            self.resolve_single_mount_batch(&paths, |p| p, FilesystemOperation::ReadFile)?;
        mount.backend.get_batch(paths).await
    }

    // ── Event plane ──

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .append(path, payload)
            .await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .append_batch(path, payloads)
            .await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.matching_mount(path)?.backend.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .tail_bounded(path, from, max_records)
            .await
    }

    // ── Legacy bytes plane ──

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.matching_mount(path)?.backend.read_file(path).await
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .read_file_bounded(path, max_bytes)
            .await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .write_file(path, bytes)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .append_file(path, bytes)
            .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.matching_mount(path)?.backend.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.matching_mount(path)?.backend.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?.backend.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .create_dir_all(path)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::VirtualPath;

    use crate::{
        BackendCapabilities, BackendId, BackendKind, BatchPut, CasExpectation,
        CompositeRootFilesystem, ContentKind, Entry, FilesystemError, FilesystemOperation,
        InMemoryBackend, IndexPolicy, MountDescriptor, RootFilesystem, StorageClass,
    };

    fn descriptor(root: &str) -> MountDescriptor {
        MountDescriptor {
            virtual_root: VirtualPath::new(root).unwrap(),
            backend_id: BackendId::new(format!("mem{}", root.replace('/', "_"))).unwrap(),
            backend_kind: BackendKind::MemoryDocuments,
            storage_class: StorageClass::StructuredRecords,
            content_kind: ContentKind::StructuredRecord,
            index_policy: IndexPolicy::NotIndexed,
            capabilities: BackendCapabilities::in_memory_full(),
        }
    }

    fn vp(s: &str) -> VirtualPath {
        VirtualPath::new(s).unwrap()
    }

    #[tokio::test]
    async fn put_batch_single_routes_to_owning_mount() {
        let mut composite = CompositeRootFilesystem::new();
        let secrets = Arc::new(InMemoryBackend::new());
        composite
            .mount(descriptor("/secrets"), secrets.clone())
            .unwrap();

        let versions = composite
            .put_batch(vec![BatchPut {
                path: vp("/secrets/leases/A"),
                entry: Entry::bytes(vec![9]),
                cas: CasExpectation::Absent,
            }])
            .await
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(
            secrets
                .get(&vp("/secrets/leases/A"))
                .await
                .unwrap()
                .unwrap()
                .entry
                .body,
            vec![9]
        );
    }

    #[tokio::test]
    async fn put_batch_cross_mount_rejected_writes_nothing() {
        let mut composite = CompositeRootFilesystem::new();
        let secrets = Arc::new(InMemoryBackend::new());
        let memory = Arc::new(InMemoryBackend::new());
        composite
            .mount(descriptor("/secrets"), secrets.clone())
            .unwrap();
        composite
            .mount(descriptor("/memory"), memory.clone())
            .unwrap();

        let err = composite
            .put_batch(vec![
                BatchPut {
                    path: vp("/secrets/leases/A"),
                    entry: Entry::bytes(vec![1]),
                    cas: CasExpectation::Absent,
                },
                BatchPut {
                    path: vp("/memory/docs/B"),
                    entry: Entry::bytes(vec![2]),
                    cas: CasExpectation::Absent,
                },
            ])
            .await
            .unwrap_err();
        assert!(
            matches!(err, FilesystemError::PathOutsideMount { .. }),
            "cross-mount put_batch must be rejected, got {err:?}"
        );
        // The composite rejects before delegating, so neither backend wrote.
        assert!(
            secrets
                .get(&vp("/secrets/leases/A"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(memory.get(&vp("/memory/docs/B")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_batch_empty_rejected() {
        let composite = CompositeRootFilesystem::new();
        let err = composite.put_batch(Vec::new()).await.unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::PutBatch,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn get_batch_routes_to_owning_mount_in_order_with_none() {
        // Through the composite dispatcher: seed a and c on the /secrets mount,
        // skip b, and read [a,b,c] in one call. The shared single-mount
        // dispatcher routes all three to the one backend, which returns
        // [Some(a), None, Some(c)] in input order.
        let mut composite = CompositeRootFilesystem::new();
        let secrets = Arc::new(InMemoryBackend::new());
        composite
            .mount(descriptor("/secrets"), secrets.clone())
            .unwrap();
        secrets
            .put(
                &vp("/secrets/leases/a"),
                Entry::bytes(vec![0xA]),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
        secrets
            .put(
                &vp("/secrets/leases/c"),
                Entry::bytes(vec![0xC]),
                CasExpectation::Absent,
            )
            .await
            .unwrap();

        let out = composite
            .get_batch(vec![
                vp("/secrets/leases/a"),
                vp("/secrets/leases/b"),
                vp("/secrets/leases/c"),
            ])
            .await
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].as_ref().unwrap().entry.body, vec![0xA]);
        assert!(out[1].is_none());
        assert_eq!(out[2].as_ref().unwrap().entry.body, vec![0xC]);
    }

    #[tokio::test]
    async fn get_batch_empty_is_ok_noop() {
        // The composite short-circuits an empty read batch to Ok(vec![]) before
        // resolving any mount — unlike the empty put_batch, which is an error.
        let composite = CompositeRootFilesystem::new();
        let out = composite.get_batch(Vec::new()).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn get_batch_cross_mount_rejected() {
        // A read batch may not straddle mounts: same single-mount dispatcher as
        // put_batch, so a path on a second mount fails PathOutsideMount.
        let mut composite = CompositeRootFilesystem::new();
        let secrets = Arc::new(InMemoryBackend::new());
        let memory = Arc::new(InMemoryBackend::new());
        composite
            .mount(descriptor("/secrets"), secrets.clone())
            .unwrap();
        composite
            .mount(descriptor("/memory"), memory.clone())
            .unwrap();

        let err = composite
            .get_batch(vec![vp("/secrets/leases/a"), vp("/memory/docs/b")])
            .await
            .unwrap_err();
        assert!(
            matches!(err, FilesystemError::PathOutsideMount { .. }),
            "cross-mount get_batch must be rejected, got {err:?}"
        );
    }
}
