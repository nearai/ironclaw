//! Multi-provider LLM failover.
//!
//! Wraps multiple LlmProvider instances and tries each in sequence
//! until one succeeds. Transparent to callers --- same LlmProvider trait.
//!
//! Providers that fail repeatedly are temporarily placed in cooldown
//! so subsequent requests skip them, reducing latency when a provider
//! is known to be down. Cooldown state is lock-free (atomics only).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

use crate::llm::retry::is_retryable;

/// Configuration for per-provider cooldown behavior.
///
/// When a provider accumulates `failure_threshold` consecutive retryable
/// failures, it enters cooldown for `cooldown_duration`. During cooldown
/// the provider is skipped (unless *all* providers are in cooldown, in
/// which case the oldest-cooled one is tried).
#[derive(Debug, Clone)]
pub struct CooldownConfig {
    /// How long a provider stays in cooldown after exceeding the threshold.
    pub cooldown_duration: Duration,
    /// Number of consecutive retryable failures before cooldown activates.
    pub failure_threshold: u32,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 3,
        }
    }
}

/// Per-provider cooldown state, entirely lock-free.
///
/// All atomic operations use `Relaxed` ordering — consistent with the
/// existing `last_used` field. Stale reads are harmless: the worst case
/// is one extra attempt against a provider that just entered cooldown.
struct ProviderCooldown {
    /// Consecutive retryable failures. Reset to 0 on success.
    failure_count: AtomicU32,
    /// Nanoseconds since `epoch` when cooldown was activated.
    /// 0 means the provider is NOT in cooldown.
    cooldown_activated_nanos: AtomicU64,
}

impl ProviderCooldown {
    fn new() -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            cooldown_activated_nanos: AtomicU64::new(0),
        }
    }

    /// Check whether the provider is currently in cooldown.
    fn is_in_cooldown(&self, now_nanos: u64, cooldown_nanos: u64) -> bool {
        let activated = self.cooldown_activated_nanos.load(Ordering::Relaxed);
        activated != 0 && now_nanos.saturating_sub(activated) < cooldown_nanos
    }

    /// Record a retryable failure. Returns `true` if the threshold was
    /// just reached (caller should activate cooldown).
    fn record_failure(&self, threshold: u32) -> bool {
        let prev = self.failure_count.fetch_add(1, Ordering::Relaxed);
        prev + 1 >= threshold
    }

    /// Activate cooldown at the given timestamp.
    fn activate_cooldown(&self, now_nanos: u64) {
        // Ensure 0 remains a safe "not in cooldown" sentinel.
        self.cooldown_activated_nanos
            .store(now_nanos.max(1), Ordering::Relaxed);
    }

    /// Reset failure count and clear cooldown (called on success).
    fn reset(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        self.cooldown_activated_nanos.store(0, Ordering::Relaxed);
    }
}

/// An LLM provider that wraps multiple providers and tries each in sequence
/// on transient failures.
///
/// The first provider in the list is the primary. If it fails with a retryable
/// error, the next provider is tried, and so on. Non-retryable errors
/// (e.g. `AuthFailed`, `ContextLengthExceeded`) propagate immediately.
///
/// Providers that repeatedly fail with retryable errors are temporarily
/// placed in cooldown and skipped, reducing latency.
pub struct FailoverProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
    /// Index of the provider that last handled a request successfully.
    /// Used by `model_name()` and `cost_per_token()` so downstream cost
    /// tracking reflects the provider that actually served the request.
    last_used: AtomicUsize,
    /// Per-provider cooldown tracking (same length as `providers`).
    cooldowns: Vec<ProviderCooldown>,
    /// Reference instant for computing elapsed nanos. Shared across all
    /// cooldown timestamps so they are comparable.
    epoch: Instant,
    /// Cooldown configuration.
    cooldown_config: CooldownConfig,
    /// Request-scoped provider index keyed by Tokio task ID.
    ///
    /// This allows `effective_model_name()` to report the provider that handled
    /// the *current* request, even when other concurrent requests update
    /// `last_used`.
    provider_for_task: Mutex<HashMap<tokio::task::Id, usize>>,
}

