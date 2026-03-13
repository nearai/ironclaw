//! Unified agentic loop engine.
//!
//! Provides a single implementation of the core LLM call → tool execution →
//! result processing → context update → repeat cycle. Three consumers
//! (chat dispatcher, job worker, container runtime) customize behavior
//! via the `LoopDelegate` trait.

use async_trait::async_trait;
use chrono::{Datelike, Utc, Weekday};

use crate::agent::session::PendingApproval;
use crate::error::Error;
use crate::llm::{ChatMessage, Reasoning, ReasoningContext, RespondResult, Role};

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
    /// A tool requires user approval before continuing (chat delegate only).
    NeedApproval(Box<PendingApproval>),
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
        reason_ctx: &mut ReasoningContext,
    ) -> TextAction;

    /// Execute tool calls and add results to context.
    /// Return `Some(outcome)` to break the loop (e.g. approval needed).
    async fn execute_tool_calls(
        &self,
        tool_calls: Vec<crate::llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<Option<LoopOutcome>, Error>;

    /// Called when the LLM expresses tool intent without actually calling a tool.
    /// Delegates can use this to emit events or log the nudge for observability.
    async fn on_tool_intent_nudge(&self, _text: &str, _reason_ctx: &mut ReasoningContext) {}

    /// Called after each successful iteration (no error, no early return).
    async fn after_iteration(&self, _iteration: usize) {}
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

        match output.result {
            RespondResult::Text(text) => {
                // Guard: for spam-folder checks, require a fresh gws_bridge tool
                // result before accepting "empty/no spam" claims.
                if config.enable_tool_intent_nudge
                    && consecutive_tool_intent_nudges < config.max_tool_intent_nudges
                    && needs_fresh_spam_tool_call(&text, reason_ctx)
                {
                    consecutive_tool_intent_nudges += 1;
                    tracing::info!(
                        iteration,
                        "Spam summary response lacked fresh gws_bridge evidence, nudging for tool call"
                    );
                    reason_ctx.messages.push(ChatMessage::assistant(&text));
                    reason_ctx.messages.push(ChatMessage::user(
                        "Do not answer from memory for spam-folder checks. \
                         Call gws_bridge now to list Gmail spam messages (fresh data), \
                         then answer from tool output only.",
                    ));
                    delegate.after_iteration(iteration).await;
                    continue;
                }
                if config.enable_tool_intent_nudge
                    && consecutive_tool_intent_nudges < config.max_tool_intent_nudges
                    && needs_fresh_calendar_tool_call(&text, reason_ctx)
                {
                    consecutive_tool_intent_nudges += 1;
                    tracing::info!(
                        iteration,
                        "Calendar summary response lacked fresh calendar tool evidence, nudging for calendar tool call"
                    );
                    reason_ctx.messages.push(ChatMessage::assistant(&text));
                    reason_ctx.messages.push(ChatMessage::user(
                        "Do not answer from memory for calendar checks. \
                         Call gws_bridge now with calendar events list args for current week \
                         (singleEvents=true, orderBy=startTime, timeMin=now, timeMax=now+7d), \
                         then answer from tool output only.",
                    ));
                    delegate.after_iteration(iteration).await;
                    continue;
                }
                if config.enable_tool_intent_nudge
                    && consecutive_tool_intent_nudges < config.max_tool_intent_nudges
                    && needs_calendar_relative_day_rewrite(&text, reason_ctx)
                {
                    consecutive_tool_intent_nudges += 1;
                    let today = Utc::now().date_naive();
                    let tomorrow = today.succ_opt().unwrap_or(today);
                    tracing::info!(
                        iteration,
                        "Calendar response used inconsistent today/tomorrow weekday labels, nudging for rewrite"
                    );
                    reason_ctx.messages.push(ChatMessage::assistant(&text));
                    reason_ctx.messages.push(ChatMessage::user(&format!(
                        "Rewrite the calendar summary with correct day mapping. \
                         Today is {} and tomorrow is {}. \
                         Use absolute dates and weekdays in the final answer.",
                        today.format("%A %d %B %Y"),
                        tomorrow.format("%A %d %B %Y")
                    )));
                    delegate.after_iteration(iteration).await;
                    continue;
                }

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

                match delegate.handle_text_response(&text, reason_ctx).await {
                    TextAction::Return(outcome) => return Ok(outcome),
                    TextAction::Continue => {}
                }
            }
            RespondResult::ToolCalls {
                tool_calls,
                content,
            } => {
                consecutive_tool_intent_nudges = 0;

                if let Some(outcome) = delegate
                    .execute_tool_calls(tool_calls, content, reason_ctx)
                    .await?
                {
                    return Ok(outcome);
                }
            }
        }

        delegate.after_iteration(iteration).await;
    }

    Ok(LoopOutcome::MaxIterations)
}

