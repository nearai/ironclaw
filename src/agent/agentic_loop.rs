//! Unified agentic loop engine.
//!
//! Provides a single implementation of the core LLM call → tool execution →
//! result processing → context update → repeat cycle. Three consumers
//! (chat dispatcher, job worker, container runtime) customize behavior
//! via the `LoopDelegate` trait.

use async_trait::async_trait;
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::agent::session::PendingApproval;
use crate::error::Error;
use crate::llm::{
    ChatMessage, FinishReason, Reasoning, ReasoningContext, RespondResult, ResponseMetadata,
    ToolCall,
};

/// Signal from the delegate indicating how the loop should proceed.
pub enum LoopSignal {
    /// Continue normally.
    Continue,
    /// Stop the loop gracefully.
    Stop,
    /// Inject a user message into context and continue.
    InjectMessage(String),
}

/// Outcome of a text response from the LLM.
pub enum TextAction {
    /// Return this as the final loop result.
    Return(LoopOutcome),
    /// Continue the loop (text was handled but loop should proceed).
    Continue,
}

/// Final outcome of the agentic loop.
pub enum LoopOutcome {
    /// Completed with a text response.
    Response(String),
    /// Loop was stopped by a signal.
    Stopped,
    /// Max iterations exceeded.
    MaxIterations,
    /// Loop terminated early with a clear failure reason.
    Failure(String),
    /// A tool requires user approval before continuing (chat delegate only).
    NeedApproval(Box<PendingApproval>),
    /// Auth flow initiated — config card already sent, suppress text response.
    AuthPending(String),
}

/// Configuration for the agentic loop.
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
    pub enable_tool_intent_nudge: bool,
    pub max_tool_intent_nudges: u32,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
        }
    }
}

/// Strategy trait — each consumer implements this to customize I/O and lifecycle.
///
/// The shared loop calls these methods at well-defined points. Consumers
/// implement only the behavior that differs between chat, job, and container
/// contexts. The loop itself handles the common logic: tool intent nudge,
/// iteration counting, tool definition refresh, and the respond → execute → process cycle.
///
/// # `Send + Sync` requirement
///
/// This trait requires `Send + Sync` because the loop accepts `&dyn LoopDelegate`.
/// Delegates using borrowed references (e.g. `ChatDelegate<'a>`) must ensure all
/// borrowed fields are `Send + Sync`. This is a load-bearing constraint: if a
/// delegate needs to be spawned into a detached task, it must use `Arc`-based
/// ownership instead of borrows (as `JobDelegate` and `ContainerDelegate` do).
#[async_trait]
pub trait LoopDelegate: Send + Sync {
    /// Called at the start of each iteration. Check for external signals
    /// (cancellation, user messages, stop requests).
    async fn check_signals(&self) -> LoopSignal;

    /// Called before the LLM call. Allows the delegate to refresh tool
    /// definitions, enforce cost guards, or inject messages.
    /// Return `Some(outcome)` to break the loop early.
    async fn before_llm_call(
        &self,
        reason_ctx: &mut ReasoningContext,
        iteration: usize,
    ) -> Option<LoopOutcome>;

    /// Call the LLM and return the result. Delegates own the LLM call
    /// to handle consumer-specific concerns (rate limiting, auto-compaction,
    /// cost tracking, force_text mode).
    async fn call_llm(
        &self,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        iteration: usize,
    ) -> Result<crate::llm::RespondOutput, Error>;

    /// Handle a text-only response from the LLM.
    /// Return `TextAction::Return` to exit the loop, `TextAction::Continue` to proceed.
    async fn handle_text_response(
        &self,
        text: &str,
        metadata: ResponseMetadata,
        reason_ctx: &mut ReasoningContext,
    ) -> TextAction;

    /// Execute tool calls and add results to context.
    /// Return `Some(outcome)` to break the loop (e.g. approval needed).
    ///
    /// Implementations should call `reason_ctx.set_last_tool_batch_all_failed(true/false)`
    /// to report whether every tool in the batch failed. This enables the
    /// duplicate tool call detector to escalate repeated identical failures.
    async fn execute_tool_calls(
        &self,
        tool_calls: Vec<crate::llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
        reasoning: Option<String>,
    ) -> Result<Option<LoopOutcome>, Error>;

