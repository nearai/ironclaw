//! TraceLlm -- a replay-based LLM provider for E2E testing.
//!
//! Replays canned responses from a JSON trace, advancing through steps
//! sequentially. Supports both text and tool-call responses with optional
//! request-hint validation.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ironclaw::error::LlmError;
use ironclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

// Re-export shared types from recording module so existing test code can
// still import them from here.
// Re-export all shared types so downstream test files can import from here.
#[allow(unused_imports)]
pub use ironclaw::llm::recording::{
    ExpectedToolResult, HttpExchange, HttpExchangeRequest, HttpExchangeResponse,
    MemorySnapshotEntry, RequestHint, TraceResponse, TraceStep, TraceToolCall,
};

// ---------------------------------------------------------------------------
// Trace types (test-only wrappers around shared recording types)
// ---------------------------------------------------------------------------

/// A single turn in a trace: one user message and the LLM response steps that follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTurn {
    pub user_input: String,
    pub steps: Vec<TraceStep>,
    /// Declarative expectations for this turn (optional).
    #[serde(default, skip_serializing_if = "TraceExpects::is_empty")]
    pub expects: TraceExpects,
}

/// A complete LLM trace: a model name and an ordered list of turns.
///
/// Each turn pairs a user message with the LLM response steps that follow it.
/// For JSON backward compatibility, traces with a flat top-level `"steps"` array
/// (no `"turns"`) are deserialized into turns by splitting at `UserInput` boundaries.
///
/// Recorded traces (from `RecordingLlm`) may also include `memory_snapshot`,
/// `http_exchanges`, and `user_input` response steps.
#[derive(Debug, Clone, Serialize)]
pub struct LlmTrace {
    pub model_name: String,
    pub turns: Vec<TraceTurn>,
    /// Workspace memory documents captured before the recording session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_snapshot: Vec<MemorySnapshotEntry>,
    /// HTTP exchanges recorded during the session, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_exchanges: Vec<HttpExchange>,
    /// Declarative expectations for the whole trace (optional).
    #[serde(default, skip_serializing_if = "TraceExpects::is_empty")]
    pub expects: TraceExpects,
    /// Raw steps before turn conversion (populated only for recorded traces).
    /// Used by `playable_steps()` for recorded-format inspection.
    #[serde(skip)]
    #[allow(dead_code)]
    pub steps: Vec<TraceStep>,
}

/// Declarative expectations for a trace or turn.
///
/// All fields are optional and default to empty/None, so traces without
/// `expects` work unchanged (backward compatible).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceExpects {
    /// Each string must appear in the response (case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_contains: Vec<String>,
    /// None of these may appear in the response (case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_not_contains: Vec<String>,
    /// Regex that must match the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_matches: Option<String>,
    /// Each tool name must appear in started calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_used: Vec<String>,
    /// None of these tool names may appear.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_not_used: Vec<String>,
    /// If true, all tools must succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_tools_succeeded: Option<bool>,
    /// Upper bound on tool call count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<usize>,
    /// Minimum response count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_responses: Option<usize>,
    /// Tool result preview must contain substring (tool_name -> substring).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tool_results_contain: std::collections::HashMap<String, String>,
    /// Tools must have been called in this relative order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_order: Vec<String>,
}

impl TraceExpects {
    /// Returns true if no expectations are set.
    pub fn is_empty(&self) -> bool {
        self.response_contains.is_empty()
            && self.response_not_contains.is_empty()
            && self.response_matches.is_none()
            && self.tools_used.is_empty()
            && self.tools_not_used.is_empty()
            && self.all_tools_succeeded.is_none()
            && self.max_tool_calls.is_none()
            && self.min_responses.is_none()
            && self.tool_results_contain.is_empty()
            && self.tools_order.is_empty()
    }
}

/// Raw deserialization helper -- accepts either `turns` or flat `steps`.
#[derive(Deserialize)]
struct RawLlmTrace {
    model_name: String,
    #[serde(default)]
    steps: Vec<TraceStep>,
    #[serde(default)]
    turns: Vec<TraceTurn>,
    #[serde(default)]
    memory_snapshot: Vec<MemorySnapshotEntry>,
    #[serde(default)]
    http_exchanges: Vec<HttpExchange>,
    #[serde(default)]
    expects: TraceExpects,
}

