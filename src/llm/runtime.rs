//! Runtime hot-reload support for LLM providers.
//!
//! The core provider chain is rebuilt from config when LLM settings change.
//! [`SwappableLlmProvider`] wraps `Arc<dyn LlmProvider>` so the outer handle
//! stays stable across rebuilds and the rest of the application doesn't have
//! to re-subscribe. [`LlmReloadHandle`] ties the primary and cheap providers
//! together and serializes overlapping reloads.
//!
//! ## Design notes
//!
//! - **One snapshot lock.** All cached metadata (`model_name`,
//!   `active_model_name`, cost, cache multipliers, and the inner provider
//!   itself) live in a single `RwLock<ProviderSnapshot>`. A reader always
//!   observes a consistent slice of one provider — never a mix of old and
//!   new after a swap.
//! - **No unbounded leaks.** `model_name()` returns `&'static str` because
//!   the trait requires it; we intern each distinct name through a global
//!   `Mutex<HashMap>` so leakage is bounded by the set of distinct model
//!   names a process ever sees (typically a handful).
//! - **`set_model()` is volatile.** Runtime model switches are forwarded to
//!   the current inner provider only. The next successful
//!   [`LlmReloadHandle::reload`] rebuilds the chain from config and drops
//!   the override. Callers that rely on a model override must persist it
//!   through the normal settings path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Intern a model-name string so it can be returned through the trait's
/// `fn model_name(&self) -> &str` contract without leaking on every swap.
fn intern_model_name(name: &str) -> &'static str {
    static INTERNER: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = INTERNER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    guard.insert(name.to_string(), leaked);
    leaked
}

#[derive(Clone)]
struct ProviderSnapshot {
    inner: Arc<dyn LlmProvider>,
    model_name: &'static str,
    active_model_name: Arc<str>,
    cost_per_token: (Decimal, Decimal),
    cache_write_multiplier: Decimal,
    cache_read_discount: Decimal,
}

impl std::fmt::Debug for ProviderSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSnapshot")
            .field("model_name", &self.model_name)
            .field("active_model_name", &&*self.active_model_name)
            .finish_non_exhaustive()
    }
}

impl ProviderSnapshot {
    fn capture(provider: Arc<dyn LlmProvider>) -> Self {
        let model_name = intern_model_name(provider.model_name());
        let active_model_name = Arc::from(provider.active_model_name());
        let cost_per_token = provider.cost_per_token();
        let cache_write_multiplier = provider.cache_write_multiplier();
        let cache_read_discount = provider.cache_read_discount();
        Self {
            inner: provider,
            model_name,
            active_model_name,
            cost_per_token,
            cache_write_multiplier,
            cache_read_discount,
        }
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A provider wrapper whose inner provider can be swapped at runtime.
///
/// See the module-level docs for the invariants this type guarantees.
pub struct SwappableLlmProvider {
    state: RwLock<ProviderSnapshot>,
}

impl std::fmt::Debug for SwappableLlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snap = read(&self.state);
        f.debug_struct("SwappableLlmProvider")
            .field("model_name", &snap.model_name)
            .field("active_model_name", &&*snap.active_model_name)
            .finish_non_exhaustive()
    }
}

impl SwappableLlmProvider {
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        Self {
            state: RwLock::new(ProviderSnapshot::capture(inner)),
        }
    }

    /// Replace the inner provider chain with a freshly rebuilt provider.
    /// Metadata is refreshed atomically in the same critical section.
    pub fn swap(&self, inner: Arc<dyn LlmProvider>) {
        let fresh = ProviderSnapshot::capture(inner);
        *write(&self.state) = fresh;
    }

    fn current(&self) -> Arc<dyn LlmProvider> {
        read(&self.state).inner.clone()
    }
}

#[async_trait]
impl LlmProvider for SwappableLlmProvider {
    fn model_name(&self) -> &str {
        read(&self.state).model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        read(&self.state).cost_per_token
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
        read(&self.state).active_model_name.to_string()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        let current = self.current();
        current.set_model(model)?;
        let fresh = ProviderSnapshot::capture(current);
        *write(&self.state) = fresh;
        Ok(())
    }

    fn cache_write_multiplier(&self) -> Decimal {
        read(&self.state).cache_write_multiplier
    }

    fn cache_read_discount(&self) -> Decimal {
        read(&self.state).cache_read_discount
    }
}

/// Stable hot-reload handle for the primary/cheap provider chain.
///
/// Holds the two [`SwappableLlmProvider`] wrappers created at startup and
/// serializes concurrent reloads through an internal mutex so rapid setting
/// changes don't trigger overlapping chain rebuilds (which would redo
/// potentially-expensive work like OAuth refresh and HTTP probes).
#[derive(Debug)]
pub struct LlmReloadHandle {
    primary: Arc<SwappableLlmProvider>,
    cheap: Option<Arc<SwappableLlmProvider>>,
    reload_lock: tokio::sync::Mutex<()>,
}

