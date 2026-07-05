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
use std::io;

use crate::agent::session::PendingApproval;
use crate::error::Error;
use ironclaw_llm::{
    ChatMessage, FinishReason, Reasoning, ReasoningContext, ReasoningDetails, RespondResult,
    ResponseMetadata, ToolCall,
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
    pub repetition_breaker_enabled: bool,
    pub repetition_breaker_threshold: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
            repetition_breaker_enabled: true,
            repetition_breaker_threshold: 5,
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
    ) -> Result<ironclaw_llm::RespondOutput, Error>;

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
        tool_calls: Vec<ironclaw_llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
        reasoning: Option<String>,
        reasoning_details: Option<ReasoningDetails>,
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

const REPETITION_BREAKER_ARG_PREFIX_CHARS: usize = 200;
const REPETITION_BREAKER_ARG_PREFIX_BYTES: usize = 300;

/// Corrective injected when the model repeats the same tool call.
///
/// Inspired by the LAB Meta-Harness repetition loop-breaker technique:
/// Lee et al., "Meta-Harness: End-to-End Optimization of Model Harnesses",
/// arXiv:2603.28052 (MIT).
const REPETITION_BREAKER_MESSAGE: &str = "\
You appear to be stuck repeating the same tool call. Repeating it again with \
the same inputs is unlikely to change the result. Change strategy: diagnose \
the problem differently or use a different method. Reference inputs by \
workspace-relative paths rather than absolute machine paths. If one file is \
unreadable or blocked, proceed without that single file if possible. If you \
already have enough information, produce the final answer or deliverable now.";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolCallPrefixSignature {
    name: String,
    arguments_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolCallBatchSignature {
    calls: Vec<ToolCallPrefixSignature>,
}

struct PrefixWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl PrefixWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl io::Write for PrefixWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if remaining == 0 {
            return Err(io::Error::other("json prefix budget exhausted"));
        }

        let take = remaining.min(buf.len());
        self.bytes.extend_from_slice(&buf[..take]);
        if take < buf.len() {
            return Err(io::Error::other("json prefix budget exhausted"));
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_json_prefix(value: &serde_json::Value) -> String {
    let mut writer = PrefixWriter::new(REPETITION_BREAKER_ARG_PREFIX_BYTES);
    let _ = serde_json::to_writer(&mut writer, value);
    String::from_utf8_lossy(&writer.bytes)
        .chars()
        .take(REPETITION_BREAKER_ARG_PREFIX_CHARS)
        .collect()
}

fn tool_call_prefix_signature(tool_call: &ToolCall) -> ToolCallPrefixSignature {
    ToolCallPrefixSignature {
        name: tool_call.name.clone(),
        arguments_prefix: bounded_json_prefix(&tool_call.arguments),
    }
}

fn tool_call_batch_signature(tool_calls: &[ToolCall]) -> ToolCallBatchSignature {
    ToolCallBatchSignature {
        calls: tool_calls.iter().map(tool_call_prefix_signature).collect(),
    }
}

fn tool_call_batch_names_match(
    tool_calls: &[ToolCall],
    signature: &ToolCallBatchSignature,
) -> bool {
    tool_calls.len() == signature.calls.len()
        && tool_calls
            .iter()
            .zip(&signature.calls)
            .all(|(tool_call, signed)| tool_call.name.as_str() == signed.name.as_str())
}

fn tool_call_batch_matches_signature(
    tool_calls: &[ToolCall],
    signature: &ToolCallBatchSignature,
) -> bool {
    tool_call_batch_names_match(tool_calls, signature)
        && tool_call_batch_signature(tool_calls) == *signature
}

/// Return the contiguous trailing run of identical assistant tool-call batches.
///
/// The trailing run is measured across the conversation suffix of assistant
/// messages that contain tool calls, skipping tool results and other non-tool
/// messages. Each batch signature preserves the ordered list of tool-call
/// `(name, bounded serialized-argument prefix)` pairs. Returns `None` when the
/// context contains no assistant tool-call messages.
fn trailing_tool_call_run(messages: &[ChatMessage]) -> Option<(usize, ToolCallBatchSignature)> {
    let mut trailing_signature = None;
    let mut trailing_count: usize = 0;

    for tool_calls in messages
        .iter()
        .rev()
        .filter(|message| message.role == ironclaw_llm::Role::Assistant)
        .filter_map(|message| message.tool_calls.as_ref())
        .filter(|tool_calls| !tool_calls.is_empty())
    {
        match &trailing_signature {
            None => {
                let signature = tool_call_batch_signature(tool_calls);
                trailing_signature = Some(signature);
                trailing_count = 1;
            }
            Some(existing) if tool_call_batch_matches_signature(tool_calls, existing) => {
                trailing_count += 1;
            }
            Some(_) => break,
        }
    }

    trailing_signature.map(|signature| (trailing_count, signature))
}

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
    let mut repetition_breaker_fired_for: Option<ToolCallBatchSignature> = None;

    for iteration in 1..=config.max_iterations {
        let mut suppress_repetition_breaker = false;
        // Check for external signals (stop, cancellation, user messages)
        match delegate.check_signals().await {
            LoopSignal::Continue => {}
            LoopSignal::Stop => return Ok(LoopOutcome::Stopped),
            LoopSignal::InjectMessage(msg) => {
                reason_ctx.messages.push(ChatMessage::user(&msg));
                repetition_breaker_fired_for = None;
                suppress_repetition_breaker = true;
            }
        }

        if !suppress_repetition_breaker
            && config.repetition_breaker_enabled
            && config.repetition_breaker_threshold > 0
        {
            // Compute the trailing identical-tool-call run once; inject a single
            // corrective per stuck streak (keyed on the call signature) and reset
            // the guard once the streak breaks.
            match trailing_tool_call_run(&reason_ctx.messages) {
                Some((streak_len, signature))
                    if streak_len >= config.repetition_breaker_threshold =>
                {
                    if repetition_breaker_fired_for.as_ref() != Some(&signature) {
                        tracing::info!(
                            iteration,
                            streak_len,
                            tool = ?signature,
                            "Repeated tool-call loop breaker injecting corrective message"
                        );
                        reason_ctx
                            .messages
                            .push(ChatMessage::user(REPETITION_BREAKER_MESSAGE));
                        repetition_breaker_fired_for = Some(signature);
                    }
                }
                _ => {
                    repetition_breaker_fired_for = None;
                }
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
                reasoning_details: _,
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
                    && ironclaw_llm::llm_signals_tool_intent(&text)
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
                        .push(ChatMessage::user(ironclaw_llm::TOOL_INTENT_NUDGE));
                    delegate.after_iteration(iteration).await;
                    continue;
                }

                // Reset nudge counter since we got a non-intent text response
                if !ironclaw_llm::llm_signals_tool_intent(&text) {
                    consecutive_tool_intent_nudges = 0;
                }

                // Text response breaks any duplicate tool call streak.
                dup_tracker.reset();
                repetition_breaker_fired_for = None;

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
                reasoning_details,
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
                        .push(ChatMessage::user(ironclaw_llm::TRUNCATED_TOOL_CALL_NOTICE));
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
                    .execute_tool_calls(
                        tool_calls,
                        content,
                        reason_ctx,
                        reasoning,
                        reasoning_details,
                    )
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
    use crate::testing::StubLlm;
    use ironclaw_llm::{RespondOutput, ResponseAnomaly, ResponseMetadata, TokenUsage, ToolCall};
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
                reasoning_details: None,
            },
            usage: zero_usage(),
            finish_reason: FinishReason::ToolUse,
            metadata: ResponseMetadata::default(),
        }
    }

    fn test_tool_call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments,
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        }
    }

    fn assistant_tool_call_message(name: &str, arguments: serde_json::Value) -> ChatMessage {
        ChatMessage::assistant_with_tool_calls(None, vec![test_tool_call(name, arguments)])
    }

    fn assistant_tool_call_batch_message(calls: Vec<ToolCall>) -> ChatMessage {
        ChatMessage::assistant_with_tool_calls(None, calls)
    }

    fn trailing_identical_tool_calls(messages: &[ChatMessage]) -> usize {
        trailing_tool_call_run(messages)
            .map(|(count, _)| count)
            .unwrap_or(0)
    }

    /// Configurable mock delegate for testing run_agentic_loop.
    struct MockDelegate {
        signals: Mutex<Vec<LoopSignal>>,
        llm_responses: Mutex<Vec<RespondOutput>>,
        tool_exec_count: AtomicUsize,
        tool_exec_outcome: Mutex<Option<LoopOutcome>>,
        iterations_seen: Mutex<Vec<usize>>,
        call_llm_iterations: Mutex<Vec<usize>>,
        early_exit: Mutex<Option<(usize, LoopOutcome)>>,
        nudge_count: AtomicUsize,
        /// When true, execute_tool_calls sets last_tool_batch_all_failed = true.
        simulate_all_failed: bool,
    }

    impl MockDelegate {
        fn new(responses: Vec<RespondOutput>) -> Self {
            Self {
                signals: Mutex::new(Vec::new()),
                llm_responses: Mutex::new(responses),
                tool_exec_count: AtomicUsize::new(0),
                tool_exec_outcome: Mutex::new(None),
                iterations_seen: Mutex::new(Vec::new()),
                call_llm_iterations: Mutex::new(Vec::new()),
                early_exit: Mutex::new(None),
                nudge_count: AtomicUsize::new(0),
                simulate_all_failed: false,
            }
        }

        fn with_signal(mut self, signal: LoopSignal) -> Self {
            self.signals = Mutex::new(vec![signal]);
            self
        }

        fn with_signals(mut self, signals: Vec<LoopSignal>) -> Self {
            self.signals = Mutex::new(signals);
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
            let mut signals = self.signals.lock().await;
            if signals.is_empty() {
                LoopSignal::Continue
            } else {
                signals.remove(0)
            }
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
            iteration: usize,
        ) -> Result<ironclaw_llm::RespondOutput, crate::error::Error> {
            self.call_llm_iterations.lock().await.push(iteration);
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
            tool_calls: Vec<ToolCall>,
            content: Option<String>,
            reason_ctx: &mut ReasoningContext,
            reasoning: Option<String>,
            reasoning_details: Option<ReasoningDetails>,
        ) -> Result<Option<LoopOutcome>, crate::error::Error> {
            self.tool_exec_count.fetch_add(1, Ordering::SeqCst);
            reason_ctx.messages.push(
                ChatMessage::assistant_with_tool_calls(content, tool_calls)
                    .with_reasoning_details(reasoning_details)
                    .with_reasoning(reasoning),
            );
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
    async fn first_call_llm_iteration_is_one() {
        // Regression: ChatDelegate::call_llm gates per-request overrides
        // (Responses API `temperature`, settings `temperature`, `selected_model`
        // from `/model`) on the first iteration. The gate's expected value must
        // match what this loop produces on its first call_llm invocation. An
        // off-by-one here silently disables every override and was shipped once
        // already (gate was `iteration == 0` while the loop starts at 1).
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        };
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![tool_call]),
            text_output("done"),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig::default();

        run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        let iterations = delegate.call_llm_iterations.lock().await;
        assert_eq!(
            iterations.first().copied(),
            Some(1),
            "first call_llm iteration must be 1 — see dispatcher.rs override gate"
        );
    }

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
            arguments_parse_error: None,
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
            ctx.messages.iter().any(
                |m| m.role == ironclaw_llm::Role::User && m.content.contains("injected prompt")
            ),
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
            ) -> Result<ironclaw_llm::RespondOutput, crate::error::Error> {
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
                _: Option<ReasoningDetails>,
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
            ) -> Result<ironclaw_llm::RespondOutput, crate::error::Error> {
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
                _: Option<ReasoningDetails>,
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
            .filter(|m| m.role == ironclaw_llm::Role::Assistant)
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
        ctx.available_tools.push(ironclaw_llm::ToolDefinition {
            name: "search".to_string(),
            description: "Search files".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        });
        let config = AgenticLoopConfig {
            max_iterations: 10,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
            ..Default::default()
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
                m.role == ironclaw_llm::Role::User
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

    #[test]
    fn trailing_identical_tool_calls_returns_zero_without_tool_calls() {
        let messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("plain response"),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 0);
    }

    #[test]
    fn trailing_identical_tool_calls_counts_only_trailing_distinct_call() {
        let messages = vec![
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
            assistant_tool_call_message("read_file", serde_json::json!({"path": "b.md"})),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 1);
    }

    #[test]
    fn trailing_identical_tool_calls_counts_identical_run() {
        let messages = vec![
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
            assistant_tool_call_message("read_file", serde_json::json!({"path": "a.md"})),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 5);
    }

    #[test]
    fn trailing_tool_call_run_returns_count_and_batch_signature() {
        let args = serde_json::json!({"path": "a.md"});
        let messages = vec![
            ChatMessage::user("please inspect this"),
            assistant_tool_call_message("read_file", args.clone()),
            ChatMessage::user("tool result"),
            assistant_tool_call_message("read_file", args),
        ];

        let (count, signature) =
            trailing_tool_call_run(&messages).expect("expected trailing tool-call run");

        assert_eq!(count, 2);
        assert_eq!(
            signature,
            ToolCallBatchSignature {
                calls: vec![ToolCallPrefixSignature {
                    name: "read_file".to_string(),
                    arguments_prefix: serde_json::json!({"path": "a.md"}).to_string(),
                }],
            }
        );
    }

    #[test]
    fn trailing_identical_tool_calls_counts_repeated_multi_tool_batches() {
        let batch = || {
            assistant_tool_call_batch_message(vec![
                test_tool_call("read_file", serde_json::json!({"path": "a.md"})),
                test_tool_call("parse_file", serde_json::json!({"path": "a.md"})),
            ])
        };
        let messages = vec![batch(), ChatMessage::user("tool result"), batch(), batch()];

        assert_eq!(trailing_identical_tool_calls(&messages), 3);
    }

    #[test]
    fn trailing_identical_tool_calls_respects_multi_tool_order() {
        let messages = vec![
            assistant_tool_call_batch_message(vec![
                test_tool_call("read_file", serde_json::json!({"path": "a.md"})),
                test_tool_call("parse_file", serde_json::json!({"path": "a.md"})),
            ]),
            assistant_tool_call_batch_message(vec![
                test_tool_call("parse_file", serde_json::json!({"path": "a.md"})),
                test_tool_call("read_file", serde_json::json!({"path": "a.md"})),
            ]),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 1);
    }

    #[test]
    fn trailing_identical_tool_calls_ignores_argument_drift_after_prefix() {
        let stable_prefix = "x".repeat(240);
        let messages = vec![
            assistant_tool_call_message(
                "shell",
                serde_json::json!({"cmd": format!("{stable_prefix} - 338")}),
            ),
            assistant_tool_call_message(
                "shell",
                serde_json::json!({"cmd": format!("{stable_prefix} - 340")}),
            ),
            assistant_tool_call_message(
                "shell",
                serde_json::json!({"cmd": format!("{stable_prefix} - 342")}),
            ),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 3);
    }

    #[test]
    fn trailing_identical_tool_calls_resets_on_different_tool_name() {
        let args = serde_json::json!({"path": "a.md"});
        let messages = vec![
            assistant_tool_call_message("read_file", args.clone()),
            assistant_tool_call_message("read_file", args.clone()),
            assistant_tool_call_message("parse_file", args),
        ];

        assert_eq!(trailing_identical_tool_calls(&messages), 1);
    }

    #[tokio::test]
    async fn test_truncated_tool_calls_discarded_on_length() {
        let truncated_tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "memory_write".to_string(),
            arguments: serde_json::json!({}), // empty — truncated
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        };
        let truncated_output = RespondOutput {
            result: RespondResult::ToolCalls {
                tool_calls: vec![truncated_tool_call],
                content: Some("I'll write the report.".to_string()),
                reasoning: None,
                reasoning_details: None,
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
                .any(|m| m.role == ironclaw_llm::Role::User && m.content.contains("truncated")),
            "Should inject truncation notice into context"
        );
        // The partial assistant content should have been preserved
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.role == ironclaw_llm::Role::Assistant
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
                    arguments_parse_error: None,
                }],
                content: None,
                reasoning: None,
                reasoning_details: None,
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
        }];
        let calls_b = vec![ToolCall {
            id: "c1".into(),
            name: "http_get".into(),
            arguments: serde_json::json!({"url": "https://b.com"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
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
            arguments_parse_error: None,
        }];
        let calls_b = vec![ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"b": 2, "a": 1}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        }];
        assert_eq!(
            DuplicateToolCallTracker::fingerprint(&calls_a),
            DuplicateToolCallTracker::fingerprint(&calls_b),
        );
    }

    #[tokio::test]
    async fn test_repetition_breaker_injects_once_until_streak_breaks() {
        let call_a = || test_tool_call("read_file", serde_json::json!({"path": "/tmp/stuck.txt"}));
        let call_b = || test_tool_call("parse_file", serde_json::json!({"path": "/tmp/stuck.txt"}));
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![call_a()]),
            tool_calls_output(vec![call_a()]),
            tool_calls_output(vec![call_a()]),
            tool_calls_output(vec![call_a()]),
            tool_calls_output(vec![call_b()]),
            tool_calls_output(vec![call_b()]),
            tool_calls_output(vec![call_b()]),
            text_output("Done."),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 8,
            repetition_breaker_enabled: true,
            repetition_breaker_threshold: 3,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        let corrective_count = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == ironclaw_llm::Role::User
                    && m.content.contains("stuck repeating the same tool call")
            })
            .count();
        assert_eq!(
            corrective_count, 2,
            "Should inject once for the first streak and again only after a different streak forms"
        );
    }

    #[tokio::test]
    async fn test_repetition_breaker_disabled_is_noop() {
        let same_call =
            || test_tool_call("read_file", serde_json::json!({"path": "/tmp/stuck.txt"}));
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            text_output("Done."),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 4,
            repetition_breaker_enabled: false,
            repetition_breaker_threshold: 3,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert!(
            ctx.messages
                .iter()
                .all(|m| !m.content.contains("stuck repeating the same tool call")),
            "Disabled repetition breaker should not inject a corrective"
        );
    }

    #[tokio::test]
    async fn test_repetition_breaker_threshold_zero_is_noop_when_enabled() {
        let same_call =
            || test_tool_call("read_file", serde_json::json!({"path": "/tmp/stuck.txt"}));
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            text_output("Done."),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 4,
            repetition_breaker_enabled: true,
            repetition_breaker_threshold: 0,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        assert!(
            ctx.messages
                .iter()
                .all(|m| !m.content.contains("stuck repeating the same tool call")),
            "Zero threshold should behave as disabled"
        );
    }

    #[tokio::test]
    async fn test_repetition_breaker_rearms_after_injected_user_message() {
        let same_call =
            || test_tool_call("read_file", serde_json::json!({"path": "/tmp/stuck.txt"}));
        let delegate = MockDelegate::new(vec![
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            tool_calls_output(vec![same_call()]),
            text_output("Done."),
        ])
        .with_signals(vec![
            LoopSignal::Continue,
            LoopSignal::Continue,
            LoopSignal::Continue,
            LoopSignal::Continue,
            LoopSignal::InjectMessage("please use my latest instruction".to_string()),
        ]);
        let reasoning = stub_reasoning();
        let mut ctx = ReasoningContext::new();
        let config = AgenticLoopConfig {
            max_iterations: 6,
            repetition_breaker_enabled: true,
            repetition_breaker_threshold: 3,
            ..Default::default()
        };

        let outcome = run_agentic_loop(&delegate, &reasoning, &mut ctx, &config)
            .await
            .unwrap();

        assert!(matches!(outcome, LoopOutcome::Response(_)));
        let corrective_count = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == ironclaw_llm::Role::User
                    && m.content.contains("stuck repeating the same tool call")
            })
            .count();
        assert_eq!(
            corrective_count, 2,
            "Breaker should fire again after a fresh user-message interruption"
        );

        let injected_index = ctx
            .messages
            .iter()
            .position(|m| m.content.contains("please use my latest instruction"))
            .expect("injected user message should be present");
        let following = ctx
            .messages
            .get(injected_index + 1)
            .expect("a later message should follow the injected user message");
        assert!(
            !following
                .content
                .contains("stuck repeating the same tool call"),
            "Fresh user message should suppress immediate corrective injection"
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
            arguments_parse_error: None,
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
                m.role == ironclaw_llm::Role::User && m.content.contains("same failing tool call")
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
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
            ) -> Result<ironclaw_llm::RespondOutput, crate::error::Error> {
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
                _reasoning_details: Option<ReasoningDetails>,
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
                m.role == ironclaw_llm::Role::User && m.content.contains("same failing tool call")
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