    /// Called when the LLM expresses tool intent without actually calling a tool.
    /// Delegates can use this to emit events or log the nudge for observability.
    async fn on_tool_intent_nudge(&self, _text: &str, _reason_ctx: &mut ReasoningContext) {}

    /// Called after each successful iteration (no error, no early return).
    async fn after_iteration(&self, _iteration: usize) {}
}

/// Warning injected after repeated identical failing tool calls.
const DUPLICATE_TOOL_CALL_WARNING: &str = "\
You have repeated the exact same failing tool call multiple times. \
The tool is returning the same error each time. \
Please try a DIFFERENT approach — use different parameters, a different tool, \
or explain to the user what went wrong and what alternatives exist.";

/// Threshold: after this many consecutive duplicate failures, inject a warning.
const DUPLICATE_WARNING_THRESHOLD: u32 = 3;

/// Threshold: after this many consecutive duplicate failures, force text-only mode.
const DUPLICATE_FORCE_TEXT_THRESHOLD: u32 = 5;

/// Tracks repeated identical tool calls that all fail, and escalates.
///
/// Fingerprints each batch of tool calls by hashing `(tool_name, canonicalized_args)`
/// for every call in the batch. If the fingerprint matches the previous batch AND
/// every tool in the batch produced an error result, the consecutive counter increments.
/// Resets when the LLM calls different tools, any tool succeeds, or a text response
/// is produced.
struct DuplicateToolCallTracker {
    last_fingerprint: Option<u64>,
    consecutive_count: u32,
}

impl DuplicateToolCallTracker {
    fn new() -> Self {
        Self {
            last_fingerprint: None,
            consecutive_count: 0,
        }
    }