impl LlmReloadHandle {
    pub fn new(
        primary: Arc<SwappableLlmProvider>,
        cheap: Option<Arc<SwappableLlmProvider>>,
    ) -> Self {
        Self {
            primary,
            cheap,
            reload_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn primary_provider(&self) -> Arc<dyn LlmProvider> {
        self.primary.clone() as Arc<dyn LlmProvider>
    }

    pub fn cheap_provider(&self) -> Option<Arc<dyn LlmProvider>> {
        self.cheap
            .as_ref()
            .map(|provider| provider.clone() as Arc<dyn LlmProvider>)
    }

    /// Rebuild the provider chain from `config` and atomically replace the
    /// inner providers of the primary (and cheap, if present) wrappers.
    ///
    /// Reloads are serialized so two concurrent callers cannot race.
    pub async fn reload(
        &self,
        config: &crate::llm::LlmConfig,
        session: Arc<crate::llm::SessionManager>,
    ) -> Result<(), LlmError> {
        let _guard = self.reload_lock.lock().await;

        let components = crate::llm::build_provider_chain_components(config, session).await?;

        self.primary.swap(components.primary);

        if let Some(ref cheap_handle) = self.cheap {
            let new_cheap = components
                .cheap
                .unwrap_or_else(|| self.primary.clone() as Arc<dyn LlmProvider>);
            cheap_handle.swap(new_cheap);
        } else if components.cheap.is_some() {
            // Asymmetry: no cheap wrapper was allocated at startup, so a
            // newly-configured cheap model cannot be activated via hot-reload.
            // Surfacing this through tracing so ops don't think the swap
            // silently took effect.
            tracing::warn!(
                "llm hot-reload: cheap provider is now configured but was not at startup; \
                 it will only take effect after a full restart",
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{CompletionRequest, ToolCompletionRequest};
    use std::sync::RwLock as StdRwLock;

    /// Simple stub that supports `set_model()` so we can exercise the
    /// snapshot-refresh path and the "override is lost on swap" behaviour.
    #[derive(Debug)]
    struct TestProvider {
        configured: &'static str,
        active: StdRwLock<String>,
        cost: (Decimal, Decimal),
        cache_write: Decimal,
        cache_read: Decimal,
    }

    impl TestProvider {
        fn new(configured: &'static str, active: &str, cost: (Decimal, Decimal)) -> Self {
            Self {
                configured,
                active: StdRwLock::new(active.to_string()),
                cost,
                cache_write: Decimal::ONE,
                cache_read: Decimal::ONE,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for TestProvider {
        fn model_name(&self) -> &str {
            self.configured
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            self.cost
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: self.configured.to_string(),
                reason: "TestProvider does not implement complete".to_string(),
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: self.configured.to_string(),
                reason: "TestProvider does not implement complete_with_tools".to_string(),
            })
        }

        fn active_model_name(&self) -> String {
            self.active.read().expect("test lock").clone()
        }

        fn set_model(&self, model: &str) -> Result<(), LlmError> {
            *self.active.write().expect("test lock") = model.to_string();
            Ok(())
        }

        fn cache_write_multiplier(&self) -> Decimal {
            self.cache_write
        }

        fn cache_read_discount(&self) -> Decimal {
            self.cache_read
        }
    }

    #[test]
    fn swap_replaces_all_metadata_atomically() {
        let a = Arc::new(TestProvider::new(
            "cfg-a",
            "active-a",
            (Decimal::new(1, 0), Decimal::new(2, 0)),
        ));
        let wrapper = SwappableLlmProvider::new(a);

        assert_eq!(wrapper.model_name(), "cfg-a");
        assert_eq!(wrapper.active_model_name(), "active-a");
        assert_eq!(
            wrapper.cost_per_token(),
            (Decimal::new(1, 0), Decimal::new(2, 0))
        );

        let b = Arc::new(TestProvider::new(
            "cfg-b",
            "active-b",
            (Decimal::new(3, 0), Decimal::new(4, 0)),
        ));
        wrapper.swap(b);

        assert_eq!(wrapper.model_name(), "cfg-b");
        assert_eq!(wrapper.active_model_name(), "active-b");
        assert_eq!(
            wrapper.cost_per_token(),
            (Decimal::new(3, 0), Decimal::new(4, 0))
        );
    }

    #[test]
    fn set_model_forwards_and_refreshes_snapshot() {
        let inner = Arc::new(TestProvider::new(
            "cfg",
            "cfg",
            (Decimal::ZERO, Decimal::ZERO),
        ));
        let wrapper = SwappableLlmProvider::new(inner);

        wrapper
            .set_model("cfg-override")
            .expect("test provider supports set_model");

        assert_eq!(wrapper.active_model_name(), "cfg-override");
    }

    #[test]
    fn set_model_override_is_dropped_on_swap() {
        let initial = Arc::new(TestProvider::new(
            "cfg-a",
            "cfg-a",
            (Decimal::ZERO, Decimal::ZERO),
        ));
        let wrapper = SwappableLlmProvider::new(initial);

        wrapper
            .set_model("cfg-a-override")
            .expect("set_model supported");
        assert_eq!(wrapper.active_model_name(), "cfg-a-override");

        let replacement = Arc::new(TestProvider::new(
            "cfg-b",
            "cfg-b",
            (Decimal::ZERO, Decimal::ZERO),
        ));
        wrapper.swap(replacement);

        assert_eq!(wrapper.active_model_name(), "cfg-b");
    }

    #[test]
    fn model_name_interner_reuses_leaked_strings() {
        let a = intern_model_name("gpt-5");
        let b = intern_model_name("gpt-5");
        assert_eq!(a.as_ptr(), b.as_ptr());
    }
}
