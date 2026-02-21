//! Smart routing provider that routes requests to cheap or primary models based on task complexity.
//!
//! Inspired by RelayPlane's cost-reduction approach: simple tasks (status checks, greetings,
//! short questions) go to a cheap model (e.g. Haiku), while complex tasks (code generation,
//! analysis, multi-step reasoning) go to the primary model (e.g. Sonnet/Opus).
//!
//! This is a decorator that wraps two `LlmProvider`s and implements `LlmProvider` itself,
//! following the same pattern as `RetryProvider`, `CachedProvider`, and `CircuitBreakerProvider`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, Role, ToolCompletionRequest,
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

/// Smart routing provider that classifies task complexity and routes to the appropriate model.
///
/// - `complete()` — classifies and routes to cheap or primary model
/// - `complete_with_tools()` — always routes to primary (tool use requires reliable structured output)
pub struct SmartRoutingProvider {
    primary: Arc<dyn LlmProvider>,
    cheap: Arc<dyn LlmProvider>,
    config: SmartRoutingConfig,
    stats: SmartRoutingStats,
}

impl SmartRoutingProvider {
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

    /// Check if a response from the cheap model shows uncertainty, warranting escalation.
    fn response_is_uncertain(response: &CompletionResponse) -> bool {
        let content = response.content.trim();

        // Very short responses for non-trivial questions suggest the model is struggling
        if content.len() < 20 {
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
    fn model_name(&self) -> &str {
        self.primary.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.primary.cost_per_token()
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
                self.cheap.complete(request).await
            }
            TaskComplexity::Complex => {
                tracing::debug!(
                    model = %self.primary.model_name(),
                    "Smart routing: Complex task -> primary model"
                );
                self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                self.primary.complete(request).await
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
                        self.primary.complete(request).await
                    } else {
                        Ok(response)
                    }
                } else {
                    // Without cascade, moderate tasks go to cheap model
                    tracing::debug!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade disabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                    self.cheap.complete(request).await
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
        self.primary.complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.primary.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.primary.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.primary.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.primary.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.primary.calculate_cost(input_tokens, output_tokens)
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
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn detects_very_short_response_as_uncertain() {
        let response = CompletionResponse {
            content: "ok".to_string(),
            input_tokens: 10,
            output_tokens: 1,
            finish_reason: crate::llm::FinishReason::Stop,
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn confident_response_is_not_uncertain() {
        let response = CompletionResponse {
            content: "The answer is 42. This is a well-known constant from the Hitchhiker's Guide."
                .to_string(),
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: crate::llm::FinishReason::Stop,
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
}
