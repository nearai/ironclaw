//! Smart routing provider that routes requests to cheap or primary models based on task complexity.
//!
//! Inspired by RelayPlane's cost-reduction approach: simple tasks (status checks, greetings,
//! short questions) go to a cheap model (e.g. Haiku), while complex tasks (code generation,
//! analysis, multi-step reasoning) go to the primary model (e.g. Sonnet/Opus).
//!
//! This is a decorator that wraps two `LlmProvider`s and implements `LlmProvider` itself,
//! following the same pattern as `RetryProvider`, `CachedProvider`, and `CircuitBreakerProvider`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use async_trait::async_trait;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, Role, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Classification of a request's complexity, determining which model handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// Short, simple queries -> cheap model
    Simple,
    /// Ambiguous complexity -> cheap model first, cascade to primary if uncertain
    Moderate,
    /// Code generation, analysis, multi-step reasoning -> primary model
    Complex,
}

/// Configuration for the smart routing provider.
#[derive(Debug, Clone)]
pub struct SmartRoutingConfig {
    /// Enable cascade mode: retry with primary if cheap model response seems uncertain.
    pub cascade_enabled: bool,
    /// Message length threshold below which a message may be classified as Simple (default: 200).
    pub simple_max_chars: usize,
    /// Message length threshold above which a message is classified as Complex (default: 1000).
    pub complex_min_chars: usize,
}

impl Default for SmartRoutingConfig {
    fn default() -> Self {
        Self {
            cascade_enabled: true,
            simple_max_chars: 200,
            complex_min_chars: 1000,
        }
    }
}

/// Atomic counters for routing observability.
struct SmartRoutingStats {
    total_requests: AtomicU64,
    cheap_requests: AtomicU64,
    primary_requests: AtomicU64,
    cascade_escalations: AtomicU64,
}

impl SmartRoutingStats {
    fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            cheap_requests: AtomicU64::new(0),
            primary_requests: AtomicU64::new(0),
            cascade_escalations: AtomicU64::new(0),
        }
    }
}

/// Snapshot of routing statistics for external consumption.
#[derive(Debug, Clone)]
pub struct SmartRoutingSnapshot {
    pub total_requests: u64,
    pub cheap_requests: u64,
    pub primary_requests: u64,
    pub cascade_escalations: u64,
}

/// Which inner provider handled a request.
#[derive(Debug, Clone, Copy)]
enum RoutedTo {
    Primary = 0,
    Cheap = 1,
}

impl RoutedTo {
    fn from_u8(v: u8) -> Self {
        if v == 1 { Self::Cheap } else { Self::Primary }
    }
}

/// Smart routing provider that classifies task complexity and routes to the appropriate model.
///
/// - `complete()` — classifies and routes to cheap or primary model
/// - `complete_with_tools()` — always routes to primary (tool use requires reliable structured output)
pub struct SmartRoutingProvider {
    primary: Arc<dyn LlmProvider>,
    cheap: Arc<dyn LlmProvider>,
    config: SmartRoutingConfig,
    stats: SmartRoutingStats,
    /// Global fallback: which provider last served a request.
    /// Used when task-scoped binding is unavailable (no Tokio task ID).
    last_routed: AtomicU8,
    /// Request-scoped routing decision keyed by Tokio task ID.
    /// Takes precedence over `last_routed` for concurrent use.
    ///
    /// Entries are inserted on `complete()`/`complete_with_tools()` and removed
    /// on `effective_model_name()`. If a task panics or is cancelled between
    /// insert and remove, entries leak. A capacity guard evicts all entries when
    /// the map exceeds [`Self::TASK_MAP_CAPACITY`] to prevent unbounded growth.
    routed_for_task: Mutex<HashMap<tokio::task::Id, RoutedTo>>,
}

impl SmartRoutingProvider {
    /// Maximum number of entries in `routed_for_task` before eviction.
    /// Each entry corresponds to a Tokio task that called `complete()` but
    /// hasn't yet called `effective_model_name()`. Under normal operation
    /// this map stays small (bounded by concurrency). A large map indicates
    /// leaked entries from cancelled or panicked tasks.
    const TASK_MAP_CAPACITY: usize = 1000;