impl<'de> Deserialize<'de> for LlmTrace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLlmTrace::deserialize(deserializer)?;
        // Keep the raw steps for `playable_steps()` inspection.
        let raw_steps = raw.steps.clone();
        let turns = if !raw.turns.is_empty() {
            raw.turns
        } else if !raw.steps.is_empty() {
            // Split flat steps at UserInput boundaries into turns.
            let mut turns = Vec::new();
            let mut current_input = "(test input)".to_string();
            let mut current_steps: Vec<TraceStep> = Vec::new();

            for step in raw.steps {
                if let TraceResponse::UserInput { ref content } = step.response {
                    // Flush accumulated steps as a turn (if any).
                    if !current_steps.is_empty() {
                        turns.push(TraceTurn {
                            user_input: current_input.clone(),
                            steps: std::mem::take(&mut current_steps),
                            expects: TraceExpects::default(),
                        });
                    }
                    current_input = content.clone();
                } else {
                    current_steps.push(step);
                }
            }

            // Flush remaining steps.
            if !current_steps.is_empty() {
                turns.push(TraceTurn {
                    user_input: current_input,
                    steps: current_steps,
                    expects: TraceExpects::default(),
                });
            }

            turns
        } else {
            vec![]
        };
        Ok(LlmTrace {
            model_name: raw.model_name,
            turns,
            memory_snapshot: raw.memory_snapshot,
            http_exchanges: raw.http_exchanges,
            expects: raw.expects,
            steps: raw_steps,
        })
    }
}

#[allow(dead_code)]
impl LlmTrace {
    /// Create a trace from turns.
    pub fn new(model_name: impl Into<String>, turns: Vec<TraceTurn>) -> Self {
        Self {
            model_name: model_name.into(),
            turns,
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects::default(),
            steps: Vec::new(),
        }
    }

