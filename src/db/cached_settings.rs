//! Write-through caching decorator for [`SettingsStore`].
//!
//! Wraps any `Arc<dyn SettingsStore>` and caches `get_all_settings()` results
//! per `user_id`. Write operations delegate to the inner store first, then
//! invalidate that user's cache entry. All callers see the same cache via
//! `Arc<CachedSettingsStore>`.
//!
//! # Design assumptions
//!
//! **Primarily single-user / small-tenant.** The cache is bounded by
//! [`DEFAULT_MAX_ENTRIES`] (default 1 000) and entries expire after
//! [`DEFAULT_TTL_SECS`] (default 300 s / 5 min). These limits prevent
//! unbounded memory growth in multi-tenant deployments while keeping the
//! implementation simple.
//!
//! **Single-process coherence only.** The cache lives in-process memory.
//! Settings changed by a separate process (e.g. `ironclaw config set` CLI,
//! direct SQL, another replica) are invisible until the cache entry expires
//! (TTL) or the process receives SIGHUP (which calls [`CachedSettingsStore::flush`]).
//!
//! **Known bypass paths.** Some subsystems still hold `Arc<dyn Database>` and
//! call `set_setting()` directly, bypassing cache invalidation.
//! `ExtensionManager` is wired with a `settings_override` to route through
//! the cache when available. `AuthManager` in `src/bridge/` extracts a raw
//! `Database` reference — its staleness is bounded by the TTL. Any new code
//! that writes settings should go through the `CachedSettingsStore` or call
//! [`CachedSettingsStore::invalidate_user`] after writing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;

use uuid::Uuid;

use crate::db::{DatabaseError, SettingRow, SettingsStore};

/// Default time-to-live for cache entries (5 minutes).
///
/// Bounds staleness from any write path that bypasses the cache (CLI,
/// direct SQL, subsystems holding raw `Arc<dyn Database>`).
const DEFAULT_TTL_SECS: u64 = 300;

/// Default maximum number of cached user entries.
///
/// When exceeded the entire cache is cleared — a simple eviction policy
/// that is acceptable because it is a rare event in practice (primarily
/// single-user deployments cache 1–2 entries).
const DEFAULT_MAX_ENTRIES: usize = 1_000;

/// A single cached entry: the settings map plus its load timestamp.
struct CacheEntry {
    settings: Arc<HashMap<String, serde_json::Value>>,
    loaded_at: Instant,
}

/// Per-user write-through cache for [`SettingsStore`].
///
/// Read-heavy methods (`get_all_settings`, `get_setting`, `has_settings`)
/// consult the cache; write methods (`set_setting`, `set_all_settings`,
/// `delete_setting`) delegate then invalidate. Metadata-bearing reads
/// (`get_setting_full`, `list_settings`) pass through to the inner store.
///
/// Entries expire after [`DEFAULT_TTL_SECS`] and the cache is bounded to
/// [`DEFAULT_MAX_ENTRIES`] user entries.
pub struct CachedSettingsStore {
    inner: Arc<dyn SettingsStore + Send + Sync>,
    /// Per-user cache: user_id -> settings + load timestamp.
    cache: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
    max_entries: usize,
}

