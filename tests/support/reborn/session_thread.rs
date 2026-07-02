use std::sync::Arc;

use ironclaw_filesystem::{
    CompositeRootFilesystem, InMemoryBackend, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ThreadId, VirtualPath,
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RebornThreadHarnessError {
    #[error("invalid mount view: {0}")]
    MountView(#[from] ironclaw_host_api::HostApiError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("thread history does not contain final assistant reply containing {0:?}")]
    MissingFinalReply(String),
}

/// Thin harness over a `FilesystemSessionThreadService<F>` for asserting thread
/// history in integration and binary-tier tests.
///
/// The type parameter `F` defaults to `InMemoryBackend` so that all existing
/// callers that write `RebornThreadHarness` (no type parameter) continue to
/// compile as `RebornThreadHarness<InMemoryBackend>` without modification.
/// `InMemoryBackend` (not `LocalFilesystem`) is the default because it is
/// CAS-capable and models the production database-backed filesystem that these
/// stores are mounted on in real deployments; `LocalFilesystem` is a byte-only
/// backend that production never uses for record-shaped CAS stores (see
/// e3e155803).
///
/// The integration tier uses `RebornThreadHarness<CompositeRootFilesystem>` via
/// `filesystem_shared_composite`, mounting the thread service directly on the
/// per-`build()` production-path composite (threads at `/tenants/{t}/users/{u}/threads`).
pub struct RebornThreadHarness<F = InMemoryBackend>
where
    F: RootFilesystem,
{
    pub scope: ThreadScope,
    pub service: Arc<FilesystemSessionThreadService<F>>,
    backend: Arc<F>,
    /// Backing `TempDir` to keep alive for tiers whose backend persists to
    /// disk (e.g. the `CompositeRootFilesystem` integration tier). `None` for
    /// in-memory tiers, which have nothing to keep alive.
    root: Option<Arc<tempfile::TempDir>>,
    /// Path prefix inserted before `/tenants/...` when constructing the thread
    /// scoped filesystem. The default `InMemoryBackend` tier uses `"/engine"`
    /// (preserving the historical `/engine/tenants/...` layout); the
    /// integration-tier `CompositeRootFilesystem` harness uses `""` so threads
    /// land at `/tenants/...` inside the production composite.
    root_prefix: String,
}

/// Shared methods: work for any `F: RootFilesystem`.
impl<F: RootFilesystem> RebornThreadHarness<F> {
    pub fn reopened(&self) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope: self.scope.clone(),
            service,
            backend: Arc::clone(&self.backend),
            root: self.root.clone(),
            root_prefix: self.root_prefix.clone(),
        })
    }

    pub fn service_instance(
        &self,
    ) -> Result<FilesystemSessionThreadService<F>, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend))?;
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

/// `InMemoryBackend`-specific constructors (default tier). CAS-capable, models
/// the production database-backed filesystem mount — see e3e155803 for why a
/// byte-only `LocalFilesystem` is wrong for these record-shaped stores.
impl RebornThreadHarness<InMemoryBackend> {
    pub fn filesystem_temp(scope: ThreadScope) -> Result<Self, RebornThreadHarnessError> {
        let backend = Arc::new(InMemoryBackend::new());
        Self::filesystem_shared_backend(scope, backend)
    }

    pub fn filesystem_shared_backend(
        scope: ThreadScope,
        backend: Arc<InMemoryBackend>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at("/engine", Arc::clone(&backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root: None,
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
        let scoped = scoped_threads_fs_at("", Arc::clone(&backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root: Some(root),
            root_prefix: String::new(),
        })
    }
}

/// Build the scoped thread filesystem.
///
/// The `/threads` mount is resolved **per filesystem operation** from that
/// operation's own `ResourceScope` (production's `invocation_mount_view`
/// shape: `ScopedFilesystem::new` + resolver) — NOT fixed once at
/// construction. `FilesystemSessionThreadService` derives each op's
/// `ResourceScope` from the request's `ThreadScope` (owner included, via
/// `ThreadScope::to_resource_scope`), so ONE service instance serves every
/// owner's `/tenants/{t}/users/{owner}/threads` subtree. This is what lets a
/// group's ONE shared runtime resolve a second actor's thread (issue #5479):
/// the runtime's per-turn owner rewrite
/// (`ThreadScopeResolver::resolve_for_turn`) now lands on the right physical
/// root instead of a mount pinned to the group's canonical actor. For any
/// single owner the resolved path is byte-identical to the previous fixed
/// view, so single-actor tests are unaffected.
///
/// `root_prefix` is prepended before `/tenants/...`:
/// - Default `InMemoryBackend` tier: `"/engine"` → `/engine/tenants/{t}/users/{u}/threads`
/// - Integration tier: `""` → `/tenants/{t}/users/{u}/threads`
fn scoped_threads_fs_at<F>(
    root_prefix: &str,
    backend: Arc<F>,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let root_prefix = root_prefix.to_owned();
    Ok(Arc::new(ScopedFilesystem::new(backend, move |scope| {
        threads_mount_view(&root_prefix, scope)
    })))
}

/// The single `/threads` mount grant for one operation's `ResourceScope`.
///
/// System-scoped operations (e.g. `find_idempotency_record`, which routes
/// through `ResourceScope::system()`) and owner-less thread scopes carry the
/// `SYSTEM_RESERVED_ID` sentinel — control bytes, not path-safe — in the
/// tenant and/or user segment. Map it to the harness's historical `_system`
/// segment: this mirrors production's `resource_scope_path_segment`
/// (`invocation_mount_view`, ironclaw_reborn_composition) only in SHAPE
/// (sentinel-in, path-safe-segment-out) — production's actual segment value
/// is `__system__`, not `_system`. Deliberately NOT switched to match, to
/// avoid rewriting every existing harness fixture path; see `path_segment`.
pub(crate) fn threads_mount_view(
    root_prefix: &str,
    scope: &ironclaw_host_api::ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_id = path_segment(scope.tenant_id.as_str());
    let user_id = path_segment(scope.user_id.as_str());
    let target = format!("{root_prefix}/tenants/{tenant_id}/users/{user_id}/threads");
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads")?,
        VirtualPath::new(target)?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// Path-safe segment for one scope axis: the `SYSTEM_RESERVED_ID` sentinel
/// becomes the harness's historical `_system` segment (NOT production's
/// `__system__` value — see `threads_mount_view`); everything else is used
/// verbatim, matching production's `resource_scope_path_segment` shape.
pub(crate) fn path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::SYSTEM_RESERVED_ID {
        "_system"
    } else {
        value
    }
}
