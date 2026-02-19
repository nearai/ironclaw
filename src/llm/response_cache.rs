//! In-memory LLM response cache with TTL and LRU eviction.
//!
//! Wraps any [`LlmProvider`] and caches [`complete()`] responses keyed
//! by a SHA-256 hash of the messages and model name. Tool-calling
//! requests are never cached since they can trigger side effects.
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │               CachedProvider                      │
//! │  complete() ──► cache lookup ──► hit? return      │
//! │                                  miss? call inner │
//! │                                  store response   │
//! │                                                    │
//! │  complete_with_tools() ──► always call inner       │
//! └──────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Configuration for the response cache.
#[derive(Debug, Clone)]
pub struct ResponseCacheConfig {
    /// Time-to-live for cache entries.
    pub ttl: Duration,
    /// Maximum number of cached entries before LRU eviction.
    pub max_entries: usize,
}

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(3600), // 1 hour
            max_entries: 1000,
        }
    }
}

struct CacheEntry {
    response: CompletionResponse,
    created_at: Instant,
    last_accessed: Instant,
    hit_count: u64,
}

/// LLM provider wrapper that caches `complete()` responses.
///
/// Tool completion requests are always forwarded without caching since
/// tool calls can have side effects that should not be replayed.
pub struct CachedProvider {
    inner: Arc<dyn LlmProvider>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    config: ResponseCacheConfig,
}

impl CachedProvider {
    /// Wrap an existing provider with response caching.
    pub fn new(inner: Arc<dyn LlmProvider>, config: ResponseCacheConfig) -> Self {
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

    /// Total cache hits across all entries.
    pub async fn total_hits(&self) -> u64 {
        self.cache.lock().await.values().map(|e| e.hit_count).sum()
    }

    /// Clear all cached entries.
    pub async fn clear(&self) {
        self.cache.lock().await.clear();
    }
}

/// Build a deterministic cache key from a completion request.
///
/// Hashes the model name, messages, and response-affecting parameters
/// (max_tokens, temperature, stop_sequences) via SHA-256. Two requests
/// with identical content and parameters produce the same key.
fn cache_key(model: &str, request: &CompletionRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"|");

    // Messages are Serialize, so we can deterministically hash them.
    // serde_json produces stable output for the same input structure.
    if let Ok(json) = serde_json::to_string(&request.messages) {
        hasher.update(json.as_bytes());
    }

    // Include response-affecting parameters so different temperatures,
    // max_tokens, or stop sequences produce distinct cache keys.
    hasher.update(b"|");
    if let Some(max_tokens) = request.max_tokens {
        hasher.update(max_tokens.to_le_bytes());
    }
    hasher.update(b"|");
    if let Some(temp) = request.temperature {
        hasher.update(temp.to_le_bytes());
    }
    hasher.update(b"|");
    if let Some(ref stops) = request.stop_sequences {
        for s in stops {
            hasher.update(s.as_bytes());
            hasher.update(b"\x00");
        }
    }

    format!("{:x}", hasher.finalize())
}

