//! LRU embedding cache wrapping any [`EmbeddingProvider`].
//!
//! Avoids redundant HTTP calls for identical texts by caching embeddings
//! in memory keyed by `SHA-256(model_name + "\0" + text)`.
//!
//! Follows the same cache pattern as `llm::response_cache::CachedProvider`:
//! `HashMap` + `last_accessed` tracking + manual LRU eviction.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::workspace::embeddings::{EmbeddingError, EmbeddingProvider};

/// Configuration for the embedding cache.
#[derive(Debug, Clone)]
pub struct EmbeddingCacheConfig {
    /// Maximum number of cached embeddings (default 10,000).
    ///
    /// Approximate raw embedding payload: `max_entries × dimension × 4 bytes`.
    /// At 10,000 entries × 1536 floats ≈ 58 MB (payload only; actual memory
    /// is higher due to HashMap buckets, `[u8; 32]` hash keys, `Vec`/`Instant`
    /// per-entry overhead).
    pub max_entries: usize,
}

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: crate::config::DEFAULT_EMBEDDING_CACHE_SIZE,
        }
    }
}

struct CacheEntry {
    /// Stored as `Arc` so that cache insertions (miss path) can share the
    /// allocation with the return value instead of cloning the entire vector.
    embedding: Arc<Vec<f32>>,
    last_accessed: Instant,
}

/// Embedding provider wrapper that caches results in memory.
///
/// Thread-safe via `std::sync::Mutex`. The lock is **never held**
/// across `.await` points (all critical sections are scoped blocks),
/// so a synchronous mutex is cheaper than `tokio::sync::Mutex`.
pub struct CachedEmbeddingProvider {
    inner: Arc<dyn EmbeddingProvider>,
    cache: Mutex<HashMap<[u8; 32], CacheEntry>>,
    config: EmbeddingCacheConfig,
}

impl CachedEmbeddingProvider {
    /// Wrap a provider with LRU caching.
    ///
    /// `config.max_entries` is clamped to at least 1.
    pub fn new(inner: Arc<dyn EmbeddingProvider>, config: EmbeddingCacheConfig) -> Self {
        let config = EmbeddingCacheConfig {
            max_entries: config.max_entries.max(1),
        };
        if config.max_entries > 100_000 {
            tracing::warn!(
                max_entries = config.max_entries,
                "Embedding cache size exceeds 100,000 entries; memory usage may be significant"
            );
        }
        Self {
            inner,
            cache: Mutex::new(HashMap::with_capacity(config.max_entries.min(1024))),
            config,
        }
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Build a deterministic cache key: `SHA-256(model_name + "\0" + text)`.
    ///
    /// Returns raw 32-byte hash to avoid a 64-char hex String allocation per lookup.
    fn cache_key(&self, text: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.inner.model_name().as_bytes());
        hasher.update(b"\0");
        hasher.update(text.as_bytes());
        hasher.finalize().into()
    }

    /// Evict the least-recently-used entry if at capacity (single-entry path).
    // TODO: O(n) scan per eviction. If max_entries grows large, switch to
    // an ordered data structure (e.g. `IndexMap` with swap_remove, or a
    // linked-list LRU like the `lru` crate).
    fn evict_lru(cache: &mut HashMap<[u8; 32], CacheEntry>, max_entries: usize) {
        while cache.len() >= max_entries {
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(k, _)| *k);

            if let Some(k) = oldest_key {
                cache.remove(&k);
            } else {
                break;
            }
        }
    }

    /// Evict the `k` oldest entries in O(n) average time via partial selection.
    ///
    /// Used by `embed_batch` to avoid the O(n×m) cost of calling
    /// `evict_lru` per insert.
    fn evict_k_oldest(cache: &mut HashMap<[u8; 32], CacheEntry>, k: usize) {
        if k == 0 || cache.is_empty() {
            return;
        }
        if k >= cache.len() {
            cache.clear();
            return;
        }
        // Partial selection: find the k oldest in O(n) average via
        // select_nth_unstable_by_key, then remove the first k entries.
        let mut entries: Vec<([u8; 32], Instant)> = cache
            .iter()
            .map(|(key, entry)| (*key, entry.last_accessed))
            .collect();
        entries.select_nth_unstable_by_key(k - 1, |(_, t)| *t);
        for (key, _) in entries.into_iter().take(k) {
            cache.remove(&key);
        }
    }
}