    /// Create a new smart routing provider wrapping a primary and cheap provider.
    pub fn new(
        primary: Arc<dyn LlmProvider>,
        cheap: Arc<dyn LlmProvider>,
        config: SmartRoutingConfig,
    ) -> Self {
        Self {
            primary,
            cheap,
            config,
            stats: SmartRoutingStats::new(),
            last_routed: AtomicU8::new(RoutedTo::Primary as u8),
            routed_for_task: Mutex::new(HashMap::new()),
        }
    }

    /// Get a snapshot of routing statistics.
    pub fn stats(&self) -> SmartRoutingSnapshot {
        SmartRoutingSnapshot {
            total_requests: self.stats.total_requests.load(Ordering::Relaxed),
            cheap_requests: self.stats.cheap_requests.load(Ordering::Relaxed),
            primary_requests: self.stats.primary_requests.load(Ordering::Relaxed),
            cascade_escalations: self.stats.cascade_escalations.load(Ordering::Relaxed),
        }
    }

    /// Classify the complexity of a request based on its last user message.
    fn classify(&self, request: &CompletionRequest) -> TaskComplexity {
        let last_user_msg = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        classify_message(last_user_msg, &self.config)
    }

    /// Bind the routing decision to the current Tokio task and update the
    /// global fallback.
    ///
    /// If the map exceeds [`Self::TASK_MAP_CAPACITY`], all entries are evicted
    /// (with a warning log) before inserting the new entry. This prevents
    /// unbounded growth from leaked entries when tasks panic or are cancelled
    /// between `complete()` and `effective_model_name()`.
    fn bind_route_to_current_task(&self, routed_to: RoutedTo) {
        self.last_routed.store(routed_to as u8, Ordering::Relaxed);
        let Some(task_id) = tokio::task::try_id() else {
            return;
        };
        if let Ok(mut guard) = self.routed_for_task.lock() {
            if guard.len() >= Self::TASK_MAP_CAPACITY {
                tracing::warn!(
                    entries = guard.len(),
                    capacity = Self::TASK_MAP_CAPACITY,
                    "routed_for_task map exceeded capacity, evicting stale entries"
                );
                guard.clear();
            }
            guard.insert(task_id, routed_to);
        }
    }

    /// Take and remove the routing decision bound to the current task.
    /// Falls back to `last_routed` if no task ID is available.
    fn take_route_for_current_task(&self) -> RoutedTo {
        let task_routed = tokio::task::try_id().and_then(|task_id| {
            self.routed_for_task
                .lock()
                .ok()
                .and_then(|mut guard| guard.remove(&task_id))
        });
        task_routed.unwrap_or_else(|| RoutedTo::from_u8(self.last_routed.load(Ordering::Relaxed)))
    }

    /// Check if a response from the cheap model shows uncertainty, warranting escalation.
    fn response_is_uncertain(response: &CompletionResponse) -> bool {
        let content = response.content.trim();

        // Empty response is always uncertain
        if content.is_empty() {
            return true;
        }

        let lower = content.to_lowercase();

        // Uncertainty signals
        let uncertainty_patterns = [
            "i'm not sure",
            "i am not sure",
            "i don't know",
            "i do not know",
            "i'm unable to",
            "i am unable to",
            "i cannot",
            "i can't",
            "beyond my capabilities",
            "beyond my ability",
            "i'm not able to",
            "i am not able to",
            "i don't have enough",
            "i do not have enough",
            "i need more context",
            "i need more information",
            "could you clarify",
            "could you provide more",
            "i'm not confident",
            "i am not confident",
        ];

        uncertainty_patterns.iter().any(|p| lower.contains(p))
    }
}

