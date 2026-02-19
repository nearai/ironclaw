//! In-memory idempotency cache for tool executions.
//!
//! For tools that declare `is_idempotent() == true`, successful results are
//! cached per-job so repeated identical calls (common during self-repair
//! recovery, stuck job retries, or chat-mode retry loops) return instantly
//! without re-executing.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │                  ToolIdempotencyCache                     │
//! │                                                          │
//! │  get(job_id, tool_name, args) -> Option<ToolOutput>      │
//! │  put(job_id, tool_name, args, output)                    │
//! │  invalidate_job(job_id)  // cleanup on job completion    │
//! │                                                          │
//! │  Internal: Mutex<HashMap<(Uuid, CacheKey), CacheEntry>>  │
//! │  Key: sha256(tool_name | canonical_json(args))           │
//! │  Scoped by job_id                                        │
//! │  TTL + max entries per job with LRU eviction             │
//! └──────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::tools::ToolOutput;

/// Configuration for the idempotency cache.
#[derive(Debug, Clone)]
pub struct IdempotencyCacheConfig {
    /// Time-to-live for cache entries.
    pub ttl: Duration,
    /// Maximum number of cached entries per job before LRU eviction.
    pub max_entries_per_job: usize,
}

impl Default for IdempotencyCacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(30 * 60), // 30 minutes
            max_entries_per_job: 500,
        }
    }
}

/// SHA-256 hex digest used as cache key.
type CacheKey = String;

struct CacheEntry {
    output: ToolOutput,
    created_at: Instant,
    last_accessed: Instant,
}

/// Per-job idempotency cache for tool results.
///
/// Only caches `Ok(ToolOutput)` results. Errors are never cached so retries
/// after transient failures get a fresh execution.
pub struct ToolIdempotencyCache {
    /// Map from (job_id, cache_key) -> cached result.
    entries: Mutex<HashMap<(Uuid, CacheKey), CacheEntry>>,
    config: IdempotencyCacheConfig,
}