#[async_trait]
impl EmbeddingProvider for CachedEmbeddingProvider {
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn max_input_length(&self) -> usize {
        self.inner.max_input_length()
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let key = self.cache_key(text);

        // Check cache (short critical section)
        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = guard.get_mut(&key) {
                entry.last_accessed = Instant::now();
                tracing::trace!("embedding cache hit");
                return Ok((*entry.embedding).clone());
            }
        }
        // Lock released before HTTP call.
        // NOTE: Thundering herd — multiple concurrent callers with the same
        // uncached key will each call the inner provider. This is acceptable:
        // embeddings are idempotent and the last writer wins in the HashMap.

        let embedding = Arc::new(self.inner.embed(text).await?);

        // Store result. Re-check under lock: another concurrent caller may
        // have inserted this key while the lock was released for the HTTP call.
        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = guard.get_mut(&key) {
                // Key already present (thundering herd) — just update, no eviction needed.
                entry.embedding = Arc::clone(&embedding);
                entry.last_accessed = Instant::now();
            } else {
                Self::evict_lru(&mut guard, self.config.max_entries);
                guard.insert(
                    key,
                    CacheEntry {
                        embedding: Arc::clone(&embedding),
                        last_accessed: Instant::now(),
                    },
                );
            }
        }

        tracing::trace!("embedding cache miss");
        // Unwrap the Arc if we're the sole owner (avoids a clone on the miss path).
        Ok(Arc::try_unwrap(embedding).unwrap_or_else(|arc| (*arc).clone()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Partition into hits and misses
        let keys: Vec<[u8; 32]> = texts.iter().map(|t| self.cache_key(t)).collect();
        let mut results: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut miss_indices: Vec<usize> = Vec::new();

        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            for (i, key) in keys.iter().enumerate() {
                if let Some(entry) = guard.get_mut(key) {
                    entry.last_accessed = now;
                    results[i] = Some((*entry.embedding).clone());
                } else {
                    miss_indices.push(i);
                }
            }
        }
        // Lock released before HTTP call

        if miss_indices.is_empty() {
            tracing::trace!(count = texts.len(), "embedding batch: all cache hits");
            // All slots populated from cache hits
            return results
                .into_iter()
                .enumerate()
                .map(|(i, slot)| {
                    slot.ok_or_else(|| {
                        EmbeddingError::InvalidResponse(format!(
                            "embedding slot {i} was not populated"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>();
        }

        // Fetch missing embeddings
        let miss_texts: Vec<String> = miss_indices.iter().map(|&i| texts[i].clone()).collect();
        let new_embeddings: Vec<Arc<Vec<f32>>> = self
            .inner
            .embed_batch(&miss_texts)
            .await?
            .into_iter()
            .map(Arc::new)
            .collect();

        if new_embeddings.len() != miss_indices.len() {
            return Err(EmbeddingError::InvalidResponse(format!(
                "embed_batch returned {} embeddings, expected {}",
                new_embeddings.len(),
                miss_indices.len()
            )));
        }

        tracing::trace!(
            hits = texts.len() - miss_indices.len(),
            misses = miss_indices.len(),
            "embedding batch: partial cache"
        );

        // Cache the new embeddings, respecting max_entries.
        // Done BEFORE assembling results so that Arc::try_unwrap can
        // avoid a clone when the cache doesn't keep a copy.
        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let cacheable = miss_indices.len().min(self.config.max_entries);
            let skip = miss_indices.len() - cacheable;
            let need_to_evict = (guard.len() + cacheable).saturating_sub(self.config.max_entries);
            if need_to_evict > 0 {
                Self::evict_k_oldest(&mut guard, need_to_evict);
            }
            let now = Instant::now();
            for (&orig_idx, emb) in miss_indices[skip..].iter().zip(&new_embeddings[skip..]) {
                guard.insert(
                    keys[orig_idx],
                    CacheEntry {
                        embedding: Arc::clone(emb),
                        last_accessed: now,
                    },
                );
            }
        }

        // Assemble results: try_unwrap avoids a clone when the Arc is sole-owned
        // (i.e., when the embedding was skipped during caching due to capacity).
        for (orig_idx, emb) in miss_indices.iter().copied().zip(new_embeddings) {
            results[orig_idx] = Some(Arc::try_unwrap(emb).unwrap_or_else(|arc| (*arc).clone()));
        }

        results
            .into_iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.ok_or_else(|| {
                    EmbeddingError::InvalidResponse(format!("embedding slot {i} was not populated"))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock embedding provider that counts calls.
    struct CountingMock {
        dimension: usize,
        model: String,
        embed_calls: AtomicU32,
        batch_calls: AtomicU32,
    }

    impl CountingMock {
        fn new(dimension: usize, model: &str) -> Self {
            Self {
                dimension,
                model: model.to_string(),
                embed_calls: AtomicU32::new(0),
                batch_calls: AtomicU32::new(0),
            }
        }

        fn embed_calls(&self) -> u32 {
            self.embed_calls.load(Ordering::SeqCst)
        }

        fn batch_calls(&self) -> u32 {
            self.batch_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingProvider for CountingMock {
        fn dimension(&self) -> usize {
            self.dimension
        }
        fn model_name(&self) -> &str {
            &self.model
        }
        fn max_input_length(&self) -> usize {
            10_000
        }
        async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            self.embed_calls.fetch_add(1, Ordering::SeqCst);
            // Simple deterministic embedding: val = text.len() / 100.0
            let val = text.len() as f32 / 100.0;
            Ok(vec![val; self.dimension])
        }
        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            texts
                .iter()
                .map(|t| {
                    let val = t.len() as f32 / 100.0;
                    Ok(vec![val; self.dimension])
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn cache_hit_avoids_inner_call() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        let r1 = cached.embed("hello").await.unwrap();
        assert_eq!(inner.embed_calls(), 1);

        let r2 = cached.embed("hello").await.unwrap();
        assert_eq!(inner.embed_calls(), 1); // still 1 -- cache hit
        assert_eq!(r1, r2);

        assert_eq!(cached.len(), 1);
    }

    #[tokio::test]
    async fn cache_miss_calls_inner() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        cached.embed("hello").await.unwrap();
        cached.embed("world").await.unwrap();
        assert_eq!(inner.embed_calls(), 2);
        assert_eq!(cached.len(), 2);
    }

    #[tokio::test]
    async fn cache_key_includes_model() {
        let inner_a = Arc::new(CountingMock::new(4, "model-a"));
        let inner_b = Arc::new(CountingMock::new(4, "model-b"));

        let cached_a = CachedEmbeddingProvider::new(
            inner_a.clone(),
            EmbeddingCacheConfig { max_entries: 100 },
        );
        let cached_b = CachedEmbeddingProvider::new(
            inner_b.clone(),
            EmbeddingCacheConfig { max_entries: 100 },
        );

        // Same text, different models -> different cache keys
        let key_a = cached_a.cache_key("hello");
        let key_b = cached_b.cache_key("hello");
        assert_ne!(key_a, key_b);
    }

    #[tokio::test]
    async fn lru_eviction() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 2 });

        cached.embed("first").await.unwrap();
        cached.embed("second").await.unwrap();
        assert_eq!(cached.len(), 2);

        // Third entry should evict the oldest ("first")
        cached.embed("third").await.unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(inner.embed_calls(), 3);

        // "first" should be a cache miss now
        cached.embed("first").await.unwrap();
        assert_eq!(inner.embed_calls(), 4);
    }

    #[tokio::test]
    async fn embed_batch_partial_hits() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Pre-cache one text
        cached.embed("cached").await.unwrap();
        assert_eq!(inner.embed_calls(), 1);

        // Batch with 1 cached + 2 new
        let texts = vec![
            "cached".to_string(),
            "new_one".to_string(),
            "new_two".to_string(),
        ];
        let results = cached.embed_batch(&texts).await.unwrap();

        // Should have called embed_batch on inner for 2 misses
        assert_eq!(inner.batch_calls(), 1);
        assert_eq!(results.len(), 3);
        assert_eq!(cached.len(), 3);
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Pre-cache "bb" (len 2)
        cached.embed("bb").await.unwrap();

        // Batch: "a" (miss, len 1), "bb" (hit, len 2), "ccc" (miss, len 3)
        let texts = vec!["a".to_string(), "bb".to_string(), "ccc".to_string()];
        let results = cached.embed_batch(&texts).await.unwrap();

        assert_eq!(results.len(), 3);
        let expected_a = vec![1.0_f32 / 100.0; 4];
        let expected_bb = vec![2.0_f32 / 100.0; 4];
        let expected_ccc = vec![3.0_f32 / 100.0; 4];
        assert_eq!(results[0], expected_a);
        assert_eq!(results[1], expected_bb);
        assert_eq!(results[2], expected_ccc);
    }

    #[tokio::test]
    async fn batch_exceeding_capacity_respects_max_entries() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 3 });

        // Batch with 5 misses but cache capacity is 3
        let texts: Vec<String> = (0..5).map(|i| format!("text_{i}")).collect();
        let results = cached.embed_batch(&texts).await.unwrap();

        assert_eq!(results.len(), 5);
        let len = cached.len();
        assert!(len <= 3, "cache len {len} exceeds max 3");
    }

    /// Mock embedding provider that fails the first N calls, then succeeds.
    struct FailThenSucceedMock {
        dimension: usize,
        model: String,
        remaining_failures: AtomicU32,
    }

    impl FailThenSucceedMock {
        fn new(dimension: usize, fail_count: u32) -> Self {
            Self {
                dimension,
                model: "fail-mock".to_string(),
                remaining_failures: AtomicU32::new(fail_count),
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for FailThenSucceedMock {
        fn dimension(&self) -> usize {
            self.dimension
        }
        fn model_name(&self) -> &str {
            &self.model
        }
        fn max_input_length(&self) -> usize {
            10_000
        }
        async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            let prev =
                self.remaining_failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                        if v > 0 { Some(v - 1) } else { None }
                    });
            if prev.is_ok() {
                return Err(EmbeddingError::HttpError("simulated failure".to_string()));
            }
            let val = text.len() as f32 / 100.0;
            Ok(vec![val; self.dimension])
        }
        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            let prev =
                self.remaining_failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                        if v > 0 { Some(v - 1) } else { None }
                    });
            if prev.is_ok() {
                return Err(EmbeddingError::HttpError("simulated failure".to_string()));
            }
            texts
                .iter()
                .map(|t| {
                    let val = t.len() as f32 / 100.0;
                    Ok(vec![val; self.dimension])
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn error_does_not_pollute_cache() {
        let inner = Arc::new(FailThenSucceedMock::new(4, 1));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // First call fails
        let err = cached.embed("hello").await;
        assert!(err.is_err());
        assert!(cached.is_empty(), "cache should be empty after error");

        // Second call succeeds and should call the inner provider (not serve stale error)
        let result = cached.embed("hello").await;
        assert!(result.is_ok());
        assert_eq!(cached.len(), 1);
    }

    #[tokio::test]
    async fn embed_batch_empty_input() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        let results = cached.embed_batch(&[]).await.unwrap();
        assert!(results.is_empty());
        assert_eq!(inner.batch_calls(), 0);
    }

    #[tokio::test]
    async fn embed_batch_all_misses() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Nothing cached — every text is a miss
        let texts: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let results = cached.embed_batch(&texts).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(inner.batch_calls(), 1, "inner called once for misses");
        assert_eq!(cached.len(), 3, "all results should be cached");

        // Second call should be all hits — no new inner calls
        let results2 = cached.embed_batch(&texts).await.unwrap();
        assert_eq!(results2.len(), 3);
        assert_eq!(inner.batch_calls(), 1, "no new inner calls");
    }

    #[tokio::test]
    async fn zero_max_entries_clamped_to_one() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 0 });

        // Should behave as max_entries=1 (clamped in constructor)
        cached.embed("hello").await.unwrap();
        assert_eq!(cached.len(), 1);

        // Second entry evicts the first
        cached.embed("world").await.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(inner.embed_calls(), 2);
    }
}
