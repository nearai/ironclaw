use std::sync::Arc;

use ironclaw_filesystem::{
    CompositeRootFilesystem, LocalFilesystem, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ThreadId, VirtualPath,
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use thiserror::Error;

use super::filesystem::local_filesystem;

#[derive(Debug, Error)]
pub enum RebornThreadHarnessError {
    #[error("failed to create thread harness tempdir: {0}")]
    Tempdir(#[from] std::io::Error),
    #[error("failed to configure local filesystem: {0}")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("invalid mount view: {0}")]
    MountView(#[from] ironclaw_host_api::HostApiError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("thread history does not contain final assistant reply containing {0:?}")]
    MissingFinalReply(String),
}

/// Thin harness over a `FilesystemSessionThreadService<F>` for asserting thread
/// history in integration and binary-E2E tests.
///
/// The type parameter `F` defaults to `LocalFilesystem` so that all existing
/// binary-tier callers that write `RebornThreadHarness` (no type parameter) continue
/// to compile as `RebornThreadHarness<LocalFilesystem>` without modification.
///
/// The integration tier uses `RebornThreadHarness<CompositeRootFilesystem>` via
/// `filesystem_shared_composite`, mounting the thread service directly on the
/// per-`build()` production-path composite (threads at `/tenants/{t}/users/{u}/threads`).
pub struct RebornThreadHarness<F = LocalFilesystem>
where
    F: RootFilesystem,
{
    pub scope: ThreadScope,
    pub service: Arc<FilesystemSessionThreadService<F>>,
    backend: Arc<F>,
    root: Arc<tempfile::TempDir>,
    /// Path prefix inserted before `/tenants/...` when constructing the thread
    /// scoped filesystem. Binary-tier instances use `"/engine"` (preserving the
    /// `/engine/tenants/...` layout); integration-tier instances use `""` so
    /// threads land at `/tenants/...` inside the production composite.
    root_prefix: String,
}

/// Shared methods: work for any `F: RootFilesystem`.
impl<F: RootFilesystem> RebornThreadHarness<F> {
    pub fn reopened(&self) -> Result<Self, RebornThreadHarnessError> {
        let scoped =
            scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend), &self.scope)?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope: self.scope.clone(),
            service,
            backend: Arc::clone(&self.backend),
            root: Arc::clone(&self.root),
            root_prefix: self.root_prefix.clone(),
        })
    }

    pub fn service_instance(
        &self,
    ) -> Result<FilesystemSessionThreadService<F>, RebornThreadHarnessError> {
        let scoped =
            scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend), &self.scope)?;
        Ok(FilesystemSessionThreadService::new(scoped))
    }

    pub async fn history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, RebornThreadHarnessError> {
        Ok(self
            .service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.scope.clone(),
                thread_id,
            })
            .await?
            .messages)
    }

    pub async fn assert_final_reply(
        &self,
        thread_id: ThreadId,
        text: &str,
    ) -> Result<(), RebornThreadHarnessError> {
        let history = self.history(thread_id).await?;
        let found = history
            .iter()
            .rev()
            .find(|message| {
                message.kind == ironclaw_threads::MessageKind::Assistant
                    && message.status == ironclaw_threads::MessageStatus::Finalized
            })
            .is_some_and(|message| {
                message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains(text))
            });
        if found {
            Ok(())
        } else {
            Err(RebornThreadHarnessError::MissingFinalReply(
                text.to_string(),
            ))
        }
    }
}

/// `LocalFilesystem`-specific constructors (binary-E2E tier).
impl RebornThreadHarness<LocalFilesystem> {
    /// Create a harness with a private per-call `TempDir` and a fresh
    /// `LocalFilesystem` mounted under `/engine`. Used by the binary-E2E tier.
    pub fn filesystem_temp(scope: ThreadScope) -> Result<Self, RebornThreadHarnessError> {
        let root = Arc::new(tempfile::tempdir()?);
        let backend = Arc::new(local_filesystem(root.path())?);
        Self::filesystem_shared_backend(scope, backend, root)
    }

    /// Create a harness sharing an already-constructed `LocalFilesystem` backend.
    /// `root` keeps the backing `TempDir` alive for the harness's lifetime.
    /// Uses the `/engine/tenants/...` path layout (binary-E2E convention).
    pub fn filesystem_shared_backend(
        scope: ThreadScope,
        backend: Arc<LocalFilesystem>,
        root: Arc<tempfile::TempDir>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at("/engine", Arc::clone(&backend), &scope)?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root,
            root_prefix: "/engine".to_string(),
        })
    }
}

/// `CompositeRootFilesystem`-specific constructor (integration tier).
impl RebornThreadHarness<CompositeRootFilesystem> {
    /// Create a harness backed by a shared production-path composite.
    ///
    /// Threads land at `/tenants/{tenant}/users/{user}/threads` inside the
    /// composite (no `/engine` prefix), so they are visible through the
    /// `/tenants` mount that `mount_local_dev_database_roots` installs.
    /// `root` keeps the composite's `TempDir` alive; the same `Arc` is also
    /// held by `GroupSharedStorage::turn_root` so on-disk libsql data persists
    /// across calls to `reopened()`, which rebuilds the scoped service from
    /// the same composite backend.
    pub fn filesystem_shared_composite(
        scope: ThreadScope,
        backend: Arc<CompositeRootFilesystem>,
        root: Arc<tempfile::TempDir>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at("", Arc::clone(&backend), &scope)?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root,
            root_prefix: String::new(),
        })
    }
}

/// Build the scoped thread filesystem for `scope`.
///
/// `root_prefix` is prepended before `/tenants/...`:
/// - Binary-E2E tier: `"/engine"` → `/engine/tenants/{t}/users/{u}/threads`
/// - Integration tier: `""` → `/tenants/{t}/users/{u}/threads`
fn scoped_threads_fs_at<F>(
    root_prefix: &str,
    backend: Arc<F>,
    scope: &ThreadScope,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let user_id = scope
        .owner_user_id
        .as_ref()
        .map_or("_system", |user_id| user_id.as_str());
    let target = format!(
        "{root_prefix}/tenants/{}/users/{}/threads",
        scope.tenant_id, user_id
    );
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("valid threads alias"),
        VirtualPath::new(target).expect("valid threads target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}