impl FailoverProvider {
    /// Create a new failover provider with default cooldown settings.
    ///
    /// Returns an error if `providers` is empty.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Result<Self, LlmError> {
        Self::with_cooldown(providers, CooldownConfig::default())
    }

    /// Create a new failover provider with explicit cooldown configuration.
    ///
    /// Returns an error if `providers` is empty.
    pub fn with_cooldown(
        providers: Vec<Arc<dyn LlmProvider>>,
        cooldown_config: CooldownConfig,
    ) -> Result<Self, LlmError> {
        if providers.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: "FailoverProvider requires at least one provider".to_string(),
            });
        }
        let cooldowns = (0..providers.len())
            .map(|_| ProviderCooldown::new())
            .collect();
        Ok(Self {
            providers,
            last_used: AtomicUsize::new(0),
            cooldowns,
            epoch: Instant::now(),
            cooldown_config,
            provider_for_task: Mutex::new(HashMap::new()),
        })
    }

    /// Nanoseconds elapsed since `self.epoch`.
    ///
    /// Truncates `u128` → `u64` (wraps after ~584 years of continuous
    /// uptime). Acceptable because `epoch` is set at construction time.
    fn now_nanos(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Current Tokio task ID if available.
    fn current_task_id() -> Option<tokio::task::Id> {
        tokio::task::try_id()
    }

    /// Bind the selected provider index to the current task.
    fn bind_provider_to_current_task(&self, provider_idx: usize) {
        let Some(task_id) = Self::current_task_id() else {
            return;
        };
        if let Ok(mut guard) = self.provider_for_task.lock() {
            guard.insert(task_id, provider_idx);
        }
    }

    /// Take and remove the provider index bound to the current task.
    fn take_bound_provider_for_current_task(&self) -> Option<usize> {
        let task_id = Self::current_task_id()?;
        self.provider_for_task
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(&task_id))
    }

    /// Try each provider in sequence until one succeeds or all fail.
    ///
    /// Providers in cooldown are skipped unless *all* providers are in
    /// cooldown, in which case the one with the oldest cooldown timestamp
    /// (most likely to have recovered) is tried.
    async fn try_providers<T, F, Fut>(&self, mut call: F) -> Result<(usize, T), LlmError>
    where
        F: FnMut(Arc<dyn LlmProvider>) -> Fut,
        Fut: Future<Output = Result<T, LlmError>>,
    {
        let now_nanos = self.now_nanos();
        let cooldown_nanos = self.cooldown_config.cooldown_duration.as_nanos() as u64;

        // Partition providers into available and cooled-down.
        let (mut available, cooled_down): (Vec<usize>, Vec<usize>) = (0..self.providers.len())
            .partition(|&i| !self.cooldowns[i].is_in_cooldown(now_nanos, cooldown_nanos));

        // Log skipped providers.
        for &i in &cooled_down {
            tracing::info!(
                provider = %self.providers[i].model_name(),
                "Skipping provider (in cooldown)"
            );
        }

        // Never skip ALL providers: if every provider is in cooldown, pick
        // the one with the oldest cooldown activation (most likely recovered).
        if available.is_empty() {
            let oldest = (0..self.providers.len())
                .min_by_key(|&i| {
                    self.cooldowns[i]
                        .cooldown_activated_nanos
                        .load(Ordering::Relaxed)
                })
                .ok_or_else(|| LlmError::RequestFailed {
                    provider: "failover".to_string(),
                    reason: "FailoverProvider requires at least one provider".to_string(),
                })?;
            tracing::info!(
                provider = %self.providers[oldest].model_name(),
                "All providers in cooldown, trying oldest-cooled provider"
            );
            available.push(oldest);
        }

        let mut last_error: Option<LlmError> = None;

        for (pos, &i) in available.iter().enumerate() {
            let provider = &self.providers[i];
            let result = call(Arc::clone(provider)).await;
            match result {
                Ok(response) => {
                    self.last_used.store(i, Ordering::Relaxed);
                    self.cooldowns[i].reset();
                    return Ok((i, response));
                }
                Err(err) => {
                    if !is_retryable(&err) {
                        return Err(err);
                    }

                    // Increment failure count; activate cooldown if threshold reached.
                    if self.cooldowns[i].record_failure(self.cooldown_config.failure_threshold) {
                        let nanos = self.now_nanos();
                        self.cooldowns[i].activate_cooldown(nanos);
                        tracing::warn!(
                            provider = %provider.model_name(),
                            threshold = self.cooldown_config.failure_threshold,
                            cooldown_secs = self.cooldown_config.cooldown_duration.as_secs(),
                            "Provider entered cooldown after repeated failures"
                        );
                    }

                    if pos + 1 < available.len() {
                        let next_i = available[pos + 1];
                        tracing::warn!(
                            provider = %provider.model_name(),
                            error = %err,
                            next_provider = %self.providers[next_i].model_name(),
                            "Provider failed with retryable error, trying next provider"
                        );
                    }
                    last_error = Some(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: "failover".to_string(),
            reason: "Invariant violated in FailoverProvider: providers were exhausted but no last_error was recorded (this branch should be unreachable; possible causes: no provider attempts were made or `available` was unexpectedly empty).".to_string(),
        }))
    }
}

