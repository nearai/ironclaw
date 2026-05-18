//! Per-(tenant, user) store cache for the universal-FS dispatch composition.
//!
//! Production composition holds a single underlying [`RootFilesystem`] for the
//! whole process, but consumer-store records must land under
//! `/tenants/<tenant_id>/users/<user_id>/<alias>/…` so two tenants cannot
//! collide on identically-shaped paths (same agent, same project, same handle).
//!
//! The mount-view rewriting that places records under the per-tenant prefix
//! lives in [`crate::invocation_mount_view`]. This module provides the
//! companion runtime piece: a cache that lazily builds one `Arc<Store>` per
//! `(tenant_id, user_id)` over a per-tenant
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem), and reuses it
//! for the lifetime of the process.
//!
//! Per the design decision recorded in
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md` ("Open
//! Question 1"), the cache is **unbounded and long-lived**: a tenant's stores
//! are constructed on first request and held until process exit. The cache
//! key is `(TenantId, UserId)` — matching the per-invocation mount view's
//! rewrite prefix.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{HostApiError, ResourceScope, TenantId, UserId};

use crate::invocation_mount_view;

type TenantKey = (TenantId, UserId);

/// Lazily-built, process-lifetime cache of per-tenant `Arc<Store>` instances.
///
/// Each entry is constructed by calling the user-supplied `build` closure with
/// the `Arc<ScopedFilesystem<F>>` produced from
/// [`invocation_mount_view`]`(scope)`. Two distinct `(TenantId, UserId)` keys
/// always yield two distinct `Arc<Store>`s; identical keys reuse the same
/// `Arc`.
///
/// Read access uses a single `RwLock` read lock; cache misses fall through to
/// a write lock that runs the builder under contention. The cache is
/// optimised for the read-heavy case (every dispatched trait call is a hit
/// after the first per-tenant call).
pub(crate) struct TenantStoreCache<F, S>
where
    F: RootFilesystem,
{
    root: Arc<F>,
    cache: RwLock<HashMap<TenantKey, Arc<S>>>,
    build: Box<dyn Fn(Arc<ScopedFilesystem<F>>) -> Arc<S> + Send + Sync>,
}

impl<F, S> TenantStoreCache<F, S>
where
    F: RootFilesystem,
{
    /// Create a new cache over `root`. `build` is invoked exactly once per
    /// `(tenant_id, user_id)` to materialise the per-tenant store.
    pub fn new<B>(root: Arc<F>, build: B) -> Self
    where
        B: Fn(Arc<ScopedFilesystem<F>>) -> Arc<S> + Send + Sync + 'static,
    {
        Self {
            root,
            cache: RwLock::new(HashMap::new()),
            build: Box::new(build),
        }
    }

    /// Return the cached `Arc<S>` for `scope`'s tenant/user pair, constructing
    /// it on first miss. The build closure receives a per-tenant
    /// [`ScopedFilesystem`] whose `MountView` resolves every consumer alias
    /// to `/tenants/<tenant>/users/<user>/<alias>` on the shared root.
    pub fn for_scope(&self, scope: &ResourceScope) -> Result<Arc<S>, HostApiError> {
        let key = (scope.tenant_id.clone(), scope.user_id.clone());
        if let Some(existing) = self
            .cache
            .read()
            .expect("tenant store cache lock poisoned")
            .get(&key)
        {
            return Ok(Arc::clone(existing));
        }
        let view = invocation_mount_view(scope)?;
        let scoped = Arc::new(ScopedFilesystem::new(Arc::clone(&self.root), view));
        let store = (self.build)(scoped);
        let mut cache = self
            .cache
            .write()
            .expect("tenant store cache lock poisoned");
        // Another writer may have populated the entry between the read-lock
        // release and our write-lock acquire; honour that build and discard
        // ours. The per-store internals are stateless (`FilesystemSecretStore`
        // and siblings hold only an `Arc<ScopedFilesystem>` plus shared
        // crypto/locking statics), so the discarded build has no side effect.
        Ok(Arc::clone(cache.entry(key).or_insert(store)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        AgentId, InvocationId, MissionId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };

    use super::TenantStoreCache;

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("github").unwrap()),
            project_id: Some(ProjectId::new("p").unwrap()),
            mission_id: Some(MissionId::new("m").unwrap()),
            thread_id: Some(ThreadId::new("t").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn distinct_tenant_or_user_yields_distinct_arc() {
        // Build a cache that stores `Arc<usize>` — a stand-in for any
        // per-tenant store. Counter proves the builder runs once per key.
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_inner = Arc::clone(&counter);
        let cache: TenantStoreCache<InMemoryBackend, usize> =
            TenantStoreCache::new(Arc::new(InMemoryBackend::new()), move |_| {
                Arc::new(counter_inner.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
            });

        let alpha = cache.for_scope(&scope("tenant_a", "alice")).unwrap();
        let bravo = cache.for_scope(&scope("tenant_b", "bob")).unwrap();
        let alpha_again = cache.for_scope(&scope("tenant_a", "alice")).unwrap();

        assert!(
            !Arc::ptr_eq(&alpha, &bravo),
            "two distinct (tenant, user) keys must produce two distinct Arcs"
        );
        assert!(
            Arc::ptr_eq(&alpha, &alpha_again),
            "the same key must reuse the cached Arc"
        );
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "builder should run once per distinct key"
        );
    }
}