fn needs_fresh_spam_tool_call(text: &str, reason_ctx: &ReasoningContext) -> bool {
    let latest_user = latest_user_message(reason_ctx);
    let Some(user_msg) = latest_user else {
        return false;
    };
    let user_lower = user_msg.to_lowercase();
    let spam_request = (user_lower.contains("spam") && user_lower.contains("email"))
        || user_lower.contains("spam folder")
        || user_lower.contains("in:spam");
    if !spam_request {
        return false;
    }

    let text_lower = text.to_lowercase();
    let claims_empty = (text_lower.contains("spam folder") || text_lower.contains("spam"))
        && (text_lower.contains("empty")
            || text_lower.contains("no spam")
            || text_lower.contains("no messages"));
    if !claims_empty {
        return false;
    }

    !has_tool_result_since_last_user(reason_ctx, "gws_bridge")
}

fn latest_user_message(reason_ctx: &ReasoningContext) -> Option<&str> {
    reason_ctx
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
}

fn has_tool_result_since_last_user(reason_ctx: &ReasoningContext, tool_name: &str) -> bool {
    let slice_start = reason_ctx
        .messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(0, |idx| idx + 1);
    reason_ctx.messages[slice_start..]
        .iter()
        .any(|m| m.role == Role::Tool && m.name.as_deref().is_some_and(|n| n.eq(tool_name)))
}

fn needs_fresh_calendar_tool_call(text: &str, reason_ctx: &ReasoningContext) -> bool {
    let latest_user = latest_user_message(reason_ctx);
    let Some(user_msg) = latest_user else {
        return false;
    };
    let user_lower = user_msg.to_lowercase();
    let calendar_request = user_lower.contains("calendar")
        && (user_lower.contains("week") || user_lower.contains("remaining"));
    if !calendar_request {
        return false;
    }

    let text_lower = text.to_lowercase();
    let claims_empty_calendar = text_lower.contains("calendar")
        && (text_lower.contains("no events")
            || text_lower.contains("zero events")
            || text_lower.contains("completely empty")
            || text_lower.contains("empty"));
    if !claims_empty_calendar {
        return false;
    }

    // Require a fresh calendar events list tool call after the latest user message.
    !has_calendar_events_tool_call_since_last_user(reason_ctx)
}

fn has_calendar_events_tool_call_since_last_user(reason_ctx: &ReasoningContext) -> bool {
    let slice_start = reason_ctx
        .messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(0, |idx| idx + 1);

    reason_ctx.messages[slice_start..].iter().any(|m| {
        if m.role != Role::Assistant {
            return false;
        }
        let Some(calls) = m.tool_calls.as_ref() else {
            return false;
        };
        calls.iter().any(|tc| {
            if tc.name != "gws_bridge" {
                return false;
            }
            let Some(args) = tc.arguments.get("args").and_then(|v| v.as_array()) else {
                return false;
            };
            let as_str: Vec<&str> = args.iter().filter_map(|v| v.as_str()).collect();
            as_str.len() >= 3
                && as_str[0] == "calendar"
                && as_str[1] == "events"
                && as_str[2] == "list"
        })
    })
}

fn needs_calendar_relative_day_rewrite(text: &str, reason_ctx: &ReasoningContext) -> bool {
    let latest_user = latest_user_message(reason_ctx);
    let Some(user_msg) = latest_user else {
        return false;
    };
    let user_lower = user_msg.to_lowercase();
    let calendar_request = user_lower.contains("calendar");
    if !calendar_request {
        return false;
    }
    calendar_relative_day_mismatch_for_date(text, Utc::now().date_naive())
}

fn calendar_relative_day_mismatch_for_date(text: &str, today: chrono::NaiveDate) -> bool {
    let text_lower = text.to_lowercase();
    let today_mismatch =
        extract_labeled_weekday(&text_lower, "today").is_some_and(|w| w != today.weekday());
    let tomorrow_mismatch = extract_labeled_weekday(&text_lower, "tomorrow").is_some_and(|w| {
        let tomorrow = today.succ_opt().unwrap_or(today);
        w != tomorrow.weekday()
    });
    today_mismatch || tomorrow_mismatch
}

fn extract_labeled_weekday(text_lower: &str, label: &str) -> Option<Weekday> {
    let marker = format!("{label} (");
    let pos = text_lower.find(&marker)?;
    let day_start = pos + marker.len();
    let rem = &text_lower[day_start..];
    let day_end = rem.find(')')?;
    parse_weekday_name(&rem[..day_end])
}