    /// Compute a fingerprint for a batch of tool calls.
    fn fingerprint(tool_calls: &[ToolCall]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for tc in tool_calls {
            tc.name.hash(&mut hasher);
            let canonical = crate::util::canonicalize_json_value(tc.arguments.clone());
            canonical.to_string().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Record a pre-computed fingerprint and whether all tools failed.
    /// Returns the current consecutive duplicate failure count after this update.
    fn record_with_fingerprint(&mut self, fp: u64, all_failed: bool) -> u32 {
        if all_failed && self.last_fingerprint == Some(fp) {
            self.consecutive_count += 1;
        } else if all_failed {
            // Different tool calls but still all failed — start a new streak
            self.last_fingerprint = Some(fp);
            self.consecutive_count = 1;
        } else {
            // At least one tool succeeded — reset
            self.reset();
            self.last_fingerprint = Some(fp);
        }

        self.consecutive_count
    }

    /// Reset when the LLM produces a text response or calls different tools successfully.
    fn reset(&mut self) {
        self.last_fingerprint = None;
        self.consecutive_count = 0;
    }
}

/// Run the unified agentic loop.
///
/// This is the single implementation used by all three consumers (chat, job, container).
/// The `delegate` provides consumer-specific behavior via the `LoopDelegate` trait.
pub async fn run_agentic_loop(
    delegate: &dyn LoopDelegate,
    reasoning: &Reasoning,
    reason_ctx: &mut ReasoningContext,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, Error> {
    let mut consecutive_tool_intent_nudges: u32 = 0;
    // Accumulates across all iterations (not reset by text responses) so
    // non-consecutive truncations still escalate to force_text.
    let mut truncation_count: u32 = 0;
    let mut dup_tracker = DuplicateToolCallTracker::new();

    for iteration in 1..=config.max_iterations {
        // Check for external signals (stop, cancellation, user messages)
        match delegate.check_signals().await {
            LoopSignal::Continue => {}
            LoopSignal::Stop => return Ok(LoopOutcome::Stopped),
            LoopSignal::InjectMessage(msg) => {
                reason_ctx.messages.push(ChatMessage::user(&msg));
            }
        }

        // Pre-LLM call hook (cost guard, tool refresh, iteration limit nudge)
        if let Some(outcome) = delegate.before_llm_call(reason_ctx, iteration).await {
            return Ok(outcome);
        }

        // Call LLM
        let output = delegate.call_llm(reasoning, reason_ctx, iteration).await?;

        match &output.result {
            RespondResult::Text(text) => {
                tracing::debug!(
                    iteration,
                    len = text.len(),
                    has_suggestions = text.contains("<suggestions>"),
                    response = %text,
                    "LLM text response"
                );
            }
            RespondResult::ToolCalls {
                tool_calls,
                content,
                reasoning: _,
            } => {
                let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                tracing::debug!(
                    iteration,
                    tools = ?names,
                    has_content = content.is_some(),
                    "LLM tool_calls response"
                );
            }
        }

        match output.result {
            RespondResult::Text(text) => {
                // Tool intent nudge: if the LLM says "let me search..." without
                // actually calling a tool, inject a nudge message.
                if config.enable_tool_intent_nudge
                    && !reason_ctx.available_tools.is_empty()
                    && !reason_ctx.force_text
                    && consecutive_tool_intent_nudges < config.max_tool_intent_nudges
                    && crate::llm::llm_signals_tool_intent(&text)
                {
                    consecutive_tool_intent_nudges += 1;
                    tracing::info!(
                        iteration,
                        "LLM expressed tool intent without calling a tool, nudging"
                    );
                    delegate.on_tool_intent_nudge(&text, reason_ctx).await;
                    reason_ctx.messages.push(ChatMessage::assistant(&text));
                    reason_ctx
                        .messages
                        .push(ChatMessage::user(crate::llm::TOOL_INTENT_NUDGE));
                    delegate.after_iteration(iteration).await;
                    continue;
                }

                // Reset nudge counter since we got a non-intent text response
                if !crate::llm::llm_signals_tool_intent(&text) {
                    consecutive_tool_intent_nudges = 0;
                }

                // Text response breaks any duplicate tool call streak.
                dup_tracker.reset();

                match delegate
                    .handle_text_response(&text, output.metadata, reason_ctx)
                    .await
                {
                    TextAction::Return(outcome) => return Ok(outcome),
                    TextAction::Continue => {}
                }
            }
            RespondResult::ToolCalls {
                tool_calls,
                content,
                reasoning,
            } => {
                // If the response was truncated, tool call parameters are likely
                // incomplete. Discard them and tell the LLM to try a different
                // approach rather than executing malformed tool calls.
                if output.finish_reason == FinishReason::Length {
                    truncation_count += 1;
                    let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                    tracing::warn!(
                        iteration,
                        tools = ?names,
                        truncation_count,
                        "Discarding truncated tool calls (finish_reason=Length)"
                    );
                    if let Some(ref text) = content {
                        reason_ctx.messages.push(ChatMessage::assistant(text));
                    }
                    reason_ctx
                        .messages
                        .push(ChatMessage::user(crate::llm::TRUNCATED_TOOL_CALL_NOTICE));
                    // After repeated truncations, force text-only mode so the LLM
                    // stops attempting tool calls it can't fit in the output budget.
                    if truncation_count >= 3 {
                        reason_ctx.force_text = true;
                    }
                    delegate.after_iteration(iteration).await;
                    continue;
                }

                consecutive_tool_intent_nudges = 0;
                truncation_count = 0;

                // Fingerprint before execution (avoids cloning the full Vec).
                let batch_fingerprint = DuplicateToolCallTracker::fingerprint(&tool_calls);

                // Reset the flag before execution; delegates set it in execute_tool_calls.
                reason_ctx.last_tool_batch_all_failed = false;

                if let Some(outcome) = delegate
                    .execute_tool_calls(tool_calls, content, reason_ctx, reasoning)
                    .await?
                {
                    return Ok(outcome);
                }

                // Track duplicate failing tool calls and escalate.
                let dup_count = dup_tracker.record_with_fingerprint(
                    batch_fingerprint,
                    reason_ctx.last_tool_batch_all_failed,
                );
                if dup_count >= DUPLICATE_FORCE_TEXT_THRESHOLD {
                    tracing::debug!(
                        iteration,
                        dup_count,
                        "Repeated duplicate failing tool calls — forcing text mode"
                    );
                    reason_ctx.force_text = true;
                    reason_ctx
                        .messages
                        .push(ChatMessage::user(DUPLICATE_TOOL_CALL_WARNING));
                } else if dup_count >= DUPLICATE_WARNING_THRESHOLD {
                    tracing::debug!(
                        iteration,
                        dup_count,
                        "Repeated duplicate failing tool calls — injecting warning"
                    );
                    reason_ctx
                        .messages
                        .push(ChatMessage::user(DUPLICATE_TOOL_CALL_WARNING));
                }
            }
        }

        delegate.after_iteration(iteration).await;
    }

    Ok(LoopOutcome::MaxIterations)
}

/// Truncate a string for log/status previews.
///
/// `max` is a byte budget. The result is truncated at the last valid char
/// boundary at or before `max` bytes, so it is always valid UTF-8.
pub fn truncate_for_preview(s: &str, max: usize) -> Cow<'_, str> {
    if s.len() <= max {
        Cow::Borrowed(s)
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        Cow::Owned(format!("{}...", &s[..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{RespondOutput, ResponseAnomaly, ResponseMetadata, TokenUsage, ToolCall};
    use crate::testing::StubLlm;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    fn stub_reasoning() -> Reasoning {
        Reasoning::new(Arc::new(StubLlm::default()))
    }

    fn zero_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }

    fn text_output(text: &str) -> RespondOutput {
        RespondOutput {
            result: RespondResult::Text(text.to_string()),
            usage: zero_usage(),
            finish_reason: FinishReason::Stop,
            metadata: ResponseMetadata::default(),
        }
    }

    fn tool_calls_output(calls: Vec<ToolCall>) -> RespondOutput {
        RespondOutput {
            result: RespondResult::ToolCalls {
                tool_calls: calls,
                content: None,
                reasoning: None,
            },
            usage: zero_usage(),
            finish_reason: FinishReason::ToolUse,
            metadata: ResponseMetadata::default(),
        }
    }

    /// Configurable mock delegate for testing run_agentic_loop.
    struct MockDelegate {
        signal: Mutex<LoopSignal>,
        llm_responses: Mutex<Vec<RespondOutput>>,
        tool_exec_count: AtomicUsize,
        tool_exec_outcome: Mutex<Option<LoopOutcome>>,
        iterations_seen: Mutex<Vec<usize>>,
        early_exit: Mutex<Option<(usize, LoopOutcome)>>,
        nudge_count: AtomicUsize,
        /// When true, execute_tool_calls sets last_tool_batch_all_failed = true.
        simulate_all_failed: bool,
    }

    impl MockDelegate {
        fn new(responses: Vec<RespondOutput>) -> Self {
            Self {
                signal: Mutex::new(LoopSignal::Continue),
                llm_responses: Mutex::new(responses),
                tool_exec_count: AtomicUsize::new(0),
                tool_exec_outcome: Mutex::new(None),
                iterations_seen: Mutex::new(Vec::new()),
                early_exit: Mutex::new(None),
                nudge_count: AtomicUsize::new(0),
                simulate_all_failed: false,
            }
        }

        fn with_signal(mut self, signal: LoopSignal) -> Self {
            self.signal = Mutex::new(signal);
            self
        }

        fn with_early_exit(mut self, iteration: usize, outcome: LoopOutcome) -> Self {
            self.early_exit = Mutex::new(Some((iteration, outcome)));
            self
        }
    }

    #[async_trait]
    impl LoopDelegate for MockDelegate {
        async fn check_signals(&self) -> LoopSignal {
            let mut sig = self.signal.lock().await;
            std::mem::replace(&mut *sig, LoopSignal::Continue)
        }

        async fn before_llm_call(
            &self,
            _reason_ctx: &mut ReasoningContext,
            iteration: usize,
        ) -> Option<LoopOutcome> {
            let mut guard = self.early_exit.lock().await;
            let should_take = guard
                .as_ref()
                .is_some_and(|(target, _)| *target == iteration);
            if should_take {
                guard.take().map(|(_, o)| o)
            } else {
                None
            }
        }

        async fn call_llm(
            &self,
            _reasoning: &Reasoning,
            _reason_ctx: &mut ReasoningContext,
            _iteration: usize,
        ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
            let mut responses = self.llm_responses.lock().await;
            if responses.is_empty() {
                panic!("MockDelegate: no more LLM responses queued");
            }
            Ok(responses.remove(0))
        }

        async fn handle_text_response(
            &self,
            text: &str,
            _metadata: ResponseMetadata,
            _reason_ctx: &mut ReasoningContext,
        ) -> TextAction {
            TextAction::Return(LoopOutcome::Response(text.to_string()))
        }

        async fn execute_tool_calls(
            &self,
            _tool_calls: Vec<ToolCall>,
            _content: Option<String>,
            reason_ctx: &mut ReasoningContext,
            _reasoning: Option<String>,
        ) -> Result<Option<LoopOutcome>, crate::error::Error> {
            self.tool_exec_count.fetch_add(1, Ordering::SeqCst);
            reason_ctx
                .messages
                .push(ChatMessage::user("tool result stub"));
            reason_ctx.last_tool_batch_all_failed = self.simulate_all_failed;
            let outcome = self.tool_exec_outcome.lock().await.take();
            Ok(outcome)
        }

        async fn on_tool_intent_nudge(&self, _text: &str, _reason_ctx: &mut ReasoningContext) {
            self.nudge_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn after_iteration(&self, iteration: usize) {
            self.iterations_seen.lock().await.push(iteration);
        }
    }

    // --- Tests ---

    #[tokio::test]
    async fn test_text_response_returns_immediately() {
        let delegate = MockDelegate::new(vec![text_output("Hello, world!")]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        match outcome {
            LoopOutcome::Response(text) => assert_eq!(text, "Hello, world!"),
            _ => panic!("Expected LoopOutcome::Response"),
        }
        // after_iteration is NOT called when handle_text_response returns Return
        // (the loop exits before reaching after_iteration).
        assert!(delegate.iterations_seen.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_tool_call_then_text_response() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
            reasoning: None,
            signature: None,
        };
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![tool_call]),
            text_output("Done!"),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        match outcome {
            LoopOutcome::Response(text) => assert_eq!(text, "Done!"),
            _ => panic!("Expected LoopOutcome::Response"),
        }
        assert_eq!(delegate.tool_exec_count.load(Ordering::SeqCst), 1);
        // after_iteration called for iteration 1 (tool call), but not 2
        // (text response exits before after_iteration).
        assert_eq!(*delegate.iterations_seen.lock().await, vec![1]);
    }

    #[tokio::test]
    async fn test_stop_signal_exits_immediately() {
        let delegate =
            MockDelegate::new(vec![text_output("unreachable")]).with_signal(LoopSignal::Stop);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Stopped));
        assert!(delegate.iterations_seen.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_inject_message_adds_user_message() {
        let delegate = MockDelegate::new(vec![text_output("Got it")])
            .with_signal(LoopSignal::InjectMessage("injected prompt".to_string()));
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.role == crate::llm::Role::User && m.content.contains("injected prompt")),
            "Injected message should appear in context"
        );
    }

    #[tokio::test]
    async fn test_text_response_metadata_can_fail_fast() {
        struct FailOnMalformedResponse;

        #[async_trait]
        impl LoopDelegate for FailOnMalformedResponse {
            async fn check_signals(&self) -> LoopSignal {
                LoopSignal::Continue
            }

            async fn before_llm_call(
                &self,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Option<LoopOutcome> {
                None
            }

            async fn call_llm(
                &self,
                _: &Reasoning,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
                Ok(RespondOutput {
                    result: RespondResult::Text("fallback".to_string()),
                    usage: zero_usage(),
                    finish_reason: FinishReason::Stop,
                    metadata: ResponseMetadata {
                        anomaly: Some(ResponseAnomaly::EmptyToolCompletion),
                    },
                })
            }

            async fn handle_text_response(
                &self,
                _: &str,
                metadata: ResponseMetadata,
                _: &mut ReasoningContext,
            ) -> TextAction {
                assert_eq!(metadata.anomaly, Some(ResponseAnomaly::EmptyToolCompletion));
                TextAction::Return(LoopOutcome::Failure(
                    "malformed tool completion".to_string(),
                ))
            }

            async fn execute_tool_calls(
                &self,
                _: Vec<ToolCall>,
                _: Option<String>,
                _: &mut ReasoningContext,
                _: Option<String>,
            ) -> Result<Option<LoopOutcome>, crate::error::Error> {
                Ok(None)
            }
        }

        let delegate = FailOnMalformedResponse;
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let outcome = run_agentic_loop(
            &delegate,
            &reasoning,
            &mut ctx,
            &AgenticLoopConfig::default(),
        )
        .await
        .unwrap();

        assert!(
            matches!(outcome, LoopOutcome::Failure(ref reason) if reason == "malformed tool completion")
        );
    }

    #[tokio::test]
    async fn test_max_iterations_reached() {
        struct ContinueDelegate;

        #[async_trait]
        impl LoopDelegate for ContinueDelegate {
            async fn check_signals(&self) -> LoopSignal {
                LoopSignal::Continue
            }
            async fn before_llm_call(
                &self,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Option<LoopOutcome> {
                None
            }
            async fn call_llm(
                &self,
                _: &Reasoning,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
                Ok(text_output("still working"))
            }
            async fn handle_text_response(
                &self,
                _: &str,
                _: ResponseMetadata,
                ctx: &mut ReasoningContext,
            ) -> TextAction {
                ctx.messages.push(ChatMessage::assistant("still working"));
                TextAction::Continue
            }
            async fn execute_tool_calls(
                &self,
                _: Vec<ToolCall>,
                _: Option<String>,
                _: &mut ReasoningContext,
                _: Option<String>,
            ) -> Result<Option<LoopOutcome>, crate::error::Error> {
                Ok(None)
            }
        }

        let delegate = ContinueDelegate;
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 3,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::MaxIterations));
        let assistant_count = ctx
            .messages
            .iter()
            .filter(|m| m.role == crate::llm::Role::Assistant)
            .count();
        assert_eq!(assistant_count, 3);
    }

    #[tokio::test]
    async fn test_tool_intent_nudge_fires_and_caps() {
        let delegate = MockDelegate::new(vec![
            text_output("Let me search for that file"),
            text_output("Let me search for that file"),
            text_output("Let me search for that file"),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        ctx.available_tools.push(crate::llm::ToolDefinition {
            name: "search".to_string(),
            description: "Search files".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        });
        let config = AgenticLoopConfig {
            max_iterations: 10,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert_eq!(delegate.nudge_count.load(Ordering::SeqCst), 2);
        let nudge_messages = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == crate::llm::Role::User
                    && m.content.contains("you did not include any tool calls")
            })
            .count();
        assert_eq!(
            nudge_messages, 2,
            "Should have exactly 2 nudge messages in context"
        );
    }

    #[tokio::test]
    async fn test_before_llm_call_early_exit() {
        let delegate = MockDelegate::new(vec![text_output("unreachable")])
            .with_early_exit(1, LoopOutcome::Stopped);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Stopped));
        assert!(delegate.iterations_seen.lock().await.is_empty());
    }

    #[test]
    fn test_truncate_short_string_unchanged() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_short_string_borrows() {
        let result = truncate_for_preview("hello", 10);
        assert!(matches!(result, Cow::Borrowed("hello")));
    }

    #[test]
    fn test_truncate_long_string_adds_ellipsis() {
        let result = truncate_for_preview("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_long_string_owns() {
        let result = truncate_for_preview("hello world", 5);
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        let result = truncate_for_preview("café", 4);
        assert_eq!(result, "caf...");
    }

    #[tokio::test]
    async fn test_truncated_tool_calls_discarded_on_length() {
        let truncated_tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "memory_write".to_string(),
            arguments: serde_json::json!({}), // empty — truncated
            reasoning: None,
            signature: None,
        };
        let truncated_output = RespondOutput {
            result: RespondResult::ToolCalls {
                tool_calls: vec![truncated_tool_call],
                content: Some("I'll write the report.".to_string()),
                reasoning: None,
            },
            usage: zero_usage(),
            finish_reason: FinishReason::Length, // response was truncated
            metadata: ResponseMetadata::default(),
        };
        let delegate = MockDelegate::new(vec![truncated_output, text_output("Summarized it.")]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 5,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        // Tool calls should NOT have been executed
        assert_eq!(delegate.tool_exec_count.load(Ordering::SeqCst), 0);
        // The loop should have continued and returned the text response
        assert!(matches!(outcome, LoopOutcome::Response(ref t) if t == "Summarized it."));
        // A truncation notice should have been injected into context
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.role == crate::llm::Role::User && m.content.contains("truncated")),
            "Should inject truncation notice into context"
        );
        // The partial assistant content should have been preserved
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.role == crate::llm::Role::Assistant
                    && m.content.contains("write the report")),
            "Should preserve partial assistant content"
        );
    }