#[async_trait]
impl LlmProvider for CachedProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let effective_model = self.inner.effective_model_name(request.model.as_deref());
        let key = cache_key(&effective_model, &request);
        let now = Instant::now();

        // Check cache
        {
            let mut guard = self.cache.lock().await;
            if let Some(entry) = guard.get_mut(&key) {
                if now.duration_since(entry.created_at) < self.config.ttl {
                    entry.last_accessed = now;
                    entry.hit_count += 1;
                    tracing::debug!(hits = entry.hit_count, "response cache hit");
                    return Ok(entry.response.clone());
                }
                // Expired, remove it
                guard.remove(&key);
            }
        }

        // Cache miss, call the real provider
        let response = self.inner.complete(request).await?;

        // Store in cache
        {
            let mut guard = self.cache.lock().await;

            // Evict expired entries
            guard.retain(|_, entry| now.duration_since(entry.created_at) < self.config.ttl);

            // LRU eviction if over capacity
            while guard.len() >= self.config.max_entries {
                let oldest_key = guard
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_accessed)
                    .map(|(k, _)| k.clone());

                if let Some(k) = oldest_key {
                    guard.remove(&k);
                } else {
                    break;
                }
            }

            guard.insert(
                key,
                CacheEntry {
                    response: response.clone(),
                    created_at: now,
                    last_accessed: now,
                    hit_count: 0,
                },
            );
        }

        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        // Never cache tool calls; they can trigger side effects.
        self.inner.complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn seed_response_chain(&self, thread_id: &str, response_id: String) {
        self.inner.seed_response_chain(thread_id, response_id);
    }

    fn get_response_chain_id(&self, thread_id: &str) -> Option<String> {
        self.inner.get_response_chain_id(thread_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::provider::ChatMessage;
    use crate::llm::response_cache::*;
    use crate::testing::StubLlm;

    fn simple_request() -> CompletionRequest {
        CompletionRequest {
            messages: vec![ChatMessage::user("hello")],
            model: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: Default::default(),
        }
    }

    fn different_request() -> CompletionRequest {
        CompletionRequest {
            messages: vec![ChatMessage::user("goodbye")],
            model: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn cache_key_is_deterministic() {
        let req = simple_request();
        let k1 = cache_key("model-a", &req);
        let k2 = cache_key("model-a", &req);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn cache_key_varies_by_model() {
        let req = simple_request();
        let k1 = cache_key("model-a", &req);
        let k2 = cache_key("model-b", &req);
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_varies_by_messages() {
        let k1 = cache_key("model-a", &simple_request());
        let k2 = cache_key("model-a", &different_request());
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_varies_by_temperature() {
        let mut req_a = simple_request();
        req_a.temperature = Some(0.0);
        let mut req_b = simple_request();
        req_b.temperature = Some(1.0);
        assert_ne!(cache_key("m", &req_a), cache_key("m", &req_b));
    }

    #[test]
    fn cache_key_varies_by_max_tokens() {
        let mut req_a = simple_request();
        req_a.max_tokens = Some(100);
        let mut req_b = simple_request();
        req_b.max_tokens = Some(500);
        assert_ne!(cache_key("m", &req_a), cache_key("m", &req_b));
    }

    #[tokio::test]
    async fn cache_hit_avoids_provider_call() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(
            stub.clone(),
            ResponseCacheConfig {
                ttl: Duration::from_secs(60),
                max_entries: 100,
            },
        );

        // First call: cache miss
        let r1 = cached.complete(simple_request()).await.unwrap();
        assert_eq!(stub.calls(), 1);
        assert_eq!(r1.content, "cached response");

        // Second call: cache hit
        let r2 = cached.complete(simple_request()).await.unwrap();
        assert_eq!(stub.calls(), 1); // still 1
        assert_eq!(r2.content, "cached response");

        assert_eq!(cached.total_hits().await, 1);
    }

    #[tokio::test]
    async fn different_messages_get_different_entries() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(stub.clone(), ResponseCacheConfig::default());

        cached.complete(simple_request()).await.unwrap();
        cached.complete(different_request()).await.unwrap();

        assert_eq!(stub.calls(), 2);
        assert_eq!(cached.len().await, 2);
    }

    #[tokio::test]
    async fn expired_entries_are_evicted() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(
            stub.clone(),
            ResponseCacheConfig {
                ttl: Duration::from_millis(1),
                max_entries: 100,
            },
        );

        cached.complete(simple_request()).await.unwrap();
        assert_eq!(stub.calls(), 1);

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Should be a cache miss now
        cached.complete(simple_request()).await.unwrap();
        assert_eq!(stub.calls(), 2);
    }

    #[tokio::test]
    async fn lru_eviction_removes_oldest() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(
            stub.clone(),
            ResponseCacheConfig {
                ttl: Duration::from_secs(60),
                max_entries: 2,
            },
        );

        // Fill cache with 2 entries
        cached.complete(simple_request()).await.unwrap();
        cached.complete(different_request()).await.unwrap();
        assert_eq!(cached.len().await, 2);

        // Add a third: should evict the oldest
        let third = CompletionRequest {
            messages: vec![ChatMessage::user("third")],
            model: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: Default::default(),
        };
        cached.complete(third).await.unwrap();
        assert_eq!(cached.len().await, 2);
        assert_eq!(stub.calls(), 3);
    }

    #[tokio::test]
    async fn tool_calls_are_never_cached() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(stub.clone(), ResponseCacheConfig::default());

        let req = ToolCompletionRequest {
            messages: vec![ChatMessage::user("use tool")],
            tools: vec![],
            model: None,
            max_tokens: None,
            temperature: None,
            tool_choice: None,
            metadata: Default::default(),
        };

        cached.complete_with_tools(req.clone()).await.unwrap();
        cached.complete_with_tools(req).await.unwrap();

        // Both should have called through
        assert_eq!(stub.calls(), 2);
        assert!(cached.is_empty().await);
    }

    #[tokio::test]
    async fn provider_errors_are_not_cached() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(
            stub.clone(),
            ResponseCacheConfig {
                ttl: Duration::from_secs(60),
                max_entries: 100,
            },
        );

        stub.set_failing(true);
        let result = cached.complete(simple_request()).await;
        assert!(result.is_err());
        assert!(cached.is_empty().await);

        // After fixing the provider, should succeed and cache
        stub.set_failing(false);
        cached.complete(simple_request()).await.unwrap();
        assert_eq!(cached.len().await, 1);
    }

    #[tokio::test]
    async fn clear_empties_cache() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(stub.clone(), ResponseCacheConfig::default());

        cached.complete(simple_request()).await.unwrap();
        assert_eq!(cached.len().await, 1);

        cached.clear().await;
        assert!(cached.is_empty().await);
    }

    #[tokio::test]
    async fn model_override_gets_distinct_cache_entries() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(stub.clone(), ResponseCacheConfig::default());

        let mut req_a = simple_request();
        req_a.model = Some("model-a".to_string());
        let mut req_b = simple_request();
        req_b.model = Some("model-b".to_string());

        cached.complete(req_a).await.unwrap();
        cached.complete(req_b).await.unwrap();

        assert_eq!(stub.calls(), 2);
        assert_eq!(cached.len().await, 2);
    }

    #[test]
    fn default_config_is_reasonable() {
        let cfg = ResponseCacheConfig::default();
        assert_eq!(cfg.ttl, Duration::from_secs(3600));
        assert_eq!(cfg.max_entries, 1000);
    }

    #[tokio::test]
    async fn delegates_model_name() {
        let stub = Arc::new(StubLlm::new("cached response"));
        let cached = CachedProvider::new(stub.clone(), ResponseCacheConfig::default());
        assert_eq!(cached.model_name(), "stub-model");
    }
}