/// Classify a message's complexity based on content patterns and length.
///
/// Exposed as a free function for testability.
fn classify_message(msg: &str, config: &SmartRoutingConfig) -> TaskComplexity {
    let trimmed = msg.trim();
    let len = trimmed.len();

    // Empty or very short -> Simple
    if len == 0 {
        return TaskComplexity::Simple;
    }

    // Check for code blocks (triple backticks) -> Complex
    if trimmed.contains("```") {
        return TaskComplexity::Complex;
    }

    let lower = trimmed.to_lowercase();

    // Complex keywords/patterns -> Complex regardless of length
    const COMPLEX_KEYWORDS: &[&str] = &[
        "implement",
        "refactor",
        "analyze",
        "debug",
        "create a",
        "build a",
        "design",
        "fix the",
        "fix this",
        "write a",
        "write the",
        "explain how",
        "explain why",
        "explain the",
        "compare",
        "optimize",
        "review",
        "rewrite",
        "migrate",
        "architect",
        "integrate",
    ];

    if COMPLEX_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return TaskComplexity::Complex;
    }

    // Long messages -> Complex
    if len >= config.complex_min_chars {
        return TaskComplexity::Complex;
    }

    // Simple keywords/patterns for short messages
    const SIMPLE_KEYWORDS: &[&str] = &[
        "list",
        "show",
        "what is",
        "what's",
        "status",
        "help",
        "yes",
        "no",
        "ok",
        "thanks",
        "thank you",
        "hello",
        "hi",
        "hey",
        "ping",
        "version",
        "how many",
        "when",
        "where is",
        "who",
    ];

    if len <= config.simple_max_chars && SIMPLE_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return TaskComplexity::Simple;
    }

    // Short confirmations / single words -> Simple
    if len <= 10 {
        return TaskComplexity::Simple;
    }

    // Everything else -> Moderate
    TaskComplexity::Moderate
}