    /// Convenience: create a single-turn trace (for simple tests).
    pub fn single_turn(
        model_name: impl Into<String>,
        user_input: impl Into<String>,
        steps: Vec<TraceStep>,
    ) -> Self {
        Self {
            model_name: model_name.into(),
            turns: vec![TraceTurn {
                user_input: user_input.into(),
                steps,
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects::default(),
            steps: Vec::new(),
        }
    }

    /// Load a trace from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let trace: Self = serde_json::from_str(&contents)?;
        Ok(trace)
    }

    /// Return only the playable steps from the raw steps (text + tool_calls),
    /// skipping `user_input` markers. Only meaningful for recorded traces that
    /// were deserialized from a flat `steps` array.
    #[allow(dead_code)]
    pub fn playable_steps(&self) -> Vec<&TraceStep> {
        self.steps
            .iter()
            .filter(|s| !matches!(s.response, TraceResponse::UserInput { .. }))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// TraceLlm provider
// ---------------------------------------------------------------------------

/// An `LlmProvider` that replays canned responses from a trace.
///
/// Steps from all turns are flattened into a single sequence at construction
/// time. By default the provider advances linearly through them, but when
/// concurrent threads share the same `TraceLlm` (e.g. an engine v2 mission
/// thread spawned alongside the foreground turn) it falls back to *hint-based
/// matching*: it scans forward for the first remaining step whose
/// `request_hint.last_user_message_contains` substring is present in the
/// current request's last user message. This lets the foreground thread and
/// the mission thread each pick up their own steps regardless of which
/// tokio task is scheduled first.
///
/// **Matching policy** (in order):
/// 1. **Head match (fast path):** if the next step in the queue either has
///    no hint or its hint substring is present in the current last user
///    message, return it. Preserves backward compatibility with sequential
///    traces (and with traces whose steps have no hint at all).
/// 2. **Hint scan:** otherwise scan forward for the first remaining step
///    whose hint matches. Removes that step from the middle of the queue
///    so it isn't returned twice. Used when concurrent sub-threads
///    interleave their LLM calls in a different order than recording time.
/// 3. **Legacy fallback:** if no step matches the current request, return
///    the head of the queue and increment `hint_mismatches`. Preserves the
///    "warn but continue" contract that pre-existing tests rely on.
pub struct TraceLlm {
    model_name: String,
    steps: Mutex<std::collections::VecDeque<TraceStep>>,
    /// Total non-error calls served, regardless of which step they returned.
    calls_served: AtomicUsize,
    hint_mismatches: AtomicUsize,
    captured_requests: Mutex<Vec<Vec<ChatMessage>>>,
}

/// Return the `last_user_message_contains` substring of a step, if any.
fn step_hint(step: &TraceStep) -> Option<&str> {
    step.request_hint
        .as_ref()
        .and_then(|h| h.last_user_message_contains.as_deref())
}

/// Best-effort coercion of a Python `repr(dict)` string into valid JSON.
/// Mirrors `coerce_python_repr_to_json` in `src/llm/recording.rs` so the
/// recorder and replay engine treat the same shapes consistently. The
/// engine v2 Python orchestrator stringifies tool results with `str(output)`,
/// which uses single quotes and Python keyword literals (`True`/`False`/`None`).
fn coerce_python_repr_to_json(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut state: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match state {
            0 => match c {
                b'\'' => {
                    out.push('"');
                    state = 1;
                }
                b'"' => {
                    out.push('"');
                    state = 2;
                }
                b'T' if bytes.get(i..i + 4) == Some(b"True") => {
                    out.push_str("true");
                    i += 4;
                    continue;
                }
                b'F' if bytes.get(i..i + 5) == Some(b"False") => {
                    out.push_str("false");
                    i += 5;
                    continue;
                }
                b'N' if bytes.get(i..i + 4) == Some(b"None") => {
                    out.push_str("null");
                    i += 4;
                    continue;
                }
                _ => out.push(c as char),
            },
            1 => match c {
                b'\\' if i + 1 < bytes.len() => {
                    out.push('\\');
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                b'"' => {
                    out.push('\\');
                    out.push('"');
                }
                b'\'' => {
                    out.push('"');
                    state = 0;
                }
                _ => out.push(c as char),
            },
            2 => match c {
                b'\\' if i + 1 < bytes.len() => {
                    out.push('\\');
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                b'"' => {
                    out.push('"');
                    state = 0;
                }
                _ => out.push(c as char),
            },
            _ => unreachable!(),
        }
        i += 1;
    }
    if state != 0 {
        return None;
    }
    Some(out)
}

/// Parse the sanitize-rewrite format `[Tool \`name\` returned: <payload>]`.
///
/// Returns the tool name and the payload string. Mirrors the recorder's
/// helper of the same name in `src/llm/recording.rs` so that templates
/// produced by the recorder can be resolved at replay time.
fn parse_user_tool_result(content: &str) -> Option<(String, &str)> {
    let rest = content.strip_prefix("[Tool ")?;
    let (name_start, after_name_quote) = if let Some(stripped) = rest.strip_prefix('`') {
        (stripped, true)
    } else {
        (rest, false)
    };
    let (name, after_name) = if after_name_quote {
        let close = name_start.find('`')?;
        (&name_start[..close], &name_start[close + 1..])
    } else {
        let close = name_start.find(' ')?;
        (&name_start[..close], &name_start[close..])
    };
    let after_returned = after_name.trim_start().strip_prefix("returned:")?;
    let payload = after_returned.trim_start();
    let payload = payload.strip_suffix(']').unwrap_or(payload);
    Some((name.to_string(), payload.trim()))
}

/// Decide whether `step` is an acceptable match for the current request.
///
/// A step matches if it has no hint (legacy behaviour) OR its hint substring
/// is present in the current request's last user message. Used by the
/// hint-based matching policy in [`TraceLlm::next_step`].
fn step_matches(step: &TraceStep, last_user_content: Option<&str>) -> bool {
    match step_hint(step) {
        None => true,
        Some(hint) => last_user_content
            .map(|content| {
                content
                    .to_ascii_lowercase()
                    .contains(&hint.to_ascii_lowercase())
            })
            .unwrap_or(false),
    }
}

#[allow(dead_code)]
impl TraceLlm {
    /// Create from an in-memory trace.
    pub fn from_trace(trace: LlmTrace) -> Self {
        let steps: std::collections::VecDeque<TraceStep> =
            trace.turns.into_iter().flat_map(|t| t.steps).collect();
        Self {
            model_name: trace.model_name,
            steps: Mutex::new(steps),
            calls_served: AtomicUsize::new(0),
            hint_mismatches: AtomicUsize::new(0),
            captured_requests: Mutex::new(Vec::new()),
        }
    }

    /// Load from a JSON file and create the provider.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let trace = LlmTrace::from_file(path)?;
        Ok(Self::from_trace(trace))
    }

    /// Number of calls made so far.
    pub fn calls(&self) -> usize {
        self.calls_served.load(Ordering::Relaxed)
    }

    /// Number of request-hint mismatches observed (warnings only).
    pub fn hint_mismatches(&self) -> usize {
        self.hint_mismatches.load(Ordering::Relaxed)
    }

    /// Clone of all captured request message lists.
    pub fn captured_requests(&self) -> Vec<Vec<ChatMessage>> {
        self.captured_requests.lock().unwrap().clone()
    }

    // -- internal helpers ---------------------------------------------------

    /// Pick the next step that satisfies the current request.
    ///
    /// See the [`TraceLlm`] doc comment for the matching policy. Removes the
    /// chosen step from the queue and applies template substitution on tool
    /// call arguments before returning.
    fn next_step(&self, messages: &[ChatMessage]) -> Result<TraceStep, LlmError> {
        // Capture the request messages for inspection-based assertions.
        self.captured_requests
            .lock()
            .unwrap()
            .push(messages.to_vec());

        let last_user_content: Option<String> = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, Role::User))
            .map(|m| m.content.clone());

        let mut step = {
            let mut steps = self.steps.lock().unwrap();
            if steps.is_empty() {
                return Err(LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: format!(
                        "TraceLlm exhausted: served {} call(s), no steps left",
                        self.calls_served.load(Ordering::Relaxed)
                    ),
                });
            }

            // (1) Head fast path: head matches (or has no hint at all).
            let head_matches = step_matches(&steps[0], last_user_content.as_deref());
            if head_matches {
                steps.pop_front().expect("checked non-empty above")
            } else {
                // (2) Hint scan: look for the first step whose hint substring
                //     is present in the current last user message.
                let scan_pos = (1..steps.len())
                    .find(|&i| step_matches(&steps[i], last_user_content.as_deref()));
                if let Some(idx) = scan_pos {
                    steps.remove(idx).expect("scan position is valid")
                } else {
                    // (3) Legacy fallback: nothing matches. Return the head
                    //     and warn — preserves the "warn but continue" contract
                    //     that pre-existing tests rely on.
                    self.hint_mismatches.fetch_add(1, Ordering::Relaxed);
                    if let Some(hint) = step_hint(&steps[0]) {
                        eprintln!(
                            "[TraceLlm WARN] Request hint mismatch: expected last user message to contain {:?}, \
                             got {:?}",
                            hint,
                            last_user_content.as_deref(),
                        );
                    }
                    steps.pop_front().expect("checked non-empty above")
                }
            }
        };

        // Soft-validate min_message_count on the chosen step.
        if let Some(ref hint) = step.request_hint
            && let Some(min_count) = hint.min_message_count
            && messages.len() < min_count
        {
            self.hint_mismatches.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[TraceLlm WARN] Request hint mismatch: expected >= {} messages, got {}",
                min_count,
                messages.len(),
            );
        }