    #[tokio::test]
    async fn test_repeated_truncations_force_text_mode() {
        let make_truncated = || RespondOutput {
            result: RespondResult::ToolCalls {
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "memory_write".to_string(),
                    arguments: serde_json::json!({}),
                    reasoning: None,
                    signature: None,
                }],
                content: None,
                reasoning: None,
            },
            usage: zero_usage(),
            finish_reason: FinishReason::Length,
            metadata: ResponseMetadata::default(),
        };
        // Three truncated responses, then a text response
        let delegate = MockDelegate::new(vec![
            make_truncated(),
            make_truncated(),
            make_truncated(),
            text_output("Gave up on tool calls."),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 5,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert_eq!(delegate.tool_exec_count.load(Ordering::SeqCst), 0);
        // After 3 truncations, force_text should be set
        assert!(
            ctx.force_text,
            "Should escalate to force_text after repeated truncations"
        );
    }

    // --- Duplicate tool call tracker unit tests ---

    #[test]
    fn test_dup_tracker_no_duplicates_when_tools_succeed() {
        let mut tracker = DuplicateToolCallTracker::new();
        let calls = vec![ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"msg": "hi"}),
            reasoning: None,
            signature: None,
        }];
        let fp = DuplicateToolCallTracker::fingerprint(&calls);
        // Tool succeeded — count stays at 0
        assert_eq!(tracker.record_with_fingerprint(fp, false), 0);
        assert_eq!(tracker.record_with_fingerprint(fp, false), 0);
    }

    #[test]
    fn test_dup_tracker_counts_identical_failures() {
        let mut tracker = DuplicateToolCallTracker::new();
        let calls = vec![ToolCall {
            id: "c1".into(),
            name: "http_get".into(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
            signature: None,
        }];
        let fp = DuplicateToolCallTracker::fingerprint(&calls);
        assert_eq!(tracker.record_with_fingerprint(fp, true), 1);
        assert_eq!(tracker.record_with_fingerprint(fp, true), 2);
        assert_eq!(tracker.record_with_fingerprint(fp, true), 3);
    }

    #[test]
    fn test_dup_tracker_resets_on_success() {
        let mut tracker = DuplicateToolCallTracker::new();
        let calls = vec![ToolCall {
            id: "c1".into(),
            name: "http_get".into(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
            signature: None,
        }];
        let fp = DuplicateToolCallTracker::fingerprint(&calls);
        assert_eq!(tracker.record_with_fingerprint(fp, true), 1);
        assert_eq!(tracker.record_with_fingerprint(fp, true), 2);
        // Success resets the tracker
        assert_eq!(tracker.record_with_fingerprint(fp, false), 0);
        // Starts fresh
        assert_eq!(tracker.record_with_fingerprint(fp, true), 1);
    }

    #[test]
    fn test_dup_tracker_different_args_restarts_streak() {
        let mut tracker = DuplicateToolCallTracker::new();
        let calls_a = vec![ToolCall {
            id: "c1".into(),
            name: "http_get".into(),
            arguments: serde_json::json!({"url": "https://a.com"}),
            reasoning: None,
            signature: None,
        }];
        let calls_b = vec![ToolCall {
            id: "c1".into(),
            name: "http_get".into(),
            arguments: serde_json::json!({"url": "https://b.com"}),
            reasoning: None,
            signature: None,
        }];
        let fp_a = DuplicateToolCallTracker::fingerprint(&calls_a);
        let fp_b = DuplicateToolCallTracker::fingerprint(&calls_b);
        assert_eq!(tracker.record_with_fingerprint(fp_a, true), 1);
        assert_eq!(tracker.record_with_fingerprint(fp_a, true), 2);
        // Different call — streak restarts at 1
        assert_eq!(tracker.record_with_fingerprint(fp_b, true), 1);
    }

    #[test]
    fn test_dup_tracker_canonicalizes_arg_order() {
        // Same keys in different insertion order should produce same fingerprint
        let calls_a = vec![ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"a": 1, "b": 2}),
            reasoning: None,
            signature: None,
        }];
        let calls_b = vec![ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"b": 2, "a": 1}),
            reasoning: None,
            signature: None,
        }];
        assert_eq!(
            DuplicateToolCallTracker::fingerprint(&calls_a),
            DuplicateToolCallTracker::fingerprint(&calls_b),
        );
    }

    // --- Agentic loop integration tests for duplicate detection ---

    #[tokio::test]
    async fn test_duplicate_failing_tool_calls_inject_warning() {
        let same_call = || ToolCall {
            id: "call_1".to_string(),
            name: "http_get".to_string(),
            arguments: serde_json::json!({"url": "https://broken.example.com"}),
            reasoning: None,
            signature: None,
        };
        // 3 identical failing tool calls, then text response
        let mut delegate = MockDelegate::new(vec![
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            text_output("I give up."),
        ]);
        delegate.simulate_all_failed = true;
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 10,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert_eq!(delegate.tool_exec_count.load(Ordering::SeqCst), 3);
        // After 3 consecutive duplicate failures, a warning should be injected
        let warning_count = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == crate::llm::Role::User && m.content.contains("same failing tool call")
            })
            .count();
        assert!(
            warning_count >= 1,
            "Should have at least 1 duplicate warning message in context"
        );
        // force_text should NOT be set yet (only at threshold 5)
        assert!(
            !ctx.force_text,
            "force_text should not be set after only 3 duplicate failures"
        );
    }

    #[tokio::test]
    async fn test_duplicate_failing_tool_calls_force_text_at_threshold() {
        let same_call = || ToolCall {
            id: "call_1".to_string(),
            name: "http_get".to_string(),
            arguments: serde_json::json!({"url": "https://broken.example.com"}),
            reasoning: None,
            signature: None,
        };
        // 5 identical failing tool calls, then text response
        let mut delegate = MockDelegate::new(vec![
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            text_output("Forced text response."),
        ]);
        delegate.simulate_all_failed = true;
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 10,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert_eq!(delegate.tool_exec_count.load(Ordering::SeqCst), 5);
        assert!(
            ctx.force_text,
            "force_text should be set after 5 consecutive duplicate failures"
        );
    }

    #[tokio::test]
    async fn test_duplicate_tracker_resets_on_text_response() {
        let call_a = || ToolCall {
            id: "call_1".to_string(),
            name: "http_get".to_string(),
            arguments: serde_json::json!({"url": "https://broken.example.com"}),
            reasoning: None,
            signature: None,
        };
        // 2 failing calls, then a text continuation, then 2 more of the same failing calls
        // The text response in the middle should reset the streak, so we never hit 3.
        struct DupResetDelegate {
            llm_responses: Mutex<Vec<RespondOutput>>,
            tool_exec_count: AtomicUsize,
            text_response_count: AtomicUsize,
        }

        #[async_trait]
        impl LoopDelegate for DupResetDelegate {
            async fn check_signals(&self) -> LoopSignal {
                LoopSignal::Continue
            }
            async fn before_llm_call(
                &self,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Option<LoopOutcome> {
                None
            }
            async fn call_llm(
                &self,
                _: &Reasoning,
                _: &mut ReasoningContext,
                _: usize,
            ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
                let mut responses = self.llm_responses.lock().await;
                if responses.is_empty() {
                    panic!("No more responses");
                }
                Ok(responses.remove(0))
            }
            async fn handle_text_response(
                &self,
                text: &str,
                _: ResponseMetadata,
                ctx: &mut ReasoningContext,
            ) -> TextAction {
                let count = self.text_response_count.fetch_add(1, Ordering::SeqCst);
                if count >= 1 {
                    // Second text response — return final result
                    TextAction::Return(LoopOutcome::Response(text.to_string()))
                } else {
                    ctx.messages.push(ChatMessage::assistant("thinking..."));
                    TextAction::Continue
                }
            }
            async fn execute_tool_calls(
                &self,
                _: Vec<ToolCall>,
                _: Option<String>,
                reason_ctx: &mut ReasoningContext,
                _reasoning: Option<String>,
            ) -> Result<Option<LoopOutcome>, crate::error::Error> {
                self.tool_exec_count.fetch_add(1, Ordering::SeqCst);
                reason_ctx.messages.push(ChatMessage::user("tool error"));
                reason_ctx.last_tool_batch_all_failed = true;
                Ok(None)
            }
        }

        let delegate = DupResetDelegate {
            llm_responses: Mutex::new(vec![
                tool_calls_output(vec![call_a()]),
                tool_calls_output(vec![call_a()]),
                text_output("Let me reconsider."),
                tool_calls_output(vec![call_a()]),
                tool_calls_output(vec![call_a()]),
                text_output("Final answer."),
            ]),
            tool_exec_count: AtomicUsize::new(0),
            text_response_count: AtomicUsize::new(0),
        };
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 10,
            ..Default::default()
        };

        let _outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        // No warning should have been injected because the text response
        // reset the streak before it hit 3.
        let warning_count = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == crate::llm::Role::User && m.content.contains("same failing tool call")
            })
            .count();
        assert_eq!(
            warning_count, 0,
            "No duplicate warning should be injected when text response resets the streak"
        );
        assert!(
            !ctx.force_text,
            "force_text should not be set when streak was reset"
        );
    }
}
