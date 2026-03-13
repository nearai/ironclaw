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
    /// is higher due to HashMap, String keys, and per-entry overhead).
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
    embedding: Vec<f32>,
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
            cache: Mutex::new(HashMap::with_capacity(config.max_entries)),
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

    /// Evict the least-recently-used entry if at capacity.
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
                tracing::debug!("embedding cache hit");
                return Ok(entry.embedding.clone());
            }
        }
        // Lock released before HTTP call.
        // NOTE: Thundering herd — multiple concurrent callers with the same
        // uncached key will each call the inner provider. This is acceptable:
        // embeddings are idempotent and the last writer wins in the HashMap.

        let embedding = self.inner.embed(text).await?;

        // Store result
        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            Self::evict_lru(&mut guard, self.config.max_entries);
            guard.insert(
                key,
                CacheEntry {
                    embedding: embedding.clone(),
                    last_accessed: Instant::now(),
                },
            );
        }

        tracing::debug!("embedding cache miss");
        Ok(embedding)
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
                    results[i] = Some(entry.embedding.clone());
                } else {
                    miss_indices.push(i);
                }
            }
        }
        // Lock released before HTTP call

        if miss_indices.is_empty() {
            tracing::debug!(count = texts.len(), "embedding batch: all cache hits");
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
        let new_embeddings = self.inner.embed_batch(&miss_texts).await?;

        if new_embeddings.len() != miss_indices.len() {
            return Err(EmbeddingError::InvalidResponse(format!(
                "embed_batch returned {} embeddings, expected {}",
                new_embeddings.len(),
                miss_indices.len()
            )));
        }

        tracing::debug!(
            hits = texts.len() - miss_indices.len(),
            misses = miss_indices.len(),
            "embedding batch: partial cache"
        );

        // Store misses and assemble results.
        // Evict before each insert to keep peak memory bounded.
        {
            let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            for (orig_idx, emb) in miss_indices.iter().copied().zip(new_embeddings) {
                Self::evict_lru(&mut guard, self.config.max_entries);
                guard.insert(
                    keys[orig_idx],
                    CacheEntry {
                        embedding: emb.clone(),
                        last_accessed: now,
                    },
                );
                results[orig_idx] = Some(emb);
            }
        }

        Ok(results
            .into_iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.ok_or_else(|| {
                    EmbeddingError::InvalidResponse(format!("embedding slot {i} was not populated"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?)
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

        let r1 = cached.embed("hello").await.unwrap(); // safety: test
        assert_eq!(inner.embed_calls(), 1); // safety: test

        let r2 = cached.embed("hello").await.unwrap(); // safety: test
        assert_eq!(inner.embed_calls(), 1); // still 1 -- cache hit // safety: test
        assert_eq!(r1, r2); // safety: test

        assert_eq!(cached.len(), 1); // safety: test
    }

    #[tokio::test]
    async fn cache_miss_calls_inner() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        cached.embed("hello").await.unwrap(); // safety: test
        cached.embed("world").await.unwrap(); // safety: test
        assert_eq!(inner.embed_calls(), 2); // safety: test
        assert_eq!(cached.len(), 2); // safety: test
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
        assert_ne!(key_a, key_b); // safety: test
    }

    #[tokio::test]
    async fn lru_eviction() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 2 });

        cached.embed("first").await.unwrap(); // safety: test
        cached.embed("second").await.unwrap(); // safety: test
        assert_eq!(cached.len(), 2); // safety: test

        // Third entry should evict the oldest ("first")
        cached.embed("third").await.unwrap(); // safety: test
        assert_eq!(cached.len(), 2); // safety: test
        assert_eq!(inner.embed_calls(), 3); // safety: test

        // "first" should be a cache miss now
        cached.embed("first").await.unwrap(); // safety: test
        assert_eq!(inner.embed_calls(), 4); // safety: test
    }

    #[tokio::test]
    async fn embed_batch_partial_hits() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Pre-cache one text
        cached.embed("cached").await.unwrap(); // safety: test
        assert_eq!(inner.embed_calls(), 1); // safety: test

        // Batch with 1 cached + 2 new
        let texts = vec![
            "cached".to_string(),
            "new_one".to_string(),
            "new_two".to_string(),
        ];
        let results = cached.embed_batch(&texts).await.unwrap(); // safety: test

        // Should have called embed_batch on inner for 2 misses
        assert_eq!(inner.batch_calls(), 1); // safety: test
        assert_eq!(results.len(), 3); // safety: test
        assert_eq!(cached.len(), 3); // all three now cached // safety: test
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Pre-cache "bb" (len 2)
        cached.embed("bb").await.unwrap(); // safety: test

        // Batch: "a" (miss, len 1), "bb" (hit, len 2), "ccc" (miss, len 3)
        // Different lengths ensure distinct embeddings from CountingMock.
        let texts = vec!["a".to_string(), "bb".to_string(), "ccc".to_string()];
        let results = cached.embed_batch(&texts).await.unwrap(); // safety: test

        assert_eq!(results.len(), 3); // safety: test
        // CountingMock produces val = text.len() / 100.0, so each input
        // with a different length yields a distinct embedding.
        let expected_a = vec![1.0_f32 / 100.0; 4];
        let expected_bb = vec![2.0_f32 / 100.0; 4];
        let expected_ccc = vec![3.0_f32 / 100.0; 4];
        assert_eq!(results[0], expected_a); // safety: test
        assert_eq!(results[1], expected_bb); // safety: test
        assert_eq!(results[2], expected_ccc); // safety: test
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
            let prev = self.remaining_failures.load(Ordering::SeqCst);
            if prev > 0 {
                self.remaining_failures.store(prev - 1, Ordering::SeqCst);
                return Err(EmbeddingError::HttpError("simulated failure".to_string()));
            }
            let val = text.len() as f32 / 100.0;
            Ok(vec![val; self.dimension])
        }
        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            let prev = self.remaining_failures.load(Ordering::SeqCst);
            if prev > 0 {
                self.remaining_failures.store(prev - 1, Ordering::SeqCst);
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
        assert!(err.is_err()); // safety: test
        assert!(cached.is_empty(), "cache should be empty after error"); // safety: test

        // Second call succeeds and should call the inner provider (not serve stale error)
        let result = cached.embed("hello").await;
        assert!(result.is_ok()); // safety: test
        assert_eq!(cached.len(), 1); // safety: test
    }

    #[tokio::test]
    async fn embed_batch_empty_input() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        let results = cached.embed_batch(&[]).await.unwrap(); // safety: test
        assert!(results.is_empty()); // safety: test
        // Inner provider should not have been called
        assert_eq!(inner.batch_calls(), 0); // safety: test
    }

    #[tokio::test]
    async fn embed_batch_all_misses() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Nothing cached — every text is a miss
        let texts: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let results = cached.embed_batch(&texts).await.unwrap(); // safety: test
        assert_eq!(results.len(), 3); // safety: test
        assert_eq!(inner.batch_calls(), 1, "inner called once for misses"); // safety: test
        assert_eq!(cached.len(), 3, "all results should be cached"); // safety: test

        // Second call should be all hits — no new inner calls
        let results2 = cached.embed_batch(&texts).await.unwrap(); // safety: test
        assert_eq!(results2.len(), 3); // safety: test
        assert_eq!(inner.batch_calls(), 1, "no new inner calls"); // safety: test
    }

    #[tokio::test]
    async fn zero_max_entries_clamped_to_one() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 0 });

        // Should behave as max_entries=1 (clamped in constructor)
        cached.embed("hello").await.unwrap(); // safety: test
        assert_eq!(cached.len(), 1); // safety: test

        // Second entry evicts the first
        cached.embed("world").await.unwrap(); // safety: test
        assert_eq!(cached.len(), 1); // safety: test
        assert_eq!(inner.embed_calls(), 2); // safety: test
    }
}
