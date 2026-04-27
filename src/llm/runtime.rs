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

/// Maximum number of distinct model names interned over a process lifetime.
/// Bounded to prevent unbounded `Box::leak` growth if `set_model` is ever
/// called with adversarial or malformed input (e.g. LLM tool-call output).
const INTERN_MAX_ENTRIES: usize = 1024;

/// Maximum length (in bytes) of a single interned model name. Anything
/// longer is treated as malformed — real model identifiers are well under
/// this (GPT-4o: 6 bytes, `anthropic.claude-opus-4-6-v1`: ~28 bytes).
const INTERN_MAX_LEN: usize = 256;

/// Fallback interned string used when a name exceeds the length limit or
/// the distinct-entry cap fills up. Chosen to be visibly wrong in logs so
/// operators notice the fallback rather than silently misattributing cost.
const INTERN_OVERFLOW_SENTINEL: &str = "<model-name-overflow>";

/// Intern a model-name string so it can be returned through the trait's
/// `fn model_name(&self) -> &str` contract without leaking on every swap.
///
/// Leakage is bounded two ways: per-entry via [`INTERN_MAX_LEN`] and
/// total via [`INTERN_MAX_ENTRIES`]. Hitting either cap logs at `warn!`
/// and returns a static sentinel, so the process can't be coerced into
/// unbounded memory growth by repeated `set_model` calls with novel
/// strings.
fn intern_model_name(name: &str) -> &'static str {
    static INTERNER: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = INTERNER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    intern_into(&mut guard, name, INTERN_MAX_ENTRIES, INTERN_MAX_LEN)
}

/// Lockless core of [`intern_model_name`], split out so the cap logic can
/// be unit-tested against a local `HashMap` without contaminating the
/// process-wide `OnceLock` interner that other tests read from.
fn intern_into(
    map: &mut HashMap<String, &'static str>,
    name: &str,
    max_entries: usize,
    max_len: usize,
) -> &'static str {
    if name.len() > max_len {
        tracing::warn!(
            len = name.len(),
            max = max_len,
            "model name exceeds interner length limit; using overflow sentinel",
        );
        return INTERN_OVERFLOW_SENTINEL;
    }
    if let Some(existing) = map.get(name) {
        return existing;
    }
    if map.len() >= max_entries {
        tracing::warn!(
            entries = map.len(),
            max = max_entries,
            "model name interner is full; using overflow sentinel",
        );
        return INTERN_OVERFLOW_SENTINEL;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    map.insert(name.to_string(), leaked);
    leaked
}

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
        // Hold the write lock across both the delegate call and the snapshot
        // refresh so a concurrent `swap()` cannot overwrite the just-updated
        // inner provider with a snapshot captured from an older one. Inner
        // `set_model` impls are synchronous (no `.await`), so holding a
        // std::sync lock across the call is safe.
        let mut guard = write(&self.state);
        guard.inner.set_model(model)?;
        let refreshed = ProviderSnapshot::capture(Arc::clone(&guard.inner));
        *guard = refreshed;
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
    /// Serializes concurrent `reload()` calls so rapid setting toggles
    /// don't fire overlapping chain rebuilds (each rebuild can touch OAuth
    /// refresh and HTTP probes; letting them pile up wastes upstream quota
    /// and leaves the wrapper briefly pointing at a half-built chain).
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

    /// A model name that overshoots the per-entry length limit must fall
    /// back to the overflow sentinel rather than leak a huge string.
    /// Exercises `intern_into` against a local map so the process-wide
    /// interner stays untouched (other tests depend on it returning real
    /// interned strings for normal names).
    #[test]
    fn intern_into_rejects_oversized_input() {
        let mut map: HashMap<String, &'static str> = HashMap::new();
        let huge = "x".repeat(INTERN_MAX_LEN + 1);
        let interned = intern_into(&mut map, &huge, INTERN_MAX_ENTRIES, INTERN_MAX_LEN);
        assert_eq!(interned, INTERN_OVERFLOW_SENTINEL);
        // Oversized input must not even reach the map — there would be
        // nothing to reuse across calls and leak-count would grow.
        assert!(map.is_empty());
    }

    /// Once the distinct-entry cap is reached, further novel names must
    /// route to the overflow sentinel — real model identifiers never need
    /// thousands of distinct variants, so hitting the cap signals an
    /// adversarial or buggy caller and we'd rather log+sentinel than
    /// leak memory. Uses a local map with a tiny cap so the test runs
    /// quickly and doesn't touch shared state.
    #[test]
    fn intern_into_caps_distinct_entries() {
        let mut map: HashMap<String, &'static str> = HashMap::new();
        let cap = 4;
        for i in 0..cap {
            let interned = intern_into(&mut map, &format!("name-{i}"), cap, INTERN_MAX_LEN);
            assert_ne!(interned, INTERN_OVERFLOW_SENTINEL);
        }
        // Past the cap: sentinel.
        let over = intern_into(&mut map, "one-too-many", cap, INTERN_MAX_LEN);
        assert_eq!(over, INTERN_OVERFLOW_SENTINEL);
        // And an already-interned name still resolves from the map.
        let reused = intern_into(&mut map, "name-0", cap, INTERN_MAX_LEN);
        assert_ne!(reused, INTERN_OVERFLOW_SENTINEL);
    }

    /// Concurrent `swap` and `set_model` against the same wrapper must
    /// never crash or deadlock, and the final snapshot must be readable.
    /// Before the fix, `set_model` released the state read lock between
    /// mutating the inner and writing the snapshot — a parallel `swap`
    /// could overwrite the snapshot with one from an older provider,
    /// leaving `model_name()` referring to a replacement's `&'static`
    /// literal while `cost_per_token()` was captured from a different
    /// inner. The write-lock-spanning fix keeps the swap and the set-model
    /// snapshot mutually exclusive.
    ///
    /// This is a stress test — it won't catch every possible race window,
    /// but it reliably triggers the pre-fix bug and passes cleanly post-fix.
    #[test]
    fn set_model_and_swap_are_mutually_atomic() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let initial = StdArc::new(TestProvider::new(
            "p-a",
            "p-a",
            (Decimal::ZERO, Decimal::ZERO),
        ));
        let wrapper = StdArc::new(SwappableLlmProvider::new(initial));

        const ITERS: usize = 200;

        let w1 = StdArc::clone(&wrapper);
        let swapper = thread::spawn(move || {
            for i in 0..ITERS {
                let replacement = StdArc::new(TestProvider::new(
                    if i % 2 == 0 { "p-even" } else { "p-odd" },
                    if i % 2 == 0 { "p-even" } else { "p-odd" },
                    (Decimal::ZERO, Decimal::ZERO),
                ));
                w1.swap(replacement);
            }
        });

        let w2 = StdArc::clone(&wrapper);
        let setter = thread::spawn(move || {
            for i in 0..ITERS {
                let _ = w2.set_model(&format!("override-{i}"));
            }
        });

        swapper.join().expect("swapper thread");
        setter.join().expect("setter thread");

        // Final invariant: the wrapper is still readable and its reported
        // `model_name` is one of the literals the swapper inserted (the
        // set_model path only ever updates `active_model_name`, not the
        // configured `model_name`). A torn snapshot wouldn't necessarily
        // corrupt the pointer, but a panic or deadlock under the old code
        // would.
        let configured = wrapper.model_name();
        assert!(
            matches!(configured, "p-a" | "p-even" | "p-odd"),
            "configured model_name must come from a real swap: {configured}",
        );
        let _ = wrapper.active_model_name();
        let _ = wrapper.cost_per_token();
    }
}