#[async_trait]
impl LlmProvider for SmartRoutingProvider {
    crate::delegate_llm_provider!(self.primary, skip_effective_model_name);

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        match self.take_route_for_current_task() {
            RoutedTo::Primary => self.primary.effective_model_name(requested_model),
            RoutedTo::Cheap => self.cheap.effective_model_name(requested_model),
        }
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);

        let complexity = self.classify(&request);

        match complexity {
            TaskComplexity::Simple => {
                tracing::debug!(
                    model = %self.cheap.model_name(),
                    "Smart routing: Simple task -> cheap model"
                );
                self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                let result = self.cheap.complete(request).await;
                if result.is_ok() {
                    self.bind_route_to_current_task(RoutedTo::Cheap);
                }
                result
            }
            TaskComplexity::Complex => {
                tracing::debug!(
                    model = %self.primary.model_name(),
                    "Smart routing: Complex task -> primary model"
                );
                self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                let result = self.primary.complete(request).await;
                if result.is_ok() {
                    self.bind_route_to_current_task(RoutedTo::Primary);
                }
                result
            }
            TaskComplexity::Moderate => {
                if self.config.cascade_enabled {
                    tracing::debug!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade enabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);

                    let response = self.cheap.complete(request.clone()).await?;

                    if Self::response_is_uncertain(&response) {
                        tracing::info!(
                            cheap_model = %self.cheap.model_name(),
                            primary_model = %self.primary.model_name(),
                            "Smart routing: Escalating to primary (cheap model response uncertain)"
                        );
                        self.stats
                            .cascade_escalations
                            .fetch_add(1, Ordering::Relaxed);
                        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                        let result = self.primary.complete(request).await;
                        if result.is_ok() {
                            self.bind_route_to_current_task(RoutedTo::Primary);
                        }
                        result
                    } else {
                        self.bind_route_to_current_task(RoutedTo::Cheap);
                        Ok(response)
                    }
                } else {
                    // Without cascade, moderate tasks go to cheap model
                    tracing::debug!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade disabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                    let result = self.cheap.complete(request).await;
                    if result.is_ok() {
                        self.bind_route_to_current_task(RoutedTo::Cheap);
                    }
                    result
                }
            }
        }
    }

    /// Tool use always goes to the primary model for reliable structured output.
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);
        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            model = %self.primary.model_name(),
            "Smart routing: Tool use -> primary model (always)"
        );
        let result = self.primary.complete_with_tools(request).await;
        if result.is_ok() {
            self.bind_route_to_current_task(RoutedTo::Primary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;
    use crate::testing::StubLlm;

    fn default_config() -> SmartRoutingConfig {
        SmartRoutingConfig::default()
    }

    // -- Classification tests --

    #[test]
    fn classify_empty_message_as_simple() {
        assert_eq!(
            classify_message("", &default_config()),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classify_greeting_as_simple() {
        assert_eq!(
            classify_message("hello", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("hi there", &default_config()),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classify_short_question_with_simple_keyword() {
        assert_eq!(
            classify_message("what is the status?", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("show me the list", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("help", &default_config()),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classify_yes_no_as_simple() {
        assert_eq!(
            classify_message("yes", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("no", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("ok", &default_config()),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classify_code_generation_as_complex() {
        assert_eq!(
            classify_message("implement a binary search function", &default_config()),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_message("refactor the auth module", &default_config()),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_message("debug this error", &default_config()),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn classify_code_blocks_as_complex() {
        let msg = "What does this do?\n```rust\nfn main() {}\n```";
        assert_eq!(
            classify_message(msg, &default_config()),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn classify_long_message_as_complex() {
        let long_msg = "a ".repeat(600); // 1200 chars
        assert_eq!(
            classify_message(&long_msg, &default_config()),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn classify_medium_message_without_keywords_as_moderate() {
        // > 10 chars, < 1000 chars, no simple or complex keywords
        let msg = "Tell me about the weather patterns in the Pacific Ocean during summer months";
        assert_eq!(
            classify_message(msg, &default_config()),
            TaskComplexity::Moderate
        );
    }

    #[test]
    fn classify_very_short_unknown_as_simple() {
        // <= 10 chars, no keywords
        assert_eq!(
            classify_message("foo", &default_config()),
            TaskComplexity::Simple
        );
    }

    // -- Uncertainty detection tests --

    #[test]
    fn detects_uncertain_short_response() {
        let response = CompletionResponse {
            content: "I'm not sure.".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: crate::llm::FinishReason::Stop,
            cached: false,
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn detects_empty_response_as_uncertain() {
        let response = CompletionResponse {
            content: "".to_string(),
            input_tokens: 10,
            output_tokens: 0,
            finish_reason: crate::llm::FinishReason::Stop,
            cached: false,
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn short_confident_response_is_not_uncertain() {
        let response = CompletionResponse {
            content: "Yes.".to_string(),
            input_tokens: 10,
            output_tokens: 1,
            finish_reason: crate::llm::FinishReason::Stop,
            cached: false,
        };
        assert!(!SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn confident_response_is_not_uncertain() {
        let response = CompletionResponse {
            content: "The answer is 42. This is a well-known constant from the Hitchhiker's Guide."
                .to_string(),
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: crate::llm::FinishReason::Stop,
            cached: false,
        };
        assert!(!SmartRoutingProvider::response_is_uncertain(&response));
    }

    // -- Routing tests --

    fn make_request(content: &str) -> CompletionRequest {
        CompletionRequest::new(vec![ChatMessage::user(content)])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![ChatMessage::user("implement a search")], vec![])
    }

    #[tokio::test]
    async fn simple_task_routes_to_cheap() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        let resp = router.complete(make_request("hello")).await.unwrap();
        assert_eq!(resp.content, "cheap-response");
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);
    }

    #[tokio::test]
    async fn complex_task_routes_to_primary() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(primary.clone(), cheap.clone(), default_config());

        let resp = router
            .complete(make_request("implement a binary search"))
            .await
            .unwrap();
        assert_eq!(resp.content, "primary-response");
        assert_eq!(primary.calls(), 1);
        assert_eq!(cheap.calls(), 0);
    }

    #[tokio::test]
    async fn tool_use_always_routes_to_primary() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(primary.clone(), cheap.clone(), default_config());

        let resp = router
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();
        assert_eq!(resp.content, Some("primary-response".to_string()));
        assert_eq!(primary.calls(), 1);
        assert_eq!(cheap.calls(), 0);
    }

    #[tokio::test]
    async fn stats_increment_correctly() {
        let primary = Arc::new(StubLlm::new("primary").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        // Simple -> cheap
        router.complete(make_request("hello")).await.unwrap();
        // Complex -> primary
        router
            .complete(make_request("implement a search"))
            .await
            .unwrap();
        // Tool use -> primary
        router
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        let stats = router.stats();
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.cheap_requests, 1);
        assert_eq!(stats.primary_requests, 2);
        assert_eq!(stats.cascade_escalations, 0);
    }

    #[tokio::test]
    async fn cascade_escalates_on_uncertain_response() {
        // Cheap model returns an uncertain response
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("I'm not sure about that.").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: true,
                ..default_config()
            },
        );

        // A moderate task (no simple/complex keywords, medium length)
        let resp = router
            .complete(make_request(
                "Tell me about the weather patterns in the Pacific Ocean during summer months",
            ))
            .await
            .unwrap();

        // Should have escalated to primary
        assert_eq!(resp.content, "primary-response");
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 1);

        let stats = router.stats();
        assert_eq!(stats.cascade_escalations, 1);
    }

    #[tokio::test]
    async fn cascade_does_not_escalate_on_confident_response() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(
            StubLlm::new(
                "The Pacific Ocean weather patterns during summer are characterized by trade winds.",
            )
            .with_model_name("cheap"),
        );

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: true,
                ..default_config()
            },
        );

        let resp = router
            .complete(make_request(
                "Tell me about the weather patterns in the Pacific Ocean during summer months",
            ))
            .await
            .unwrap();

        // Should NOT have escalated
        assert!(resp.content.contains("Pacific Ocean"));
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);

        let stats = router.stats();
        assert_eq!(stats.cascade_escalations, 0);
    }

    #[tokio::test]
    async fn model_name_returns_primary() {
        let primary = Arc::new(StubLlm::new("ok").with_model_name("sonnet"));
        let cheap = Arc::new(StubLlm::new("ok").with_model_name("haiku"));

        let router = SmartRoutingProvider::new(primary, cheap, default_config());
        assert_eq!(router.model_name(), "sonnet");
        assert_eq!(router.active_model_name(), "sonnet");
    }

    /// Regression test for C3: effective_model_name must report the model
    /// that *actually* handled the request, not always the primary.
    /// Before the fix, SmartRoutingProvider delegated effective_model_name()
    /// to self.primary unconditionally, so a haiku-routed request was
    /// reported as sonnet.
    #[tokio::test]
    async fn effective_model_name_reports_cheap_model_for_simple_request() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("sonnet"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("haiku"));

        let router = SmartRoutingProvider::new(
            primary,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        // "hello" is classified as Simple → routes to cheap model
        let resp = router.complete(make_request("hello")).await.unwrap();
        assert_eq!(resp.content, "cheap-response");

        // REGRESSION: Before fix, this returned "sonnet" (primary model)
        let effective = router.effective_model_name(None);
        assert_eq!(
            effective, "haiku",
            "Expected cheap model 'haiku' but got '{effective}'"
        );
    }

    /// C3: effective_model_name reports primary for complex requests.
    #[tokio::test]
    async fn effective_model_name_reports_primary_for_complex_request() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("sonnet"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("haiku"));

        let router = SmartRoutingProvider::new(primary, cheap, default_config());

        let resp = router
            .complete(make_request("implement a binary search"))
            .await
            .unwrap();
        assert_eq!(resp.content, "primary-response");

        let effective = router.effective_model_name(None);
        assert_eq!(effective, "sonnet");
    }

    /// C3: cascade escalation reports the primary model.
    #[tokio::test]
    async fn effective_model_name_reports_primary_after_cascade_escalation() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("sonnet"));
        let cheap = Arc::new(StubLlm::new("I'm not sure about that.").with_model_name("haiku"));

        let router = SmartRoutingProvider::new(
            primary,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: true,
                ..default_config()
            },
        );

        // Moderate task → cheap → uncertain → escalates to primary
        let resp = router
            .complete(make_request(
                "Tell me about the weather patterns in the Pacific Ocean during summer months",
            ))
            .await
            .unwrap();
        assert_eq!(resp.content, "primary-response");

        let effective = router.effective_model_name(None);
        assert_eq!(effective, "sonnet");
    }

    // L2: routed_for_task capacity guard evicts stale entries when map exceeds threshold.
    #[test]
    fn routed_for_task_evicts_when_capacity_exceeded() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(primary, cheap, default_config());

        // Verify map starts empty.
        assert_eq!(router.routed_for_task.lock().unwrap().len(), 0);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let router = Arc::new(router);

            // Spawn tasks that insert but never call effective_model_name (simulating leak).
            let mut handles = Vec::new();
            for _ in 0..SmartRoutingProvider::TASK_MAP_CAPACITY {
                let r = Arc::clone(&router);
                handles.push(tokio::spawn(async move {
                    r.bind_route_to_current_task(RoutedTo::Primary);
                }));
            }
            for h in handles {
                h.await.unwrap();
            }

            // Map should be at or near capacity.
            let len_before = router.routed_for_task.lock().unwrap().len();
            assert!(len_before > 0, "Expected entries in map, got 0");

            // One more insert should trigger eviction.
            let r = Arc::clone(&router);
            tokio::spawn(async move {
                r.bind_route_to_current_task(RoutedTo::Cheap);
            })
            .await
            .unwrap();

            // After eviction, map should have just the one new entry.
            let len_after = router.routed_for_task.lock().unwrap().len();
            assert!(
                len_after <= 1,
                "Expected map to be evicted to at most 1 entry, got {len_after}"
            );
        });
    }
}
