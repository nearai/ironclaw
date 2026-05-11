//! Route-keyed provider pool for managing per-route LLM provider instances.
//!
//! Each unique `(provider_id, model_id)` route gets its own `Arc<dyn LlmProvider>`
//! instance. This is critical for providers like [`RigAdapter`](crate::RigAdapter)
//! that bake the model at construction time — sharing a single provider across
//! different models would silently use the wrong model.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::provider::LlmProvider;

/// A pool of LLM provider instances keyed by `(provider_id, model_id)` route.
///
/// Thread-safe (`Send + Sync`) via interior `RwLock`. Each unique route gets
/// its own `Arc<dyn LlmProvider>` instance, created lazily on first access
/// via a caller-provided factory.
pub struct RouteKeyedProviderPool {
    providers: RwLock<HashMap<(String, String), Arc<dyn LlmProvider>>>,
}

impl std::fmt::Debug for RouteKeyedProviderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .providers
            .read()
            .map(|p| p.len())
            .unwrap_or(0);
        f.debug_struct("RouteKeyedProviderPool")
            .field("cached_count", &count)
            .finish()
    }
}

impl Default for RouteKeyedProviderPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteKeyedProviderPool {
    /// Create an empty provider pool.
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a provider for the given route.
    ///
    /// If a provider already exists for this `(provider_id, model_id)` pair,
    /// it is returned directly. Otherwise, `factory` is called to create one,
    /// which is then cached and returned.
    ///
    /// The factory is only called once per unique key — subsequent calls for
    /// the same key return the cached `Arc`.
    pub fn get_or_create<F>(
        &self,
        provider_id: &str,
        model_id: &str,
        factory: F,
    ) -> Arc<dyn LlmProvider>
    where
        F: FnOnce() -> Arc<dyn LlmProvider>,
    {
        let key = (provider_id.to_string(), model_id.to_string());

        // Fast path: read lock
        {
            let providers = self.providers.read().expect("provider pool lock poisoned");
            if let Some(provider) = providers.get(&key) {
                return Arc::clone(provider);
            }
        }

        // Slow path: write lock + double-check
        let mut providers = self.providers.write().expect("provider pool lock poisoned");
        if let Some(provider) = providers.get(&key) {
            return Arc::clone(provider);
        }

        let provider = factory();
        providers.insert(key, Arc::clone(&provider));
        provider
    }

    /// Remove all cached providers, forcing fresh creation on next access.
    ///
    /// Useful for reload/reset scenarios where provider configuration has changed.
    pub fn clear(&self) {
        let mut providers = self.providers.write().expect("provider pool lock poisoned");
        providers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use crate::provider::{
        CompletionRequest, CompletionResponse, ModelMetadata, ToolCompletionRequest,
        ToolCompletionResponse,
    };

    /// Minimal stub provider for testing pool behavior.
    #[derive(Debug)]
    struct StubProvider {
        name: String,
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        fn model_name(&self) -> &str {
            &self.name
        }

        async fn model_metadata(&self) -> Result<ModelMetadata, crate::error::LlmError> {
            Ok(ModelMetadata {
                id: self.name.clone(),
                context_length: None,
            })
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }
    }

    #[test]
    fn pool_returns_same_arc_for_same_key() {
        let pool = RouteKeyedProviderPool::new();
        let first = pool.get_or_create("openai", "gpt-4o", || {
            Arc::new(StubProvider {
                name: "gpt-4o".to_string(),
            })
        });
        let second = pool.get_or_create("openai", "gpt-4o", || {
            panic!("factory should not be called again")
        });
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn pool_returns_different_instance_for_different_key() {
        let pool = RouteKeyedProviderPool::new();
        let a = pool.get_or_create("openai", "gpt-4o", || {
            Arc::new(StubProvider {
                name: "gpt-4o".to_string(),
            })
        });
        let b = pool.get_or_create("anthropic", "claude-sonnet", || {
            Arc::new(StubProvider {
                name: "claude-sonnet".to_string(),
            })
        });
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(a.model_name(), "gpt-4o");
        assert_eq!(b.model_name(), "claude-sonnet");
    }

    #[test]
    fn factory_only_called_once_per_key() {
        let pool = RouteKeyedProviderPool::new();
        let call_count = AtomicUsize::new(0);
        for _ in 0..5 {
            pool.get_or_create("openai", "gpt-4o", || {
                call_count.fetch_add(1, Ordering::SeqCst);
                Arc::new(StubProvider {
                    name: "gpt-4o".to_string(),
                })
            });
        }
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clear_causes_fresh_creation() {
        let pool = RouteKeyedProviderPool::new();
        let first = pool.get_or_create("openai", "gpt-4o", || {
            Arc::new(StubProvider {
                name: "gpt-4o".to_string(),
            })
        });

        pool.clear();

        let second = pool.get_or_create("openai", "gpt-4o", || {
            Arc::new(StubProvider {
                name: "gpt-4o-v2".to_string(),
            })
        });

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.model_name(), "gpt-4o");
        assert_eq!(second.model_name(), "gpt-4o-v2");
    }
}
