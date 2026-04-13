//! Write-through caching decorator for [`SettingsStore`].
//!
//! Wraps any `Arc<dyn SettingsStore>` and caches `get_all_settings()` results
//! per `user_id`. Write operations delegate to the inner store first, then
//! invalidate that user's cache entry. All callers see the same cache via
//! `Arc<CachedSettingsStore>`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::{DatabaseError, SettingRow, SettingsStore};

/// Per-user write-through cache for [`SettingsStore`].
///
/// Read-heavy methods (`get_all_settings`, `get_setting`, `has_settings`)
/// consult the cache; write methods (`set_setting`, `set_all_settings`,
/// `delete_setting`) delegate then invalidate. Metadata-bearing reads
/// (`get_setting_full`, `list_settings`) pass through to the inner store.
pub struct CachedSettingsStore {
    inner: Arc<dyn SettingsStore + Send + Sync>,
    /// Per-user cache: user_id -> full settings map.
    cache: RwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
}

impl CachedSettingsStore {
    pub fn new(inner: Arc<dyn SettingsStore + Send + Sync>) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
        }
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
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        // Fast path: read lock.
        {
            let cache = self.cache.read().await;
            if let Some(settings) = cache.get(user_id) {
                return Ok(settings.clone());
            }
        }

        // Slow path: hold write lock across the DB load to prevent
        // loader-vs-invalidator race.
        let mut cache = self.cache.write().await;
        // Re-check: another task may have populated while we waited.
        if let Some(existing) = cache.get(user_id) {
            return Ok(existing.clone());
        }
        let settings = self.inner.get_all_settings(user_id).await?;
        cache.insert(user_id.to_owned(), settings.clone());
        Ok(settings)
    }

    /// Remove a user's entry from the cache.
    async fn invalidate(&self, user_id: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(user_id);
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
        self.invalidate(user_id).await;
        Ok(())
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        let deleted = self.inner.delete_setting(user_id, key).await?;
        self.invalidate(user_id).await;
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
        self.get_or_load(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.inner.set_all_settings(user_id, settings).await?;
        self.invalidate(user_id).await;
        Ok(())
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        let all = self.get_or_load(user_id).await?;
        Ok(!all.is_empty())
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
}