impl CachedSettingsStore {
    pub fn new(inner: Arc<dyn SettingsStore + Send + Sync>) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(DEFAULT_TTL_SECS),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    /// Create a cache with custom TTL and capacity (for testing).
    #[cfg(test)]
    fn with_ttl_and_capacity(
        inner: Arc<dyn SettingsStore + Send + Sync>,
        ttl: Duration,
        max_entries: usize,
    ) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            ttl,
            max_entries,
        }
    }

    /// Return `true` if `entry` has not yet expired.
    fn is_fresh(&self, entry: &CacheEntry) -> bool {
        entry.loaded_at.elapsed() < self.ttl
    }

    /// Load or return cached `get_all_settings()` for a user.
    ///
    /// The write lock is held across the DB load to prevent a stale-data race
    /// where a concurrent `invalidate()` (from a settings write) clears the
    /// cache between our DB read and our cache insert, causing us to store
    /// pre-write data. Serializing loaders under the write lock eliminates
    /// this window. Acceptable for a primarily single-user system.
    async fn get_or_load(
        &self,
        user_id: &str,
    ) -> Result<Arc<HashMap<String, serde_json::Value>>, DatabaseError> {
        // Fast path: read lock, Arc clone is cheap.
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(user_id).filter(|e| self.is_fresh(e)) {
                return Ok(Arc::clone(&entry.settings));
            }
        }

        // Slow path: hold write lock across the DB load to prevent
        // loader-vs-invalidator race.
        let mut cache = self.cache.write().await;
        // Re-check: another task may have populated while we waited.
        if let Some(existing) = cache.get(user_id).filter(|e| self.is_fresh(e)) {
            return Ok(Arc::clone(&existing.settings));
        }
        let settings = Arc::new(self.inner.get_all_settings(user_id).await?);
        // Evict all entries if the cache has grown beyond the cap.
        if cache.len() >= self.max_entries {
            cache.clear();
        }
        cache.insert(
            user_id.to_owned(),
            CacheEntry {
                settings: Arc::clone(&settings),
                loaded_at: Instant::now(),
            },
        );
        Ok(settings)
    }

    /// Remove a user's cached settings entry.
    ///
    /// Called internally after write operations and externally from user
    /// delete/suspend handlers to avoid serving stale data.
    pub async fn invalidate_user(&self, user_id: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(user_id);
    }

    /// Drop all cached entries.
    ///
    /// Call from SIGHUP / config-reload handlers to ensure settings
    /// modified directly in the database (outside the application) are
    /// picked up on the next read.
    pub async fn flush(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

#[async_trait]
impl SettingsStore for CachedSettingsStore {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let all = self.get_or_load(user_id).await?;
        Ok(all.get(key).cloned())
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        // Pass through — returns metadata the cache doesn't carry.
        self.inner.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.set_setting(user_id, key, value).await?;
        self.invalidate_user(user_id).await;
        Ok(())
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        let deleted = self.inner.delete_setting(user_id, key).await?;
        self.invalidate_user(user_id).await;
        Ok(deleted)
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        // Pass through — returns metadata the cache doesn't carry.
        self.inner.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        let arc = self.get_or_load(user_id).await?;
        Ok((*arc).clone())
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.inner.set_all_settings(user_id, settings).await?;
        self.invalidate_user(user_id).await;
        Ok(())
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        let all = self.get_or_load(user_id).await?;
        Ok(!all.is_empty())
    }

    // Workspace-scoped settings: delegate directly to inner store (no caching).
    async fn get_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.inner.get_setting_for_workspace(workspace_id, key).await
    }

    async fn get_setting_full_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.inner.get_setting_full_for_workspace(workspace_id, key).await
    }

    async fn set_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.set_setting_for_workspace(workspace_id, key, value).await
    }

    async fn delete_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<bool, DatabaseError> {
        self.inner.delete_setting_for_workspace(workspace_id, key).await
    }

    async fn list_settings_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<SettingRow>, DatabaseError> {
        self.inner.list_settings_for_workspace(workspace_id).await
    }

    async fn get_all_settings_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.inner.get_all_settings_for_workspace(workspace_id).await
    }

    async fn set_all_settings_for_workspace(
        &self,
        workspace_id: Uuid,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.inner.set_all_settings_for_workspace(workspace_id, settings).await
    }

    async fn has_settings_for_workspace(&self, workspace_id: Uuid) -> Result<bool, DatabaseError> {
        self.inner.has_settings_for_workspace(workspace_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal in-memory SettingsStore that counts DB hits.
    struct CountingStore {
        data: RwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
        get_all_count: AtomicUsize,
    }

    impl CountingStore {
        fn new() -> Self {
            Self {
                data: RwLock::new(HashMap::new()),
                get_all_count: AtomicUsize::new(0),
            }
        }

        fn get_all_hits(&self) -> usize {
            self.get_all_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SettingsStore for CountingStore {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, DatabaseError> {
            let data = self.data.read().await;
            Ok(data.get(user_id).and_then(|m| m.get(key)).cloned())
        }

        async fn get_setting_full(
            &self,
            _user_id: &str,
            _key: &str,
        ) -> Result<Option<SettingRow>, DatabaseError> {
            Ok(None)
        }

        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), DatabaseError> {
            let mut data = self.data.write().await;
            data.entry(user_id.to_owned())
                .or_default()
                .insert(key.to_owned(), value.clone());
            Ok(())
        }

        async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
            let mut data = self.data.write().await;
            Ok(data.get_mut(user_id).and_then(|m| m.remove(key)).is_some())
        }

        async fn list_settings(&self, _user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
            Ok(vec![])
        }

        async fn get_all_settings(
            &self,
            user_id: &str,
        ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
            self.get_all_count.fetch_add(1, Ordering::SeqCst);
            let data = self.data.read().await;
            Ok(data.get(user_id).cloned().unwrap_or_default())
        }

        async fn set_all_settings(
            &self,
            user_id: &str,
            settings: &HashMap<String, serde_json::Value>,
        ) -> Result<(), DatabaseError> {
            let mut data = self.data.write().await;
            data.insert(user_id.to_owned(), settings.clone());
            Ok(())
        }

        async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
            let data = self.data.read().await;
            Ok(data.get(user_id).is_some_and(|m| !m.is_empty()))
        }

        async fn get_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<Option<serde_json::Value>, DatabaseError> { Ok(None) }
        async fn get_setting_full_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<Option<SettingRow>, DatabaseError> { Ok(None) }
        async fn set_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str, _value: &serde_json::Value) -> Result<(), DatabaseError> { Ok(()) }
        async fn delete_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<bool, DatabaseError> { Ok(false) }
        async fn list_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<Vec<SettingRow>, DatabaseError> { Ok(vec![]) }
        async fn get_all_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<HashMap<String, serde_json::Value>, DatabaseError> { Ok(HashMap::new()) }
        async fn set_all_settings_for_workspace(&self, _workspace_id: Uuid, _settings: &HashMap<String, serde_json::Value>) -> Result<(), DatabaseError> { Ok(()) }
        async fn has_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<bool, DatabaseError> { Ok(false) }
    }

    fn make_cached(inner: Arc<CountingStore>) -> CachedSettingsStore {
        CachedSettingsStore::new(inner as Arc<dyn SettingsStore + Send + Sync>)
    }

    #[tokio::test]
    async fn get_all_settings_caches_after_first_call() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "key", &serde_json::json!("val"))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        let r1 = cached.get_all_settings("u1").await.unwrap();
        let r2 = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(r1, r2);
        assert_eq!(inner.get_all_hits(), 1, "second call should hit cache");
    }

    #[tokio::test]
    async fn set_setting_invalidates_cache() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "k", &serde_json::json!(1))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        // Populate cache.
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 1);

        // Write through the cache.
        cached
            .set_setting("u1", "k", &serde_json::json!(2))
            .await
            .unwrap();

        // Next read must hit the inner store again.
        let settings = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);
        assert_eq!(settings.get("k"), Some(&serde_json::json!(2)));
    }

    #[tokio::test]
    async fn delete_setting_invalidates_cache() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "k", &serde_json::json!("v"))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        // Populate cache.
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 1);

        // Delete through the cache.
        cached.delete_setting("u1", "k").await.unwrap();

        // Next read must hit the inner store.
        let settings = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);
        assert!(settings.is_empty());
    }

    #[tokio::test]
    async fn users_have_independent_cache_entries() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "a", &serde_json::json!(1))
            .await
            .unwrap();
        inner
            .set_setting("u2", "b", &serde_json::json!(2))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        let s1 = cached.get_all_settings("u1").await.unwrap();
        let s2 = cached.get_all_settings("u2").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);

        assert!(s1.contains_key("a"));
        assert!(!s1.contains_key("b"));
        assert!(s2.contains_key("b"));
        assert!(!s2.contains_key("a"));

        // Invalidating u1 doesn't affect u2.
        cached
            .set_setting("u1", "a", &serde_json::json!(99))
            .await
            .unwrap();
        let _ = cached.get_all_settings("u1").await.unwrap();
        let _ = cached.get_all_settings("u2").await.unwrap();
        // u1 reloaded (hit 3), u2 still cached (no extra hit).
        assert_eq!(inner.get_all_hits(), 3);
    }

    #[tokio::test]
    async fn get_setting_uses_cached_map() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "color", &serde_json::json!("blue"))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        let val = cached.get_setting("u1", "color").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("blue")));
        assert_eq!(inner.get_all_hits(), 1);

        // Second individual get_setting should not hit inner store again.
        let val2 = cached.get_setting("u1", "color").await.unwrap();
        assert_eq!(val2, Some(serde_json::json!("blue")));
        assert_eq!(inner.get_all_hits(), 1);

        // Missing key returns None.
        let missing = cached.get_setting("u1", "nope").await.unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn set_all_settings_invalidates_cache() {
        let inner = Arc::new(CountingStore::new());
        let cached = make_cached(Arc::clone(&inner));

        // Populate cache (empty).
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 1);

        // Bulk write.
        let mut bulk = HashMap::new();
        bulk.insert("x".to_owned(), serde_json::json!(42));
        cached.set_all_settings("u1", &bulk).await.unwrap();

        // Cache invalidated — next read reloads.
        let settings = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);
        assert_eq!(settings.get("x"), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn has_settings_uses_cache() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "k", &serde_json::json!(1))
            .await
            .unwrap();

        let cached = make_cached(Arc::clone(&inner));

        assert!(cached.has_settings("u1").await.unwrap());
        assert!(!cached.has_settings("u2").await.unwrap());
        // Both loaded via get_or_load.
        assert_eq!(inner.get_all_hits(), 2);

        // Subsequent calls hit cache.
        assert!(cached.has_settings("u1").await.unwrap());
        assert_eq!(inner.get_all_hits(), 2);
    }

    // --- Error-path tests ---

    /// SettingsStore that fails on the first N calls to get_all_settings.
    struct FailingStore {
        inner: CountingStore,
        fail_remaining: std::sync::atomic::AtomicI32,
    }

    impl FailingStore {
        fn new(fail_count: i32) -> Self {
            Self {
                inner: CountingStore::new(),
                fail_remaining: std::sync::atomic::AtomicI32::new(fail_count),
            }
        }
    }

    #[async_trait]
    impl SettingsStore for FailingStore {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, DatabaseError> {
            self.inner.get_setting(user_id, key).await
        }
        async fn get_setting_full(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<SettingRow>, DatabaseError> {
            self.inner.get_setting_full(user_id, key).await
        }
        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), DatabaseError> {
            self.inner.set_setting(user_id, key, value).await
        }
        async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
            self.inner.delete_setting(user_id, key).await
        }
        async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
            self.inner.list_settings(user_id).await
        }
        async fn get_all_settings(
            &self,
            user_id: &str,
        ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
            let prev = self.fail_remaining.fetch_sub(1, Ordering::SeqCst);
            if prev > 0 {
                return Err(DatabaseError::Pool("injected failure".into()));
            }
            self.inner.get_all_settings(user_id).await
        }
        async fn set_all_settings(
            &self,
            user_id: &str,
            settings: &HashMap<String, serde_json::Value>,
        ) -> Result<(), DatabaseError> {
            self.inner.set_all_settings(user_id, settings).await
        }
        async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
            self.inner.has_settings(user_id).await
        }

        async fn get_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<Option<serde_json::Value>, DatabaseError> { Ok(None) }
        async fn get_setting_full_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<Option<SettingRow>, DatabaseError> { Ok(None) }
        async fn set_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str, _value: &serde_json::Value) -> Result<(), DatabaseError> { Ok(()) }
        async fn delete_setting_for_workspace(&self, _workspace_id: Uuid, _key: &str) -> Result<bool, DatabaseError> { Ok(false) }
        async fn list_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<Vec<SettingRow>, DatabaseError> { Ok(vec![]) }
        async fn get_all_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<HashMap<String, serde_json::Value>, DatabaseError> { Ok(HashMap::new()) }
        async fn set_all_settings_for_workspace(&self, _workspace_id: Uuid, _settings: &HashMap<String, serde_json::Value>) -> Result<(), DatabaseError> { Ok(()) }
        async fn has_settings_for_workspace(&self, _workspace_id: Uuid) -> Result<bool, DatabaseError> { Ok(false) }
    }

    #[tokio::test]
    async fn inner_error_propagates_and_cache_stays_clean() {
        let inner = Arc::new(FailingStore::new(1));
        inner
            .set_setting("u1", "k", &serde_json::json!(1))
            .await
            .unwrap();

        let cached = CachedSettingsStore::new(inner as Arc<dyn SettingsStore + Send + Sync>);

        // First call fails — error propagates.
        let err = cached.get_all_settings("u1").await;
        assert!(err.is_err());

        // Cache was not poisoned — second call succeeds (fail_remaining exhausted).
        let ok = cached.get_all_settings("u1").await;
        assert!(ok.is_ok());
        assert_eq!(ok.unwrap().get("k"), Some(&serde_json::json!(1)));
    }

    // --- Concurrency test ---

    #[tokio::test]
    async fn concurrent_reads_return_consistent_data() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "x", &serde_json::json!(42))
            .await
            .unwrap();

        let cached = Arc::new(make_cached(Arc::clone(&inner)));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = Arc::clone(&cached);
                tokio::spawn(async move { c.get_all_settings("u1").await })
            })
            .collect();

        for h in handles {
            let result = h.await.unwrap().unwrap();
            assert_eq!(result.get("x"), Some(&serde_json::json!(42)));
        }

        // All reads should return the correct value. With the write lock held
        // across load, the inner store is hit exactly once.
        assert_eq!(inner.get_all_hits(), 1);
    }

    // --- TTL tests ---

    #[tokio::test]
    async fn expired_entry_triggers_reload() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "k", &serde_json::json!("v"))
            .await
            .unwrap();

        // TTL of zero means every entry is immediately expired.
        let cached = CachedSettingsStore::with_ttl_and_capacity(
            Arc::clone(&inner) as Arc<dyn SettingsStore + Send + Sync>,
            Duration::from_secs(0),
            DEFAULT_MAX_ENTRIES,
        );

        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 1);

        // Second read must reload because the entry has expired.
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);
    }

    #[tokio::test]
    async fn fresh_entry_does_not_reload() {
        let inner = Arc::new(CountingStore::new());
        inner
            .set_setting("u1", "k", &serde_json::json!("v"))
            .await
            .unwrap();

        // Large TTL — entries should stay fresh.
        let cached = CachedSettingsStore::with_ttl_and_capacity(
            Arc::clone(&inner) as Arc<dyn SettingsStore + Send + Sync>,
            Duration::from_secs(3600),
            DEFAULT_MAX_ENTRIES,
        );

        let _ = cached.get_all_settings("u1").await.unwrap();
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 1, "fresh entry should not reload");
    }

    // --- Max-entries tests ---

    #[tokio::test]
    async fn max_entries_cap_triggers_eviction() {
        let inner = Arc::new(CountingStore::new());
        // Cap at 2 entries.
        let cached = CachedSettingsStore::with_ttl_and_capacity(
            Arc::clone(&inner) as Arc<dyn SettingsStore + Send + Sync>,
            Duration::from_secs(3600),
            2,
        );

        // Load 2 users — fits within cap.
        let _ = cached.get_all_settings("u1").await.unwrap();
        let _ = cached.get_all_settings("u2").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2);

        // Both are cached.
        let _ = cached.get_all_settings("u1").await.unwrap();
        let _ = cached.get_all_settings("u2").await.unwrap();
        assert_eq!(inner.get_all_hits(), 2, "should still be cached");

        // Loading a 3rd user exceeds the cap — cache is cleared first.
        let _ = cached.get_all_settings("u3").await.unwrap();
        assert_eq!(inner.get_all_hits(), 3);

        // u1 and u2 were evicted — must reload.
        let _ = cached.get_all_settings("u1").await.unwrap();
        assert_eq!(inner.get_all_hits(), 4, "u1 should have been evicted");
    }
}
