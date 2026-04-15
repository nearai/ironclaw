//! Runtime hot-reload support for LLM providers.
//!
//! The core provider chain is rebuilt from config when LLM settings change.
//! This module keeps the public `Arc<dyn LlmProvider>` stable while allowing
//! the inner provider chain to be swapped without restarting the daemon.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

fn leak_str(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

#[derive(Debug, Clone, Copy)]
struct ProviderSnapshot {
    model_name: &'static str,
    active_model_name: &'static str,
    cost_per_token: (Decimal, Decimal),
    cache_write_multiplier: Decimal,
    cache_read_discount: Decimal,
}

impl ProviderSnapshot {
    fn capture(provider: &dyn LlmProvider) -> Self {
        Self {
            model_name: leak_str(provider.model_name().to_string()),
            active_model_name: leak_str(provider.active_model_name()),
            cost_per_token: provider.cost_per_token(),
            cache_write_multiplier: provider.cache_write_multiplier(),
            cache_read_discount: provider.cache_read_discount(),
        }
    }
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// A provider wrapper whose inner provider can be swapped at runtime.
///
/// The wrapper keeps a stable `Arc<dyn LlmProvider>` for the rest of the
/// application while the inner provider chain is rebuilt from config.
///
/// `model_name()` and `active_model_name()` are cached snapshots of the
/// latest inner provider. The cached strings are intentionally leaked on swap
/// because hot reloads are user-driven and infrequent; this keeps the wrapper
/// safe and lock-free for the trait's synchronous metadata methods.
pub struct SwappableLlmProvider {
    inner: RwLock<Arc<dyn LlmProvider>>,
    model_name: RwLock<&'static str>,
    active_model_name: RwLock<&'static str>,
    cost_per_token: RwLock<(Decimal, Decimal)>,
    cache_write_multiplier: RwLock<Decimal>,
    cache_read_discount: RwLock<Decimal>,
}

impl std::fmt::Debug for SwappableLlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwappableLlmProvider")
            .field("model_name", &self.model_name())
            .field("active_model_name", &self.active_model_name())
            .finish_non_exhaustive()
    }
}

impl SwappableLlmProvider {
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        let snapshot = ProviderSnapshot::capture(inner.as_ref());
        Self {
            inner: RwLock::new(inner),
            model_name: RwLock::new(snapshot.model_name),
            active_model_name: RwLock::new(snapshot.active_model_name),
            cost_per_token: RwLock::new(snapshot.cost_per_token),
            cache_write_multiplier: RwLock::new(snapshot.cache_write_multiplier),
            cache_read_discount: RwLock::new(snapshot.cache_read_discount),
        }
    }

    fn refresh_snapshot(&self, provider: &dyn LlmProvider) {
        let snapshot = ProviderSnapshot::capture(provider);
        *write_lock(&self.model_name) = snapshot.model_name;
        *write_lock(&self.active_model_name) = snapshot.active_model_name;
        *write_lock(&self.cost_per_token) = snapshot.cost_per_token;
        *write_lock(&self.cache_write_multiplier) = snapshot.cache_write_multiplier;
        *write_lock(&self.cache_read_discount) = snapshot.cache_read_discount;
    }

    /// Replace the inner provider chain with a freshly rebuilt provider.
    pub fn swap(&self, inner: Arc<dyn LlmProvider>) {
        *write_lock(&self.inner) = inner;
        let current = self.current();
        self.refresh_snapshot(current.as_ref());
    }

    fn current(&self) -> Arc<dyn LlmProvider> {
        read_lock(&self.inner).clone()
    }
}

#[async_trait]
impl LlmProvider for SwappableLlmProvider {
    fn model_name(&self) -> &str {
        *read_lock(&self.model_name)
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        *read_lock(&self.cost_per_token)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.current().complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.current().complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.current().list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.current().model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.current().effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        read_lock(&self.active_model_name).to_string()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        let current = self.current();
        let result = current.set_model(model);
        if result.is_ok() {
            self.refresh_snapshot(current.as_ref());
        }
        result
    }

    fn cache_write_multiplier(&self) -> Decimal {
        *read_lock(&self.cache_write_multiplier)
    }

    fn cache_read_discount(&self) -> Decimal {
        *read_lock(&self.cache_read_discount)
    }
}

/// Stable hot-reload handle for the primary/cheap provider chain.
#[derive(Debug)]
pub struct LlmReloadHandle {
    primary: Arc<SwappableLlmProvider>,
    cheap: Option<Arc<SwappableLlmProvider>>,
}

impl LlmReloadHandle {
    pub fn new(
        primary: Arc<SwappableLlmProvider>,
        cheap: Option<Arc<SwappableLlmProvider>>,
    ) -> Self {
        Self { primary, cheap }
    }

    pub fn primary_provider(&self) -> Arc<dyn LlmProvider> {
        self.primary.clone() as Arc<dyn LlmProvider>
    }

    pub fn cheap_provider(&self) -> Option<Arc<dyn LlmProvider>> {
        self.cheap
            .as_ref()
            .map(|provider| provider.clone() as Arc<dyn LlmProvider>)
    }

    pub fn primary_model_name(&self) -> String {
        self.primary.active_model_name()
    }

    pub async fn reload(
        &self,
        config: &crate::llm::LlmConfig,
        session: Arc<crate::llm::SessionManager>,
    ) -> Result<(), LlmError> {
        let components = crate::llm::build_provider_chain_components(config, session).await?;
        self.primary.swap(components.primary);

        if let Some(ref cheap_handle) = self.cheap {
            let new_cheap = components.cheap.unwrap_or_else(|| self.primary_provider());
            cheap_handle.swap(new_cheap);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    struct TestProvider {
        model_name: &'static str,
        active_model_name: String,
        cost_per_token: (Decimal, Decimal),
    }

    impl TestProvider {
        fn new(
            model_name: &'static str,
            active_model_name: impl Into<String>,
            cost_per_token: (Decimal, Decimal),
        ) -> Self {
            Self {
                model_name,
                active_model_name: active_model_name.into(),
                cost_per_token,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for TestProvider {
        fn model_name(&self) -> &str {
            self.model_name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            self.cost_per_token
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: self.model_name.to_string(),
                reason: "test provider does not complete".to_string(),
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: self.model_name.to_string(),
                reason: "test provider does not complete_with_tools".to_string(),
            })
        }

        fn active_model_name(&self) -> String {
            self.active_model_name.clone()
        }
    }

    #[test]
    fn swappable_provider_refreshes_snapshot_on_swap() {
        let primary = Arc::new(TestProvider::new(
            "configured-a",
            "active-a",
            (Decimal::new(1, 0), Decimal::new(2, 0)),
        ));
        let swappable = SwappableLlmProvider::new(primary);

        assert_eq!(swappable.model_name(), "configured-a");
        assert_eq!(swappable.active_model_name(), "active-a");
        assert_eq!(
            swappable.cost_per_token(),
            (Decimal::new(1, 0), Decimal::new(2, 0))
        );

        let replacement = Arc::new(TestProvider::new(
            "configured-b",
            "active-b",
            (Decimal::new(3, 0), Decimal::new(4, 0)),
        ));
        swappable.swap(replacement);

        assert_eq!(swappable.model_name(), "configured-b");
        assert_eq!(swappable.active_model_name(), "active-b");
        assert_eq!(
            swappable.cost_per_token(),
            (Decimal::new(3, 0), Decimal::new(4, 0))
        );
    }
}