impl ToolIdempotencyCache {
    /// Create a new cache with the given configuration.
    pub fn new(config: IdempotencyCacheConfig) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Build a deterministic cache key from a tool name and its arguments.
    ///
    /// `sha256(tool_name | canonical_json(args))`. serde_json produces
    /// stable output for the same input structure.
    fn cache_key(tool_name: &str, args: &serde_json::Value) -> CacheKey {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        if let Ok(json) = serde_json::to_string(args) {
            hasher.update(json.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    /// Look up a cached result for a tool invocation within a job.
    ///
    /// Returns `Some(ToolOutput)` on a cache hit (within TTL), `None` on miss.
    pub async fn get(
        &self,
        job_id: Uuid,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<ToolOutput> {
        let key = Self::cache_key(tool_name, args);
        let now = Instant::now();

        let mut guard = self.entries.lock().await;
        let compound_key = (job_id, key);

        if let Some(entry) = guard.get_mut(&compound_key) {
            if now.duration_since(entry.created_at) < self.config.ttl {
                entry.last_accessed = now;
                tracing::debug!(
                    tool = %tool_name,
                    job = %job_id,
                    "idempotency cache hit"
                );
                return Some(entry.output.clone());
            }
            // Expired
            guard.remove(&compound_key);
        }

        tracing::trace!(
            tool = %tool_name,
            job = %job_id,
            "idempotency cache miss"
        );
        None
    }

    /// Store a successful tool result in the cache.
    ///
    /// Evicts expired entries and applies LRU eviction if the per-job limit
    /// is exceeded.
    pub async fn put(
        &self,
        job_id: Uuid,
        tool_name: &str,
        args: &serde_json::Value,
        output: ToolOutput,
    ) {
        let key = Self::cache_key(tool_name, args);
        let now = Instant::now();

        let mut guard = self.entries.lock().await;

        // Evict expired entries for this job
        guard.retain(|(jid, _), entry| {
            *jid != job_id || now.duration_since(entry.created_at) < self.config.ttl
        });

        // Count entries for this job and evict LRU if over capacity
        let job_count = guard.keys().filter(|(jid, _)| *jid == job_id).count();
        if job_count >= self.config.max_entries_per_job {
            // Find the LRU entry for this job
            let oldest_key = guard
                .iter()
                .filter(|((jid, _), _)| *jid == job_id)
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(k, _)| k.clone());

            if let Some(k) = oldest_key {
                guard.remove(&k);
            }
        }

        tracing::trace!(
            tool = %tool_name,
            job = %job_id,
            "idempotency cache store"
        );

        guard.insert(
            (job_id, key),
            CacheEntry {
                output,
                created_at: now,
                last_accessed: now,
            },
        );
    }

    /// Remove all cached entries for a job.
    ///
    /// Call this when a job completes or fails to free memory.
    pub async fn invalidate_job(&self, job_id: Uuid) {
        let mut guard = self.entries.lock().await;
        let before = guard.len();
        guard.retain(|(jid, _), _| *jid != job_id);
        let removed = before - guard.len();
        if removed > 0 {
            tracing::debug!(
                job = %job_id,
                removed,
                "idempotency cache invalidated job"
            );
        }
    }

    /// Total number of entries across all jobs.
    #[cfg(test)]
    async fn len(&self) -> usize {
        self.entries.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use crate::tools::ToolOutput;
    use crate::tools::idempotency::{IdempotencyCacheConfig, ToolIdempotencyCache};

    fn make_cache(ttl_ms: u64, max_per_job: usize) -> ToolIdempotencyCache {
        ToolIdempotencyCache::new(IdempotencyCacheConfig {
            ttl: Duration::from_millis(ttl_ms),
            max_entries_per_job: max_per_job,
        })
    }

    fn sample_output(text: &str) -> ToolOutput {
        ToolOutput::text(text, Duration::from_millis(1))
    }

    #[test]
    fn cache_key_deterministic() {
        let args = serde_json::json!({"query": "hello", "limit": 5});
        let k1 = ToolIdempotencyCache::cache_key("memory_search", &args);
        let k2 = ToolIdempotencyCache::cache_key("memory_search", &args);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn cache_key_varies_by_tool_name() {
        let args = serde_json::json!({"query": "hello"});
        let k1 = ToolIdempotencyCache::cache_key("memory_search", &args);
        let k2 = ToolIdempotencyCache::cache_key("memory_read", &args);
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_varies_by_args() {
        let k1 = ToolIdempotencyCache::cache_key("echo", &serde_json::json!({"message": "hello"}));
        let k2 = ToolIdempotencyCache::cache_key("echo", &serde_json::json!({"message": "world"}));
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn cache_hit_returns_stored_output() {
        let cache = make_cache(60_000, 100);
        let job = Uuid::new_v4();
        let args = serde_json::json!({"message": "hi"});

        // Miss
        assert!(cache.get(job, "echo", &args).await.is_none());

        // Store
        cache.put(job, "echo", &args, sample_output("hi")).await;

        // Hit
        let hit = cache.get(job, "echo", &args).await;
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().result, serde_json::json!("hi"));
    }

    #[tokio::test]
    async fn cache_miss_for_different_job() {
        let cache = make_cache(60_000, 100);
        let job_a = Uuid::new_v4();
        let job_b = Uuid::new_v4();
        let args = serde_json::json!({"message": "hi"});

        cache.put(job_a, "echo", &args, sample_output("hi")).await;

        // Different job should miss
        assert!(cache.get(job_b, "echo", &args).await.is_none());
        // Same job should hit
        assert!(cache.get(job_a, "echo", &args).await.is_some());
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let cache = make_cache(1, 100); // 1ms TTL
        let job = Uuid::new_v4();
        let args = serde_json::json!({"message": "hi"});

        cache.put(job, "echo", &args, sample_output("hi")).await;

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(cache.get(job, "echo", &args).await.is_none());
    }

    #[tokio::test]
    async fn lru_eviction() {
        let cache = make_cache(60_000, 2); // max 2 per job
        let job = Uuid::new_v4();

        // Fill with 2 entries
        let args_a = serde_json::json!({"n": 1});
        let args_b = serde_json::json!({"n": 2});
        cache.put(job, "echo", &args_a, sample_output("a")).await;
        cache.put(job, "echo", &args_b, sample_output("b")).await;
        assert_eq!(cache.len().await, 2);

        // Access args_a so args_b becomes the LRU
        cache.get(job, "echo", &args_a).await;

        // Add a third: should evict args_b (oldest accessed)
        let args_c = serde_json::json!({"n": 3});
        cache.put(job, "echo", &args_c, sample_output("c")).await;
        assert_eq!(cache.len().await, 2);

        // args_b should be gone, args_a and args_c should remain
        assert!(cache.get(job, "echo", &args_b).await.is_none());
        assert!(cache.get(job, "echo", &args_a).await.is_some());
        assert!(cache.get(job, "echo", &args_c).await.is_some());
    }

    #[tokio::test]
    async fn invalidate_job_clears_entries() {
        let cache = make_cache(60_000, 100);
        let job_a = Uuid::new_v4();
        let job_b = Uuid::new_v4();
        let args = serde_json::json!({"message": "hi"});

        cache.put(job_a, "echo", &args, sample_output("a")).await;
        cache.put(job_b, "echo", &args, sample_output("b")).await;
        assert_eq!(cache.len().await, 2);

        cache.invalidate_job(job_a).await;

        assert_eq!(cache.len().await, 1);
        assert!(cache.get(job_a, "echo", &args).await.is_none());
        assert!(cache.get(job_b, "echo", &args).await.is_some());
    }

    #[tokio::test]
    async fn lru_eviction_does_not_affect_other_jobs() {
        let cache = make_cache(60_000, 1); // max 1 per job
        let job_a = Uuid::new_v4();
        let job_b = Uuid::new_v4();

        let args_1 = serde_json::json!({"n": 1});
        let args_2 = serde_json::json!({"n": 2});

        cache.put(job_a, "echo", &args_1, sample_output("a1")).await;
        cache.put(job_b, "echo", &args_1, sample_output("b1")).await;

        // Adding a second entry for job_a should evict job_a's first entry,
        // but not touch job_b's entry
        cache.put(job_a, "echo", &args_2, sample_output("a2")).await;

        assert!(cache.get(job_a, "echo", &args_1).await.is_none());
        assert!(cache.get(job_a, "echo", &args_2).await.is_some());
        assert!(cache.get(job_b, "echo", &args_1).await.is_some());
    }

    #[test]
    fn default_config_is_reasonable() {
        let cfg = IdempotencyCacheConfig::default();
        assert_eq!(cfg.ttl, Duration::from_secs(30 * 60));
        assert_eq!(cfg.max_entries_per_job, 500);
    }
}