#[async_trait]
impl LlmProvider for FailoverProvider {
    fn model_name(&self) -> &str {
        self.providers[self.last_used.load(Ordering::Relaxed)].model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.providers[self.last_used.load(Ordering::Relaxed)].cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (provider_idx, response) = self
            .try_providers(|provider| {
                let req = request.clone();
                async move { provider.complete(req).await }
            })
            .await?;
        self.bind_provider_to_current_task(provider_idx);
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (provider_idx, response) = self
            .try_providers(|provider| {
                let req = request.clone();
                async move { provider.complete_with_tools(req).await }
            })
            .await?;
        self.bind_provider_to_current_task(provider_idx);
        Ok(response)
    }

    fn active_model_name(&self) -> String {
        self.providers[self.last_used.load(Ordering::Relaxed)].active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        for provider in &self.providers {
            provider.set_model(model)?;
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let mut all_models = Vec::new();

        for provider in &self.providers {
            match provider.list_models().await {
                Ok(models) => all_models.extend(models),
                Err(err) => {
                    tracing::warn!(
                        provider = %provider.model_name(),
                        error = %err,
                        "Failed to list models from provider, skipping"
                    );
                }
            }
        }

        all_models.sort();
        all_models.dedup();
        Ok(all_models)
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.providers[self.last_used.load(Ordering::Relaxed)]
            .model_metadata()
            .await
    }

    fn seed_response_chain(&self, thread_id: &str, response_id: String) {
        self.providers[self.last_used.load(Ordering::Relaxed)]
            .seed_response_chain(thread_id, response_id);
    }

    fn get_response_chain_id(&self, thread_id: &str) -> Option<String> {
        self.providers[self.last_used.load(Ordering::Relaxed)].get_response_chain_id(thread_id)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.providers[self.last_used.load(Ordering::Relaxed)]
            .calculate_cost(input_tokens, output_tokens)
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        if let Some(provider_idx) = self.take_bound_provider_for_current_task() {
            return self.providers[provider_idx].effective_model_name(requested_model);
        }

        self.providers[self.last_used.load(Ordering::Relaxed)].effective_model_name(requested_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Mutex, RwLock};
    use std::time::Duration;

    use crate::llm::provider::{CompletionResponse, FinishReason, ToolCompletionResponse};

    /// A mock LLM provider that returns a predetermined result.
    struct MockProvider {
        name: String,
        active_model: RwLock<String>,
        input_cost: Decimal,
        output_cost: Decimal,
        complete_result: Mutex<Option<Result<CompletionResponse, LlmError>>>,
        tool_complete_result: Mutex<Option<Result<ToolCompletionResponse, LlmError>>>,
    }

    impl MockProvider {
        fn succeeding(name: &str, content: &str) -> Self {
            Self {
                name: name.to_string(),
                active_model: RwLock::new(name.to_string()),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Ok(CompletionResponse {
                    content: content.to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: FinishReason::Stop,
                    response_id: None,
                }))),
                tool_complete_result: Mutex::new(Some(Ok(ToolCompletionResponse {
                    content: Some(content.to_string()),
                    tool_calls: vec![],
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: FinishReason::Stop,
                    response_id: None,
                }))),
            }
        }

        fn succeeding_with_cost(
            name: &str,
            content: &str,
            input_cost: Decimal,
            output_cost: Decimal,
        ) -> Self {
            Self {
                input_cost,
                output_cost,
                ..Self::succeeding(name, content)
            }
        }

        fn failing_retryable(name: &str) -> Self {
            Self {
                name: name.to_string(),
                active_model: RwLock::new(name.to_string()),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::RequestFailed {
                    provider: name.to_string(),
                    reason: "server error".to_string(),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::RequestFailed {
                    provider: name.to_string(),
                    reason: "server error".to_string(),
                }))),
            }
        }

        fn failing_non_retryable(name: &str) -> Self {
            Self {
                name: name.to_string(),
                active_model: RwLock::new(name.to_string()),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::AuthFailed {
                    provider: name.to_string(),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::AuthFailed {
                    provider: name.to_string(),
                }))),
            }
        }

        fn failing_rate_limited(name: &str) -> Self {
            Self {
                name: name.to_string(),
                active_model: RwLock::new(name.to_string()),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::RateLimited {
                    provider: name.to_string(),
                    retry_after: Some(Duration::from_secs(30)),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::RateLimited {
                    provider: name.to_string(),
                    retry_after: Some(Duration::from_secs(30)),
                }))),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn model_name(&self) -> &str {
            &self.name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (self.input_cost, self.output_cost)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete_result
                .lock()
                .unwrap()
                .take()
                .expect("MockProvider::complete called more than once")
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.tool_complete_result
                .lock()
                .unwrap()
                .take()
                .expect("MockProvider::complete_with_tools called more than once")
        }

        async fn list_models(&self) -> Result<Vec<String>, LlmError> {
            Ok(vec![self.name.clone()])
        }

        fn active_model_name(&self) -> String {
            self.active_model.read().unwrap().clone()
        }

        fn set_model(&self, model: &str) -> Result<(), LlmError> {
            *self.active_model.write().unwrap() = model.to_string();
            Ok(())
        }
    }

    fn make_request() -> CompletionRequest {
        CompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")], vec![])
    }

    // Test 1: Primary succeeds, no failover occurs.
    #[tokio::test]
    async fn primary_succeeds_no_failover() {
        let primary = Arc::new(MockProvider::succeeding("primary", "primary response"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "primary response");
    }

    // Test 2: Primary fails with retryable error, fallback succeeds.
    #[tokio::test]
    async fn primary_fails_retryable_fallback_succeeds() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "fallback response");
    }

    // Test 3: All providers fail, returns last error.
    #[tokio::test]
    async fn all_providers_fail_returns_last_error() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::failing_retryable("fallback"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let err = failover.complete(make_request()).await.unwrap_err();
        match err {
            LlmError::RequestFailed { provider, .. } => {
                assert_eq!(provider, "fallback");
            }
            other => panic!("expected RequestFailed, got: {other:?}"),
        }
    }

    // Test 4: Non-retryable error fails immediately, no failover.
    #[tokio::test]
    async fn non_retryable_error_fails_immediately() {
        let primary = Arc::new(MockProvider::failing_non_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let err = failover.complete(make_request()).await.unwrap_err();
        match err {
            LlmError::AuthFailed { provider } => {
                assert_eq!(provider, "primary");
            }
            other => panic!("expected AuthFailed, got: {other:?}"),
        }
    }

    // Test 5: Three providers, first two fail (retryable), third succeeds.
    #[tokio::test]
    async fn three_providers_first_two_fail_third_succeeds() {
        let p1 = Arc::new(MockProvider::failing_retryable("provider-1"));
        let p2 = Arc::new(MockProvider::failing_rate_limited("provider-2"));
        let p3 = Arc::new(MockProvider::succeeding("provider-3", "third time lucky"));

        let failover = FailoverProvider::new(vec![p1, p2, p3]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "third time lucky");
    }

    // Test: complete_with_tools follows same failover logic.
    #[tokio::test]
    async fn complete_with_tools_failover() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "tools fallback"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();
        assert_eq!(response.content.as_deref(), Some("tools fallback"));
    }

    // Test: model_name and cost_per_token reflect the last-used provider.
    #[tokio::test]
    async fn model_name_and_cost_track_last_used_provider() {
        let fallback_cost = Decimal::new(15, 6); // 0.000015

        let primary = Arc::new(MockProvider::failing_retryable("primary-model"));
        let fallback = Arc::new(MockProvider::succeeding_with_cost(
            "fallback-model",
            "ok",
            fallback_cost,
            fallback_cost,
        ));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        // Before any call, defaults to primary (index 0).
        assert_eq!(failover.model_name(), "primary-model");
        assert_eq!(failover.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));

        // After failover, should reflect the fallback provider.
        let _ = failover.complete(make_request()).await.unwrap();
        assert_eq!(failover.model_name(), "fallback-model");
        assert_eq!(failover.cost_per_token(), (fallback_cost, fallback_cost));
    }

    // Test: model reporting is request-scoped under concurrent requests.
    #[tokio::test]
    async fn effective_model_name_is_request_scoped_under_concurrency() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(60),
            failure_threshold: 3,
        };
        let primary = Arc::new(MultiCallMockProvider::fail_then_ok("primary", 1));
        let fallback = Arc::new(MultiCallMockProvider::always_ok("fallback"));
        let failover =
            Arc::new(FailoverProvider::with_cooldown(vec![primary, fallback], config).unwrap());

        let (first_done_tx, first_done_rx) = tokio::sync::oneshot::channel::<()>();
        let (second_done_tx, second_done_rx) = tokio::sync::oneshot::channel::<()>();

        let failover_a = Arc::clone(&failover);
        let task_a = tokio::spawn(async move {
            // First request: primary fails once, fallback serves.
            let _ = failover_a.complete(make_request()).await.unwrap();
            let _ = first_done_tx.send(());

            // Wait until the second request finishes and updates global state.
            let _ = second_done_rx.await;
            failover_a.effective_model_name(None)
        });

        let failover_b = Arc::clone(&failover);
        let task_b = tokio::spawn(async move {
            let _ = first_done_rx.await;
            // Second request: primary now succeeds.
            let _ = failover_b.complete(make_request()).await.unwrap();
            let model = failover_b.effective_model_name(None);
            let _ = second_done_tx.send(());
            model
        });

        let model_b = task_b.await.unwrap();
        let model_a = task_a.await.unwrap();

        assert_eq!(model_a, "fallback");
        assert_eq!(model_b, "primary");
    }

    // Test: list_models aggregates from all providers.
    #[tokio::test]
    async fn list_models_aggregates_all() {
        let p1 = Arc::new(MockProvider::succeeding("model-a", "ok"));
        let p2 = Arc::new(MockProvider::succeeding("model-b", "ok"));

        let failover = FailoverProvider::new(vec![p1, p2]).unwrap();

        let models = failover.list_models().await.unwrap();
        assert!(models.contains(&"model-a".to_string()));
        assert!(models.contains(&"model-b".to_string()));
    }

    // --- MultiCallMockProvider for cooldown tests ---
    //
    // Unlike `MockProvider` which uses `.take()` (single-use), this mock
    // tracks a call counter and returns errors for the first N calls,
    // then succeeds.

    struct MultiCallMockProvider {
        name: String,
        /// How many calls should fail before succeeding. 0 = always succeed.
        fail_count: u32,
        /// Atomically tracks how many times `complete` has been called.
        calls: AtomicU32,
        /// If true, failures are non-retryable (AuthFailed).
        non_retryable: bool,
    }

    impl MultiCallMockProvider {
        /// Always succeeds.
        fn always_ok(name: &str) -> Self {
            Self {
                name: name.to_string(),
                fail_count: 0,
                calls: AtomicU32::new(0),
                non_retryable: false,
            }
        }

        /// Fails with retryable error for the first `n` calls, then succeeds.
        fn fail_then_ok(name: &str, n: u32) -> Self {
            Self {
                name: name.to_string(),
                fail_count: n,
                calls: AtomicU32::new(0),
                non_retryable: false,
            }
        }

        /// Always fails with retryable error.
        fn always_fail(name: &str) -> Self {
            Self {
                name: name.to_string(),
                fail_count: u32::MAX,
                calls: AtomicU32::new(0),
                non_retryable: false,
            }
        }

        /// Always fails with non-retryable error.
        fn always_fail_non_retryable(name: &str) -> Self {
            Self {
                name: name.to_string(),
                fail_count: u32::MAX,
                calls: AtomicU32::new(0),
                non_retryable: true,
            }
        }

        fn call_count(&self) -> u32 {
            self.calls.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl LlmProvider for MultiCallMockProvider {
        fn model_name(&self) -> &str {
            &self.name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed);
            if n < self.fail_count {
                if self.non_retryable {
                    return Err(LlmError::AuthFailed {
                        provider: self.name.clone(),
                    });
                }
                return Err(LlmError::RequestFailed {
                    provider: self.name.clone(),
                    reason: format!("call {} failed", n),
                });
            }
            Ok(CompletionResponse {
                content: format!("{} ok", self.name),
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed);
            if n < self.fail_count {
                if self.non_retryable {
                    return Err(LlmError::AuthFailed {
                        provider: self.name.clone(),
                    });
                }
                return Err(LlmError::RequestFailed {
                    provider: self.name.clone(),
                    reason: format!("call {} failed", n),
                });
            }
            Ok(ToolCompletionResponse {
                content: Some(format!("{} ok", self.name)),
                tool_calls: vec![],
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }

        async fn list_models(&self) -> Result<Vec<String>, LlmError> {
            Ok(vec![self.name.clone()])
        }
    }

    // --- Cooldown tests ---

    // Cooldown test 1: Provider enters cooldown after `threshold` consecutive failures.
    #[tokio::test]
    async fn cooldown_activates_after_threshold() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 2,
        };
        let p1 = Arc::new(MultiCallMockProvider::always_fail("p1"));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config).unwrap();

        // Request 1: p1 fails (count=1, below threshold), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 1);

        // Request 2: p1 fails again (count=2, reaches threshold → cooldown), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 2);

        // Request 3: p1 should be skipped (in cooldown), only p2 called.
        let prev_p1_calls = p1.call_count();
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        // p1 was NOT called again.
        assert_eq!(p1.call_count(), prev_p1_calls);
    }

    // Cooldown test 2: Cooldown expires after duration, provider is retried.
    #[tokio::test]
    async fn cooldown_expires_after_duration() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_millis(1),
            failure_threshold: 1,
        };
        // p1 fails once then succeeds (fail_then_ok with n=1 would work,
        // but we use always_fail to prove it's skipped, then swap).
        let p1 = Arc::new(MultiCallMockProvider::fail_then_ok("p1", 2));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config).unwrap();

        // Request 1: p1 fails (threshold=1, enters cooldown immediately), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 1);

        // Request 2: p1 in cooldown, skipped. Only p2 called.
        // (But cooldown is 1ms, so wait a bit to let it expire.)
        tokio::time::sleep(Duration::from_millis(5)).await;

        // After sleep, cooldown should have expired. p1 gets tried again.
        // p1 is set to fail 2 times total, so call #2 (index 1) still fails.
        // But it proves p1 was attempted again after cooldown expired.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(p1.call_count(), 2); // p1 was retried
        assert_eq!(r.content, "p2 ok"); // p2 handled it

        // Wait again for cooldown to expire, p1 call #3 (index 2) succeeds.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p1 ok");
        assert_eq!(p1.call_count(), 3);
    }

    // Cooldown test 3: Never skip all providers — oldest-cooled one is tried.
    #[tokio::test]
    async fn never_skip_all_providers() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 1,
        };
        // Both providers always fail.
        let p1 = Arc::new(MultiCallMockProvider::always_fail("p1"));
        let p2 = Arc::new(MultiCallMockProvider::always_fail("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config).unwrap();

        // Request 1: both tried, both fail, both enter cooldown.
        let _ = failover.complete(make_request()).await;
        assert_eq!(p1.call_count(), 1);
        assert_eq!(p2.call_count(), 1);

        // Request 2: all in cooldown, but the oldest-cooled one (p1, activated
        // first) should be tried.
        let prev_total = p1.call_count() + p2.call_count();
        let _ = failover.complete(make_request()).await;
        let new_total = p1.call_count() + p2.call_count();
        // Exactly one more call was made (to the oldest-cooled provider).
        assert_eq!(new_total, prev_total + 1);
    }

    // Cooldown test 4: Success resets failure count so it never reaches threshold.
    //
    // With threshold=3, accumulate 2 failures then succeed. Verify the
    // atomic counter is back to 0 and no cooldown was activated. Then
    // use a second provider pair to show that without the reset, 3
    // consecutive failures DO trigger cooldown (control case).
    #[tokio::test]
    async fn reset_on_success() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 3,
        };
        // p1 fails for calls 0,1 then succeeds on call 2+.
        let p1 = Arc::new(MultiCallMockProvider::fail_then_ok("p1", 2));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config.clone()).unwrap();

        // Request 1: p1 fails (failure_count=1), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");

        // Request 2: p1 fails (failure_count=2, still below threshold=3), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 2);

        // Request 3: p1 succeeds (call index 2) → counter resets to 0.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p1 ok");
        assert_eq!(p1.call_count(), 3);

        // Verify counter was reset to 0 and no cooldown activated.
        let nanos = failover.now_nanos();
        let cooldown_nanos = failover.cooldown_config.cooldown_duration.as_nanos() as u64;
        assert!(!failover.cooldowns[0].is_in_cooldown(nanos, cooldown_nanos));
        assert_eq!(
            failover.cooldowns[0].failure_count.load(Ordering::Relaxed),
            0
        );

        // Control: without a success in the middle, 3 failures DO trigger cooldown.
        let p3 = Arc::new(MultiCallMockProvider::always_fail("p3"));
        let p4 = Arc::new(MultiCallMockProvider::always_ok("p4"));
        let control =
            FailoverProvider::with_cooldown(vec![p3.clone(), p4.clone()], config).unwrap();
        for _ in 0..3 {
            let _ = control.complete(make_request()).await.unwrap();
        }
        let nanos = control.now_nanos();
        assert!(control.cooldowns[0].is_in_cooldown(nanos, cooldown_nanos));
    }

    // Cooldown test 5: threshold-1 failures don't trigger cooldown, threshold does.
    #[tokio::test]
    async fn threshold_boundary() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 3,
        };
        let p1 = Arc::new(MultiCallMockProvider::always_fail("p1"));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config).unwrap();

        // 2 requests: p1 fails twice (below threshold of 3), not in cooldown.
        for _ in 0..2 {
            let r = failover.complete(make_request()).await.unwrap();
            assert_eq!(r.content, "p2 ok");
        }
        assert_eq!(p1.call_count(), 2);

        // p1 should still be available (not in cooldown).
        let nanos = failover.now_nanos();
        let cooldown_nanos = failover.cooldown_config.cooldown_duration.as_nanos() as u64;
        assert!(!failover.cooldowns[0].is_in_cooldown(nanos, cooldown_nanos));

        // 3rd request: p1 fails → reaches threshold → enters cooldown.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 3);

        let nanos = failover.now_nanos();
        assert!(failover.cooldowns[0].is_in_cooldown(nanos, cooldown_nanos));

        // 4th request: p1 should be skipped.
        let prev = p1.call_count();
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), prev); // not called
    }

    // Cooldown test 6: Non-retryable error returns immediately, no failure bump.
    #[tokio::test]
    async fn non_retryable_does_not_increment_cooldown() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 1,
        };
        let p1 = Arc::new(MultiCallMockProvider::always_fail_non_retryable("p1"));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone()], config).unwrap();

        // Non-retryable error should return immediately.
        let err = failover.complete(make_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::AuthFailed { .. }));
        assert_eq!(p1.call_count(), 1);
        // p2 should NOT have been called (non-retryable = no failover).
        assert_eq!(p2.call_count(), 0);

        // p1 should NOT be in cooldown (non-retryable doesn't bump count).
        let nanos = failover.now_nanos();
        let cooldown_nanos = failover.cooldown_config.cooldown_duration.as_nanos() as u64;
        assert!(!failover.cooldowns[0].is_in_cooldown(nanos, cooldown_nanos));
    }

    // Cooldown test 7: Three providers, first in cooldown, second/third available.
    #[tokio::test]
    async fn three_providers_mixed_cooldown() {
        let config = CooldownConfig {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 1,
        };
        let p1 = Arc::new(MultiCallMockProvider::always_fail("p1"));
        let p2 = Arc::new(MultiCallMockProvider::always_ok("p2"));
        let p3 = Arc::new(MultiCallMockProvider::always_ok("p3"));

        let failover =
            FailoverProvider::with_cooldown(vec![p1.clone(), p2.clone(), p3.clone()], config)
                .unwrap();

        // Request 1: p1 fails → enters cooldown (threshold=1), p2 succeeds.
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), 1);

        // Request 2: p1 skipped (cooldown), p2 and p3 available.
        let prev = p1.call_count();
        let r = failover.complete(make_request()).await.unwrap();
        assert_eq!(r.content, "p2 ok");
        assert_eq!(p1.call_count(), prev); // p1 skipped
    }

    // Test: is_retryable correctly classifies errors.
    #[test]
    fn retryable_classification() {
        // Retryable
        assert!(is_retryable(&LlmError::RequestFailed {
            provider: "p".into(),
            reason: "err".into(),
        }));
        assert!(is_retryable(&LlmError::RateLimited {
            provider: "p".into(),
            retry_after: None,
        }));
        assert!(is_retryable(&LlmError::InvalidResponse {
            provider: "p".into(),
            reason: "bad json".into(),
        }));
        assert!(is_retryable(&LlmError::SessionRenewalFailed {
            provider: "p".into(),
            reason: "timeout".into(),
        }));
        assert!(is_retryable(&LlmError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));

        // Non-retryable
        assert!(!is_retryable(&LlmError::AuthFailed {
            provider: "p".into(),
        }));
        assert!(!is_retryable(&LlmError::SessionExpired {
            provider: "p".into(),
        }));
        assert!(!is_retryable(&LlmError::ContextLengthExceeded {
            used: 100_000,
            limit: 50_000,
        }));
        assert!(!is_retryable(&LlmError::ModelNotAvailable {
            provider: "p".into(),
            model: "m".into(),
        }));
    }

    // Test: empty providers list returns error (not panic).
    #[test]
    fn empty_providers_returns_error() {
        let result = FailoverProvider::new(vec![]);
        assert!(result.is_err());
    }

    // Test: activate_cooldown(0) still activates cooldown (sentinel collision fix).
    #[test]
    fn cooldown_at_nanos_zero_still_activates() {
        let cd = ProviderCooldown::new();
        cd.activate_cooldown(0);
        assert!(cd.is_in_cooldown(0, 1000));
        assert_eq!(cd.cooldown_activated_nanos.load(Ordering::Relaxed), 1);
    }

    // Test: set_model propagates to all providers and active_model_name reflects change.
    #[test]
    fn set_model_propagates_to_all_providers() {
        let p1: Arc<MockProvider> = Arc::new(MockProvider::succeeding("model-a", "ok"));
        let p2: Arc<MockProvider> = Arc::new(MockProvider::succeeding("model-b", "ok"));

        let failover = FailoverProvider::new(vec![
            Arc::clone(&p1) as Arc<dyn LlmProvider>,
            Arc::clone(&p2) as Arc<dyn LlmProvider>,
        ])
        .unwrap();

        // Before: active_model_name delegates to last_used (index 0 = p1).
        assert_eq!(failover.active_model_name(), "model-a");

        // Switch model.
        failover.set_model("new-model").unwrap();

        // Both inner providers should reflect the change.
        assert_eq!(p1.active_model_name(), "new-model");
        assert_eq!(p2.active_model_name(), "new-model");

        // FailoverProvider itself should report the new model.
        assert_eq!(failover.active_model_name(), "new-model");
    }
}