        // Apply template substitution on tool_call arguments.
        if let TraceResponse::ToolCalls {
            ref mut tool_calls, ..
        } = step.response
        {
            let vars = Self::extract_tool_result_vars(messages);
            if !vars.is_empty() {
                for tc in tool_calls.iter_mut() {
                    Self::substitute_templates(&mut tc.arguments, &vars);
                }
            }
        }

        self.calls_served.fetch_add(1, Ordering::Relaxed);
        Ok(step)
    }

    /// Build a map of `"key.field" -> resolved_value` from tool result
    /// messages in the conversation. Used to resolve `{{key.field}}`
    /// templates in recorded tool call arguments at replay time.
    ///
    /// This must be symmetric with the recorder's `build_prior_tool_lookup`
    /// — both shapes of prior tool result need to be indexed:
    ///
    /// 1. **Native `Role::Tool`** — keyed by `tool_call_id`. Used by
    ///    providers that pass tool results through unmodified.
    /// 2. **Sanitized `Role::User`** — `sanitize_tool_messages` rewrites
    ///    orphaned tool results as `[Tool \`name\` returned: <json>]` user
    ///    messages. Keyed as `tool:<name>` (the recorder uses the same key
    ///    so the templates resolve here).
    ///
    /// Tool result content may be wrapped in `<tool_output>` XML tags by
    /// the safety layer, so we strip those before parsing.
    fn extract_tool_result_vars(
        messages: &[ChatMessage],
    ) -> std::collections::HashMap<String, String> {
        let mut vars = std::collections::HashMap::new();
        for msg in messages {
            // Shape 1: native Role::Tool with structured content.
            if msg.role == Role::Tool {
                let Some(call_id) = msg.tool_call_id.as_deref() else {
                    continue;
                };
                let content = Self::unwrap_tool_output(&msg.content);
                Self::index_json_into_vars(&mut vars, call_id, content.as_ref());
                continue;
            }
            // Shape 2: Role::User rewrite of orphaned tool results.
            if msg.role == Role::User
                && let Some((tool_name, payload)) = parse_user_tool_result(&msg.content)
            {
                let key = format!("tool:{tool_name}");
                Self::index_json_into_vars(&mut vars, &key, payload);
            }
        }
        vars
    }

    /// Parse `content` as a JSON object and merge top-level scalar fields
    /// into `vars` as `"<key>.<field>" -> stringified value`. Falls back to
    /// Python-repr coercion if JSON parsing fails — the engine v2 orchestrator
    /// emits tool results as `str(dict)`, which uses single quotes and
    /// Python keyword literals.
    fn index_json_into_vars(
        vars: &mut std::collections::HashMap<String, String>,
        key: &str,
        content: &str,
    ) {
        let parsed: Option<serde_json::Value> = serde_json::from_str(content).ok().or_else(|| {
            coerce_python_repr_to_json(content).and_then(|s| serde_json::from_str(&s).ok())
        });
        let Some(json) = parsed else { return };
        let Some(obj) = json.as_object() else {
            return;
        };
        for (field, val) in obj {
            let str_val = match val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            vars.insert(format!("{key}.{field}"), str_val);
        }
    }

    /// Strip `<tool_output name="...">...\n</tool_output>` wrapper from
    /// safety-layer output and reverse the targeted `</tool_output` escape.
    fn unwrap_tool_output(content: &str) -> std::borrow::Cow<'_, str> {
        if let Some(body) = ironclaw_safety::SafetyLayer::unwrap_tool_output(content) {
            return std::borrow::Cow::Owned(body);
        }
        std::borrow::Cow::Borrowed(content)
    }

    /// Walk a JSON value and replace any string matching `{{call_id.path}}`
    /// with the resolved value from the vars map. Operates in-place.
    fn substitute_templates(
        value: &mut serde_json::Value,
        vars: &std::collections::HashMap<String, String>,
    ) {
        match value {
            serde_json::Value::String(s) => {
                // Full-value replacement: if the entire string is `{{...}}`,
                // replace the whole value (preserving type if possible).
                if s.starts_with("{{") && s.ends_with("}}") && s.matches("{{").count() == 1 {
                    let key = s[2..s.len() - 2].trim();
                    if let Some(resolved) = vars.get(key) {
                        *s = resolved.clone();
                        return;
                    }
                }
                // Inline replacement: replace all `{{...}}` occurrences within the string.
                let mut result = s.clone();
                while let Some(start) = result.find("{{") {
                    if let Some(end) = result[start..].find("}}") {
                        let end = start + end + 2;
                        let key = result[start + 2..end - 2].trim();
                        if let Some(resolved) = vars.get(key) {
                            result = format!("{}{}{}", &result[..start], resolved, &result[end..]);
                        } else {
                            // Unresolved template — leave as-is and stop to avoid infinite loop.
                            break;
                        }
                    } else {
                        break;
                    }
                }
                *s = result;
            }
            serde_json::Value::Object(map) => {
                for val in map.values_mut() {
                    Self::substitute_templates(val, vars);
                }
            }
            serde_json::Value::Array(arr) => {
                for val in arr.iter_mut() {
                    Self::substitute_templates(val, vars);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl LlmProvider for TraceLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // complete() is called when Reasoning has force_text=true (no tools
        // available). Skip any remaining ToolCalls steps in the trace and
        // return the next Text step, since in real usage the LLM would
        // produce text when no tools are offered.
        loop {
            let step = self.next_step(&request.messages)?;
            match step.response {
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    return Ok(CompletionResponse {
                        content,
                        input_tokens,
                        output_tokens,
                        finish_reason: FinishReason::Stop,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                    });
                }
                TraceResponse::ToolCalls { .. } => {
                    // Skip tool_calls steps — complete() is called in
                    // force_text mode so the LLM can't use tools anyway.
                    continue;
                }
                TraceResponse::UserInput { .. } => {
                    return Err(LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: "TraceLlm::complete() encountered a user_input step; \
                                 these should have been filtered out during construction"
                            .to_string(),
                    });
                }
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let step = self.next_step(&request.messages)?;
        match step.response {
            TraceResponse::Text {
                content,
                input_tokens,
                output_tokens,
            } => Ok(ToolCompletionResponse {
                content: Some(content),
                tool_calls: Vec::new(),
                input_tokens,
                output_tokens,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
            TraceResponse::ToolCalls {
                tool_calls,
                input_tokens,
                output_tokens,
            } => {
                let calls: Vec<ToolCall> = tool_calls
                    .into_iter()
                    .map(|tc| ToolCall {
                        id: tc.id,
                        name: tc.name,
                        arguments: tc.arguments,
                        reasoning: None,
                    })
                    .collect();
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: calls,
                    input_tokens,
                    output_tokens,
                    finish_reason: FinishReason::ToolUse,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            TraceResponse::UserInput { .. } => Err(LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "TraceLlm::complete_with_tools() encountered a user_input step; \
                         these should have been filtered out during construction"
                    .to_string(),
            }),
        }
    }
}