fn parse_weekday_name(s: &str) -> Option<Weekday> {
    match s.trim() {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Truncate a string for log/status previews.
///
/// `max` is a byte budget. The result is truncated at the last valid char
/// boundary at or before `max` bytes, so it is always valid UTF-8.
pub fn truncate_for_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{RespondOutput, TokenUsage, ToolCall};
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
        }
    }

    fn tool_calls_output(calls: Vec<ToolCall>) -> RespondOutput {
        RespondOutput {
            result: RespondResult::ToolCalls {
                tool_calls: calls,
                content: None,
            },
            usage: zero_usage(),
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
            _reason_ctx: &mut ReasoningContext,
        ) -> TextAction {
            TextAction::Return(LoopOutcome::Response(text.to_string()))
        }

        async fn execute_tool_calls(
            &self,
            _tool_calls: Vec<ToolCall>,
            _content: Option<String>,
            reason_ctx: &mut ReasoningContext,
        ) -> Result<Option<LoopOutcome>, crate::error::Error> {
            self.tool_exec_count.fetch_add(1, Ordering::SeqCst);
            reason_ctx
                .messages
                .push(ChatMessage::user("tool result stub"));
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
    fn test_truncate_long_string_adds_ellipsis() {
        let result = truncate_for_preview("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        let result = truncate_for_preview("café", 4);
        assert_eq!(result, "caf...");
    }

    #[test]
    fn test_needs_fresh_spam_tool_call_true_when_empty_claim_without_tool() {
        let mut ctx = ReasoningContext::new();
        ctx.messages
            .push(ChatMessage::user("summarise my email spam folder"));
        let text = "Your Gmail spam folder is empty.";
        assert!(needs_fresh_spam_tool_call(text, &ctx));
    }

    #[test]
    fn test_needs_fresh_spam_tool_call_false_when_tool_result_present() {
        let mut ctx = ReasoningContext::new();
        ctx.messages
            .push(ChatMessage::user("summarise my email spam folder"));
        ctx.messages.push(ChatMessage::tool_result(
            "turn0_0",
            "gws_bridge",
            "{\"resultSizeEstimate\":3}",
        ));
        let text = "Your Gmail spam folder is empty.";
        assert!(!needs_fresh_spam_tool_call(text, &ctx));
    }

    #[test]
    fn test_needs_fresh_calendar_tool_call_true_when_empty_claim_without_calendar_call() {
        let mut ctx = ReasoningContext::new();
        ctx.messages.push(ChatMessage::user(
            "summarise the remaining calendar entries for this week",
        ));
        // Wrong tool call (gmail), should not satisfy calendar evidence.
        let wrong_call = crate::llm::ToolCall {
            id: "turn0_0".to_string(),
            name: "gws_bridge".to_string(),
            arguments: serde_json::json!({"args":["gmail","users","messages","list"]}),
        };
        ctx.messages.push(ChatMessage::assistant_with_tool_calls(
            None,
            vec![wrong_call],
        ));
        let text = "Your calendar has zero events this week.";
        assert!(needs_fresh_calendar_tool_call(text, &ctx));
    }

    #[test]
    fn test_needs_fresh_calendar_tool_call_false_when_calendar_call_present() {
        let mut ctx = ReasoningContext::new();
        ctx.messages.push(ChatMessage::user(
            "summarise the remaining calendar entries for this week",
        ));
        let calendar_call = crate::llm::ToolCall {
            id: "turn0_0".to_string(),
            name: "gws_bridge".to_string(),
            arguments: serde_json::json!({"args":["calendar","events","list","--params={\"calendarId\":\"primary\"}"]}),
        };
        ctx.messages.push(ChatMessage::assistant_with_tool_calls(
            None,
            vec![calendar_call],
        ));
        let text = "Your calendar has zero events this week.";
        assert!(!needs_fresh_calendar_tool_call(text, &ctx));
    }

    #[test]
    fn test_calendar_relative_day_mismatch_detects_wrong_today_weekday() {
        let text = "You have a job interview today (Thursday).";
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 11).expect("valid date");
        assert!(calendar_relative_day_mismatch_for_date(text, today));
    }

    #[test]
    fn test_calendar_relative_day_mismatch_allows_correct_tomorrow_weekday() {
        let text = "You have a job interview tomorrow (Thursday).";
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 11).expect("valid date");
        assert!(!calendar_relative_day_mismatch_for_date(text, today));
    }

    #[test]
    fn test_has_tool_result_since_last_user_handles_empty_context() {
        let ctx = ReasoningContext::new();
        assert!(!has_tool_result_since_last_user(&ctx, "gws_bridge"));
    }

    #[test]
    fn test_has_calendar_events_tool_call_since_last_user_handles_empty_context() {
        let ctx = ReasoningContext::new();
        assert!(!has_calendar_events_tool_call_since_last_user(&ctx));
    }
}
