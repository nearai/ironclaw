use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use ironclaw_product_workflow::ResolvedBinding;

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, DiskFilesystem, Entry, FileStat,
    FilesystemError, Filter, IndexSpec, Page, RecordVersion, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{HostPath, VirtualPath};

/// Turn-state scope path for `binding` (isolated by tenant/agent/project/
/// owner user), with `root_prefix` prepended before `/tenants/...`. Shared by
/// `scoped_turns_fs` (harness.rs) and `scoped_turns_fs_composite` (builder.rs)
/// so both tiers derive turn paths from one source of truth.
pub fn turns_scope_path(root_prefix: &str, binding: &ResolvedBinding) -> String {
    let owner_user_id = binding
        .subject_user_id
        .as_ref()
        .unwrap_or(&binding.actor_user_id);
    match (binding.agent_id.as_ref(), binding.project_id.as_ref()) {
        (Some(agent_id), Some(project_id)) => format!(
            "{root_prefix}/tenants/{}/agents/{}/projects/{}/users/{}/turns",
            binding.tenant_id, agent_id, project_id, owner_user_id
        ),
        (Some(agent_id), None) => format!(
            "{root_prefix}/tenants/{}/agents/{}/users/{}/turns",
            binding.tenant_id, agent_id, owner_user_id
        ),
        (None, Some(project_id)) => format!(
            "{root_prefix}/tenants/{}/projects/{}/users/{}/turns",
            binding.tenant_id, project_id, owner_user_id
        ),
        (None, None) => format!(
            "{root_prefix}/tenants/{}/users/{}/turns",
            binding.tenant_id, owner_user_id
        ),
    }
}

pub fn local_filesystem(root: &Path) -> Result<DiskFilesystem, FilesystemError> {
    let mut fs = DiskFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").expect("valid test virtual path"),
        HostPath::from_path_buf(root.to_path_buf()),
    )?;
    Ok(fs)
}

pub struct BlockingTurnStatePutFilesystem<F> {
    inner: F,
    block_next_put: AtomicBool,
    put_blocked: AtomicBool,
    put_started: tokio::sync::Notify,
    release_put: tokio::sync::Notify,
}

impl<F> BlockingTurnStatePutFilesystem<F> {
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            block_next_put: AtomicBool::new(false),
            put_blocked: AtomicBool::new(false),
            put_started: tokio::sync::Notify::new(),
            release_put: tokio::sync::Notify::new(),
        }
    }

    pub fn block_next_put(&self) {
        self.block_next_put.store(true, Ordering::SeqCst);
    }

    pub async fn wait_for_blocked_put(&self) {
        while !self.put_blocked.load(Ordering::SeqCst) {
            self.put_started.notified().await;
        }
    }

    pub fn release_blocked_put(&self) {
        self.release_put.notify_one();
    }
}

#[async_trait]
impl<F> RootFilesystem for BlockingTurnStatePutFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if self.block_next_put.swap(false, Ordering::SeqCst) {
            self.put_blocked.store(true, Ordering::SeqCst);
            self.put_started.notify_one();
            self.release_put.notified().await;
            self.put_blocked.store(false, Ordering::SeqCst);
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir_bounded(path, max_entries).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}
