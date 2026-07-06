use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    BackendCapabilities, BackendId, BackendKind, Capability, CasExpectation, ContentKind, DirEntry,
    Entry, FileStat, FilesystemError, Filter, IndexPolicy, IndexSpec, Page, RecordVersion,
    RootFilesystem, SeqNo, StorageClass, VersionedEntry, path_prefix_matches,
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

impl CompositeRootFilesystem {
    /// Explains where a [`VirtualPath`] is placed.
    ///
    /// This describes mount placement; it does not grant runtime access.
    /// Untrusted callers must still go through [`ScopedFilesystem`] and a
    /// scoped [`MountView`].
    pub async fn describe_path(
        &self,
        path: &VirtualPath,
    ) -> Result<PathPlacement, FilesystemError> {
        let mount = self.matching_mount(path)?;
        Ok(PathPlacement::from_descriptor(
            path.clone(),
            &mount.descriptor,
        ))
    }

    pub async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError> {
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
        // wanting per-path capabilities should consult
        // [`describe_path`](CompositeRootFilesystem::describe_path).
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

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .head_seq(path, from)
            .await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.matching_mount(path)?
            .backend
            .reserve_sequence(path)
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
