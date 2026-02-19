//! LRU embedding cache wrapping any [`EmbeddingProvider`].
//!
//! Avoids redundant HTTP calls for identical texts by caching embeddings
//! in memory keyed by `SHA-256(model_name + "\0" + text)`.
//!
//! Follows the same cache pattern as `llm::response_cache::CachedProvider`:
//! `HashMap` + `last_accessed` tracking + manual LRU eviction.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::workspace::embeddings::{EmbeddingError, EmbeddingProvider};

/// Configuration for the embedding cache.
#[derive(Debug, Clone)]
pub struct EmbeddingCacheConfig {
    /// Maximum number of cached embeddings (default 10,000).
    ///
    /// Memory usage: `max_entries × dimension × 4 bytes`.
    /// At 10,000 entries × 1536 floats = ~58 MB.
    pub max_entries: usize,
}

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
        }
    }
}

struct CacheEntry {
    embedding: Vec<f32>,
    last_accessed: Instant,
}

/// Embedding provider wrapper that caches results in memory.
///
/// Thread-safe via `tokio::sync::Mutex`. The lock is **never held**
/// during HTTP calls to the inner provider.
pub struct CachedEmbeddingProvider {
    inner: Arc<dyn EmbeddingProvider>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    config: EmbeddingCacheConfig,
}

impl CachedEmbeddingProvider {
    /// Wrap a provider with LRU caching.
    pub fn new(inner: Arc<dyn EmbeddingProvider>, config: EmbeddingCacheConfig) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Number of entries currently in the cache.
    pub async fn len(&self) -> usize {
        self.cache.lock().await.len()
    }

    /// Whether the cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.cache.lock().await.is_empty()
    }

    /// Clear all cached entries.
    pub async fn clear(&self) {
        self.cache.lock().await.clear();
    }

    /// Build a deterministic cache key: `SHA-256(model_name + "\0" + text)`.
    fn cache_key(&self, text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.inner.model_name().as_bytes());
        hasher.update(b"\0");
        hasher.update(text.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Evict the least-recently-used entry if at capacity.
    fn evict_lru(cache: &mut HashMap<String, CacheEntry>, max_entries: usize) {
        while cache.len() >= max_entries {
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(k, _)| k.clone());

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
            let mut guard = self.cache.lock().await;
            if let Some(entry) = guard.get_mut(&key) {
                entry.last_accessed = Instant::now();
                tracing::debug!("embedding cache hit");
                return Ok(entry.embedding.clone());
            }
        }
        // Lock released before HTTP call

        let embedding = self.inner.embed(text).await?;

        // Store result
        {
            let mut guard = self.cache.lock().await;
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
        let keys: Vec<String> = texts.iter().map(|t| self.cache_key(t)).collect();
        let mut results: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut miss_indices: Vec<usize> = Vec::new();

        {
            let mut guard = self.cache.lock().await;
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

        // Store misses and assemble results
        {
            let mut guard = self.cache.lock().await;
            let now = Instant::now();
            for (j, &orig_idx) in miss_indices.iter().enumerate() {
                let emb = &new_embeddings[j];
                guard.insert(
                    keys[orig_idx].clone(),
                    CacheEntry {
                        embedding: emb.clone(),
                        last_accessed: now,
                    },
                );
                results[orig_idx] = Some(emb.clone());
            }
            // Enforce capacity after batch insert
            Self::evict_lru(&mut guard, self.config.max_entries + 1);
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

        let r1 = cached.embed("hello").await.unwrap();
        assert_eq!(inner.embed_calls(), 1);

        let r2 = cached.embed("hello").await.unwrap();
        assert_eq!(inner.embed_calls(), 1); // still 1 -- cache hit
        assert_eq!(r1, r2);

        assert_eq!(cached.len().await, 1);
    }

    #[tokio::test]
    async fn cache_miss_calls_inner() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        cached.embed("hello").await.unwrap();
        cached.embed("world").await.unwrap();
        assert_eq!(inner.embed_calls(), 2);
        assert_eq!(cached.len().await, 2);
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
        assert_eq!(cached.len().await, 2);

        // Third entry should evict the oldest ("first")
        cached.embed("third").await.unwrap();
        assert_eq!(cached.len().await, 2);
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
        assert_eq!(cached.len().await, 3); // all three now cached
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let inner = Arc::new(CountingMock::new(4, "test-model"));
        let cached =
            CachedEmbeddingProvider::new(inner.clone(), EmbeddingCacheConfig { max_entries: 100 });

        // Pre-cache "bb" (len 2)
        cached.embed("bb").await.unwrap();

        // Batch: "a" (miss, len 1), "bb" (hit, len 2), "ccc" (miss, len 3)
        // Different lengths ensure distinct embeddings from CountingMock.
        let texts = vec!["a".to_string(), "bb".to_string(), "ccc".to_string()];
        let results = cached.embed_batch(&texts).await.unwrap();

        assert_eq!(results.len(), 3);
        // CountingMock produces val = text.len() / 100.0, so each input
        // with a different length yields a distinct embedding.
        let expected_a = vec![1.0_f32 / 100.0; 4];
        let expected_bb = vec![2.0_f32 / 100.0; 4];
        let expected_ccc = vec![3.0_f32 / 100.0; 4];
        assert_eq!(results[0], expected_a);
        assert_eq!(results[1], expected_bb);
        assert_eq!(results[2], expected_ccc);
    }
}
