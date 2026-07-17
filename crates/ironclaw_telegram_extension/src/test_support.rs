use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, TenantId, UserId, VirtualPath,
};

use crate::state::FilesystemTelegramHostState;

pub(crate) fn telegram_state() -> Arc<FilesystemTelegramHostState> {
    let root: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::default());
    telegram_state_with_root(root)
}

pub(crate) fn fault_injected_telegram_state() -> (
    Arc<FilesystemTelegramHostState>,
    Arc<FaultInjectingFilesystem>,
) {
    let filesystem = Arc::new(FaultInjectingFilesystem::new(Arc::new(
        InMemoryBackend::default(),
    )));
    let root: Arc<dyn RootFilesystem> = filesystem.clone();
    (telegram_state_with_root(root), filesystem)
}

pub(crate) fn telegram_state_with_root(
    root: Arc<dyn RootFilesystem>,
) -> Arc<FilesystemTelegramHostState> {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("mount alias"),
        VirtualPath::new("/tenants/tenant-alpha/shared").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped: Arc<ScopedFilesystem<dyn RootFilesystem>> =
        Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    Arc::new(FilesystemTelegramHostState::new(
        scoped,
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("operator").expect("user"),
        AgentId::new("agent-alpha").expect("agent"),
        None,
    ))
}

/// Filesystem-level fault controller used instead of parallel Telegram store
/// fakes. Every non-faulted operation delegates to the production in-memory
/// backend, so tests still exercise real record paths, JSON, locks, and CAS.
pub(crate) struct FaultInjectingFilesystem {
    inner: Arc<dyn RootFilesystem>,
    fail_reads: AtomicBool,
    fail_writes: AtomicBool,
    fail_deletes: AtomicBool,
    fail_versioned_writes: AtomicBool,
    next_read_barrier: std::sync::Mutex<Option<(usize, Arc<tokio::sync::Barrier>)>>,
}

impl std::fmt::Debug for FaultInjectingFilesystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FaultInjectingFilesystem")
            .finish_non_exhaustive()
    }
}

impl FaultInjectingFilesystem {
    pub(crate) fn new(inner: Arc<dyn RootFilesystem>) -> Self {
        Self {
            inner,
            fail_reads: AtomicBool::new(false),
            fail_writes: AtomicBool::new(false),
            fail_deletes: AtomicBool::new(false),
            fail_versioned_writes: AtomicBool::new(false),
            next_read_barrier: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn fail_reads(&self) {
        self.fail_reads.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_writes(&self) {
        self.fail_writes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_deletes(&self) {
        self.fail_deletes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_versioned_writes(&self) {
        self.fail_versioned_writes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn hold_next_reads_at(&self, read_count: usize, barrier: Arc<tokio::sync::Barrier>) {
        let mut slot = match self.next_read_barrier.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some((read_count, barrier));
    }

    fn injected(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
        FilesystemError::Backend {
            path: path.clone(),
            operation,
            reason: "test-injected filesystem failure".to_string(),
        }
    }
}

#[async_trait]
impl RootFilesystem for FaultInjectingFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if self.fail_writes.load(Ordering::SeqCst)
            || (matches!(cas, CasExpectation::Version(_))
                && self.fail_versioned_writes.load(Ordering::SeqCst))
        {
            return Err(Self::injected(path, FilesystemOperation::WriteFile));
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(Self::injected(path, FilesystemOperation::ReadFile));
        }
        let barrier = {
            let mut slot = match self.next_read_barrier.lock() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some((remaining, barrier)) = slot.as_mut() {
                let barrier = Arc::clone(barrier);
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    *slot = None;
                }
                Some(barrier)
            } else {
                None
            }
        };
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if self.fail_deletes.load(Ordering::SeqCst) {
            return Err(Self::injected(path, FilesystemOperation::Delete));
        }
        self.inner.delete(path).await
    }
}
