//! Scripted tool-call workloads for the API stress scenario.
//!
//! The API driver embeds a deterministic marker in each user message:
//!
//! ```text
//! ironclaw-stress-tool <script> <user>__<op> <size_bytes>
//! ```
//!
//! The mock LLM sidecar recognizes the marker and drives a scripted tool
//! sequence for that operation (write/append/read steps through the real
//! capability host), then finishes with a verdict text:
//!
//! ```text
//! ironclaw-stress-tool result <user>__<op> <verdict>
//! ```
//!
//! Every scripted write embeds a read-back token
//! (`IRONCLAW_STRESS_READBACK_<user>__<op>`) in its content, and every
//! scripted sequence ends with a read step, so the verdict is derived from
//! what the tool actually returned through the production path:
//!
//! - `confirmed`: the read returned exactly this operation's token.
//! - `contended`: another operation of the same user overwrote the document
//!   between write and read (hot-document contention — expected under
//!   concurrent same-user writers, counted not failed).
//! - `leak`: the read returned another user's token (cross-user isolation
//!   violation — a hard failure).
//! - `missing`: the read returned no token at all (durable write lost — a
//!   hard failure).
//! - `undisclosed`: the required tool was never advertised to the model
//!   (progressive-disclosure / agent-surface regression — a hard failure).
//!
//! The module is deliberately pure: no I/O, no globals. Everything the
//! sidecar and the driver need is a function of the conversation JSON and
//! the advertised tool names, which keeps the state machine unit-testable
//! and deterministic.

use std::collections::{BTreeMap, HashSet};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Prefix of every scripted driver marker in a user message.
pub(crate) const MARKER_PREFIX: &str = "ironclaw-stress-tool";
/// Prefix of the final verdict text emitted by the sidecar.
pub(crate) const RESULT_PREFIX: &str = "ironclaw-stress-tool result";
/// Prefix of every read-back token embedded in scripted write content.
pub(crate) const READBACK_MARKER: &str = "IRONCLAW_STRESS_READBACK";
/// Relative memory document path every memory script targets. All users
/// share the same logical relative path, so each operation doubles as a
/// same-relative-path isolation check.
pub(crate) const SHARED_MEMORY_TARGET: &str = "stress/shared.md";
/// Lower bound for a scripted document size, in bytes. Scripted content is
/// a read-back token plus deterministic padding; below 4 KiB the token
/// dominates and sizes lose meaning, and the issue's workloads start at
/// 4 KiB.
pub(crate) const MIN_SCRIPTED_DOC_SIZE_BYTES: usize = 4096;
/// Upper bound for a scripted document size, in bytes.
pub(crate) const MAX_SCRIPTED_DOC_SIZE_BYTES: usize = 8 * 1024 * 1024;
/// Number of assistant turns to wait for a scripted tool to be disclosed
/// before declaring the operation `undisclosed`.
pub(crate) const UNDISCLOSED_ATTEMPTS: usize = 2;
/// Separator between the user part and the op part of a marker identity.
/// `-` and `.` can appear in user labels, so `__` is the split token.
const IDENTITY_SEPARATOR: &str = "__";
/// Upper bound for one marker identity part (`user` or `op`), in bytes. Keeps
/// read-back tokens and tool arguments bounded regardless of the configured
/// document size.
const MAX_IDENTITY_PART_LEN: usize = 64;
/// Interim text the sidecar emits while a scripted tool is not yet
/// advertised. The driver detects it in the timeline and classifies the op
/// as `undisclosed` instead of a plain timeout.
pub(crate) const PLACEHOLDER_TEXT: &str =
    "ironclaw-stress-tool pending \u{2014} I'll perform the stress tool action next.";

/// Scripted workload keys. The wire key is what the driver puts in the
/// marker and the sidecar switches on. The string mapping lives in the clap
/// value names so the CLI flag, the marker wire format, and any parsing
/// share one source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScriptKey {
    /// `builtin.write_file` then `builtin.read_file` of the same unique
    /// workspace path.
    #[value(name = "write_file_roundtrip")]
    WriteFileRoundtrip,
    /// `ironclaw.memory.write` (replace) then `ironclaw.memory.read` of the
    /// shared relative memory target.
    #[value(name = "memory_roundtrip")]
    MemoryRoundtrip,
    /// `ironclaw.memory.write` (quarter) then append (three quarters) then
    /// read of the shared target — growing-append slope workload.
    #[value(name = "memory_grow")]
    MemoryGrow,
    /// `ironclaw.memory.write` (half), read, append (half), read of the
    /// shared target — mixed read/write workload.
    #[value(name = "memory_mixed")]
    MemoryMixed,
}

impl ScriptKey {
    /// Parse a wire-format key from a marker message. Delegates to clap's
    /// value mapping so the CLI and the marker format cannot drift apart.
    pub(crate) fn parse(key: &str) -> Option<Self> {
        Self::from_str(key, false).ok()
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WriteFileRoundtrip => "write_file_roundtrip",
            Self::MemoryRoundtrip => "memory_roundtrip",
            Self::MemoryGrow => "memory_grow",
            Self::MemoryMixed => "memory_mixed",
        }
    }

    /// Number of tool-result messages the driver must observe in the
    /// timeline before the operation's tool sequence is complete.
    pub(crate) fn expected_tool_results(self) -> usize {
        self.steps().len()
    }

    pub(crate) fn steps(self) -> &'static [ScriptStep] {
        match self {
            Self::WriteFileRoundtrip => &[
                ScriptStep {
                    capability_id: "builtin.write_file",
                    kind: StepKind::WriteFile,
                },
                ScriptStep {
                    capability_id: "builtin.read_file",
                    kind: StepKind::ReadFile,
                },
            ],
            Self::MemoryRoundtrip => &[
                ScriptStep {
                    capability_id: "ironclaw.memory.write",
                    kind: StepKind::MemoryWrite {
                        append: false,
                        fraction: 4,
                    },
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.read",
                    kind: StepKind::MemoryRead,
                },
            ],
            Self::MemoryGrow => &[
                ScriptStep {
                    capability_id: "ironclaw.memory.write",
                    kind: StepKind::MemoryWrite {
                        append: false,
                        fraction: 1,
                    },
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.write",
                    kind: StepKind::MemoryWrite {
                        append: true,
                        fraction: 3,
                    },
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.read",
                    kind: StepKind::MemoryRead,
                },
            ],
            Self::MemoryMixed => &[
                ScriptStep {
                    capability_id: "ironclaw.memory.write",
                    kind: StepKind::MemoryWrite {
                        append: false,
                        fraction: 2,
                    },
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.read",
                    kind: StepKind::MemoryRead,
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.write",
                    kind: StepKind::MemoryWrite {
                        append: true,
                        fraction: 2,
                    },
                },
                ScriptStep {
                    capability_id: "ironclaw.memory.read",
                    kind: StepKind::MemoryRead,
                },
            ],
        }
    }
}

/// One tool call of a scripted sequence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptStep {
    /// Dotted capability id the step targets (e.g. `builtin.write_file`).
    pub(crate) capability_id: &'static str,
    pub(crate) kind: StepKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StepKind {
    WriteFile,
    ReadFile,
    MemoryWrite {
        append: bool,
        /// Content share of the operation's total size: numerator of a
        /// fraction with denominator 4 (1 = quarter, 2 = half, 3 = three
        /// quarters, 4 = full).
        fraction: u8,
    },
    MemoryRead,
}

/// A parsed scripted operation marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptedOp {
    pub(crate) key: ScriptKey,
    /// User identity part of the marker (`u0`, `u1`, ...).
    pub(crate) user: String,
    /// Operation identity part of the marker (`0`, `1`, `h2-3`, ...).
    pub(crate) op: String,
    /// Document size for this operation, in bytes.
    pub(crate) size_bytes: usize,
}

impl ScriptedOp {
    pub(crate) fn identity(&self) -> String {
        format!("{}{IDENTITY_SEPARATOR}{}", self.user, self.op)
    }

    pub(crate) fn readback_token(&self) -> String {
        format!("{READBACK_MARKER}_{}", self.identity())
    }
}

/// Verdict the sidecar derives from what the read-back tool actually
/// returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Confirmed,
    Contended,
    Leak,
    Missing,
    Undisclosed,
}

impl Verdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Contended => "contended",
            Self::Leak => "leak",
            Self::Missing => "missing",
            Self::Undisclosed => "undisclosed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "confirmed" => Some(Self::Confirmed),
            "contended" => Some(Self::Contended),
            "leak" => Some(Self::Leak),
            "missing" => Some(Self::Missing),
            "undisclosed" => Some(Self::Undisclosed),
            _ => None,
        }
    }

    /// Whether this verdict is a hard failure for the driver.
    pub(crate) fn is_failure(self) -> bool {
        matches!(self, Self::Leak | Self::Missing | Self::Undisclosed)
    }
}

/// What the sidecar should answer for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptedDecision {
    /// Emit a tool call for the next script step.
    ToolCall(ToolCallSpec),
    /// Emit the final verdict text.
    FinalText(String),
    /// Emit an interim text response while waiting for tool disclosure.
    Placeholder,
    /// No scripted marker in this conversation; fall through to the default
    /// text path.
    None,
}

/// A single tool call to emit, with the exact wire name advertised in the
/// request and the arguments to pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolCallSpec {
    pub(crate) wire_name: String,
    pub(crate) arguments: Value,
}

/// Build the driver marker for one operation.
pub(crate) fn marker_message(key: ScriptKey, user: &str, op: &str, size_bytes: usize) -> String {
    format!(
        "{MARKER_PREFIX} {} {}__{} {size_bytes}",
        key.as_str(),
        user,
        op
    )
}

/// Parse a driver marker out of a message's text.
pub(crate) fn parse_marker(content: &str) -> Option<ScriptedOp> {
    let mut parts = content.split_whitespace();
    if parts.next()? != MARKER_PREFIX {
        return None;
    }
    let key = ScriptKey::parse(parts.next()?)?;
    let identity = parts.next()?;
    let size_bytes = parts.next()?.parse::<usize>().ok()?;
    if !(MIN_SCRIPTED_DOC_SIZE_BYTES..=MAX_SCRIPTED_DOC_SIZE_BYTES).contains(&size_bytes) {
        return None;
    }
    let (user, op) = identity.split_once(IDENTITY_SEPARATOR)?;
    if user.is_empty()
        || op.is_empty()
        || user.len() > MAX_IDENTITY_PART_LEN
        || op.len() > MAX_IDENTITY_PART_LEN
        || !is_readback_compatible(user)
        || !is_readback_compatible(op)
    {
        return None;
    }
    Some(ScriptedOp {
        key,
        user: user.to_string(),
        op: op.to_string(),
        size_bytes,
    })
}

/// Whether an identity part survives `readback_tokens` scanning: ASCII
/// alphanumerics plus `-`, `_`, and `.`. Any other character truncates the
/// read-back token at that character, so the scripted call could never
/// produce a matching verdict.
fn is_readback_compatible(part: &str) -> bool {
    part.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// Pick the document size for an operation index from a size list, cycling.
pub(crate) fn doc_size_for(sizes: &[usize], op_index: usize) -> usize {
    if sizes.is_empty() {
        return 4096;
    }
    sizes[op_index % sizes.len()]
}

/// Extract the text of a chat message, accepting both a plain string and the
/// OpenAI parts-array shape.
pub(crate) fn message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    text.push_str(part_text);
                }
            }
            Some(text)
        }
        _ => None,
    }
}

/// Messages of a conversation split into user messages, tool results, and
/// assistant messages, in conversation order.
#[derive(Debug, Clone, Default)]
pub(crate) struct Conversation {
    pub(crate) user_messages: Vec<(usize, String)>,
    pub(crate) tool_results: Vec<(usize, String)>,
    pub(crate) assistant_messages: Vec<(usize, String)>,
}

impl Conversation {
    pub(crate) fn from_messages(messages: &[Value]) -> Self {
        let mut conversation = Self::default();
        for (index, message) in messages.iter().enumerate() {
            let Some(role) = message.get("role").and_then(Value::as_str) else {
                continue;
            };
            let Some(text) = message_text(message) else {
                continue;
            };
            match role {
                "user" => conversation.user_messages.push((index, text)),
                "tool" => conversation.tool_results.push((index, text)),
                "assistant" => conversation.assistant_messages.push((index, text)),
                _ => {}
            }
        }
        conversation
    }
}

/// Decide what the mock LLM sidecar should answer for a chat completion
/// request, together with the scripted operation the decision applies to.
/// Pure function of the conversation and the advertised tools; the message
/// array is walked exactly once.
pub(crate) fn decide_with_op(
    messages: &[Value],
    available_tool_names: &HashSet<String>,
) -> (ScriptedDecision, Option<ScriptedOp>) {
    let conversation = Conversation::from_messages(messages);
    let Some((marker_position, op)) = find_latest_op(&conversation) else {
        return (ScriptedDecision::None, None);
    };

    if conversation_has_result(&conversation, marker_position, &op) {
        return (ScriptedDecision::None, Some(op));
    }

    let tool_results_after = conversation
        .tool_results
        .iter()
        .filter(|(position, _)| *position > marker_position)
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>();
    let step_index = tool_results_after.len();
    let steps = op.key.steps();
    if step_index < steps.len() {
        let step = &steps[step_index];
        if let Some(wire_name) = resolve_wire_name(available_tool_names, step.capability_id) {
            let arguments = build_arguments(&op, step, step_index);
            return (
                ScriptedDecision::ToolCall(ToolCallSpec {
                    wire_name,
                    arguments,
                }),
                Some(op),
            );
        }
        let assistant_turns_after = conversation
            .assistant_messages
            .iter()
            .filter(|(position, _)| *position > marker_position)
            .count();
        if assistant_turns_after >= UNDISCLOSED_ATTEMPTS {
            let verdict = result_text(&op, Verdict::Undisclosed);
            return (ScriptedDecision::FinalText(verdict), Some(op));
        }
        return (ScriptedDecision::Placeholder, Some(op));
    }

    // The verdict must come from what the read steps returned: write tool
    // results may echo the written content, which embeds this operation's
    // read-back token, and would mask `missing`/`contended` verdicts.
    let read_results = steps
        .iter()
        .zip(&tool_results_after)
        .filter(|(step, _)| matches!(step.kind, StepKind::ReadFile | StepKind::MemoryRead))
        .map(|(_, text)| *text)
        .collect::<Vec<_>>();
    let verdict = compute_verdict(&op, &read_results);
    (
        ScriptedDecision::FinalText(result_text(&op, verdict)),
        Some(op),
    )
}

/// Convenience wrapper returning only the decision; the mock sidecar uses
/// [`decide_with_op`] so the conversation is parsed once. Test-only in the
/// current crate.
#[cfg(test)]
pub(crate) fn decide(
    messages: &[Value],
    available_tool_names: &HashSet<String>,
) -> ScriptedDecision {
    decide_with_op(messages, available_tool_names).0
}

/// Sidecar-side counters for scripted workloads, reported in the run
/// summary for operation attribution.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct ScriptedMockCounters {
    /// Completion requests that carried a scripted marker, per script key.
    #[serde(default)]
    pub(crate) requests_seen: BTreeMap<String, u64>,
    /// Tool calls emitted by the mock, per script key.
    #[serde(default)]
    pub(crate) tool_calls_emitted: BTreeMap<String, u64>,
    /// Interim text responses while waiting for tool disclosure.
    #[serde(default)]
    pub(crate) placeholder_responses: u64,
    /// Final verdict texts emitted, per verdict.
    #[serde(default)]
    pub(crate) final_verdicts: BTreeMap<String, u64>,
}

/// Find the latest scripted marker among the user messages.
fn find_latest_op(conversation: &Conversation) -> Option<(usize, ScriptedOp)> {
    conversation
        .user_messages
        .iter()
        .filter_map(|(position, text)| parse_marker(text).map(|op| (*position, op)))
        .max_by_key(|(position, _)| *position)
}

/// Whether the conversation already contains the final result text for this
/// operation (prevents re-driving a completed sequence).
fn conversation_has_result(
    conversation: &Conversation,
    marker_position: usize,
    op: &ScriptedOp,
) -> bool {
    conversation
        .assistant_messages
        .iter()
        .filter(|(position, _)| *position > marker_position)
        .any(|(_, text)| text.contains(&format!("{RESULT_PREFIX} {}", op.identity())))
}

/// Derive the read-back verdict from the tool results of this operation.
pub(crate) fn compute_verdict(op: &ScriptedOp, tool_result_texts: &[&str]) -> Verdict {
    let own_token = op.readback_token();
    let mut own_found = false;
    let mut foreign_user_found = false;
    let mut same_user_found = false;
    for text in tool_result_texts {
        for token in readback_tokens(text) {
            if token == own_token {
                own_found = true;
            } else if let Some(rest) = token.strip_prefix(&format!("{READBACK_MARKER}_"))
                && let Some((user, _)) = rest.split_once(IDENTITY_SEPARATOR)
            {
                if user == op.user {
                    same_user_found = true;
                } else {
                    foreign_user_found = true;
                }
            }
        }
    }
    if foreign_user_found {
        Verdict::Leak
    } else if own_found {
        Verdict::Confirmed
    } else if same_user_found {
        Verdict::Contended
    } else {
        Verdict::Missing
    }
}

/// Extract every read-back token from tool-result text.
pub(crate) fn readback_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut remainder = text;
    while let Some(start) = remainder.find(READBACK_MARKER) {
        let after = &remainder[start + READBACK_MARKER.len()..];
        let token_end = after
            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '.')
            .unwrap_or(after.len());
        tokens.push(format!("{READBACK_MARKER}{}", &after[..token_end]));
        remainder = &after[token_end..];
    }
    tokens
}

fn result_text(op: &ScriptedOp, verdict: Verdict) -> String {
    format!("{RESULT_PREFIX} {} {}", op.identity(), verdict.as_str())
}

/// Resolve the advertised wire name for a capability id, accepting the
/// `__`-encoded form or the dotted capability id. The bare tool name is
/// deliberately not a candidate: an extension could export a same-named
/// tool and the script would silently drive a different capability.
pub(crate) fn resolve_wire_name(
    available_tool_names: &HashSet<String>,
    capability_id: &str,
) -> Option<String> {
    let encoded = capability_id.replace('.', "__");
    for candidate in [encoded.as_str(), capability_id] {
        if let Some(name) = available_tool_names.get(candidate) {
            return Some(name.clone());
        }
    }
    None
}

/// Build the arguments for a script step from the operation marker.
/// `step_index` locates the step in `op.key.steps()` so split writes can
/// derive their chunk from cumulative fraction boundaries.
pub(crate) fn build_arguments(op: &ScriptedOp, step: &ScriptStep, step_index: usize) -> Value {
    match step.kind {
        StepKind::WriteFile => {
            let path = format!("stress/{}.txt", sanitize_path_segment(&op.identity()));
            let content = scripted_content(op, op.size_bytes);
            serde_json::json!({ "path": path, "content": content })
        }
        StepKind::ReadFile => {
            let path = format!("stress/{}.txt", sanitize_path_segment(&op.identity()));
            serde_json::json!({ "path": path })
        }
        StepKind::MemoryWrite { append, fraction } => {
            let cumulative_after = op
                .key
                .steps()
                .iter()
                .take(step_index + 1)
                .filter_map(|candidate| match candidate.kind {
                    StepKind::MemoryWrite { fraction, .. } => Some(fraction),
                    _ => None,
                })
                .sum::<u8>();
            let size = fraction_chunk(op.size_bytes, fraction, cumulative_after);
            serde_json::json!({
                "target": SHARED_MEMORY_TARGET,
                "content": scripted_content(op, size),
                "append": append,
            })
        }
        StepKind::MemoryRead => {
            serde_json::json!({ "path": SHARED_MEMORY_TARGET })
        }
    }
}

/// Size of the `fraction`/4 chunk of `size_bytes` ending at the cumulative
/// fraction boundary `cumulative_after` (the sum of write fractions up to
/// and including this step). Deriving each chunk from cumulative boundaries
/// assigns any size remainder exactly once, so a split write persists
/// exactly `size_bytes` in total.
fn fraction_chunk(size_bytes: usize, fraction: u8, cumulative_after: u8) -> usize {
    let after = (size_bytes * cumulative_after as usize) / 4;
    let before = (size_bytes * (cumulative_after - fraction) as usize) / 4;
    after - before
}

/// Build deterministic write content of exactly `size_bytes` carrying the
/// operation's read-back token.
pub(crate) fn scripted_content(op: &ScriptedOp, size_bytes: usize) -> String {
    let token = op.readback_token();
    if size_bytes <= token.len() + 1 {
        return token;
    }
    let padding_len = size_bytes - token.len() - 1;
    let padding = "x".repeat(padding_len);
    format!("{token} {padding}")
}

/// Sanitize an identity so it is safe inside a workspace-relative path.
pub(crate) fn sanitize_path_segment(identity: &str) -> String {
    identity
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// Parse the verdict out of a finalized assistant message for this operation.
/// The marker is located with substring matching (like
/// `conversation_has_result`) so a host wrapper around the content does not
/// make the operation unparseable.
pub(crate) fn parse_result_verdict(content: &str, op: &ScriptedOp) -> Option<Verdict> {
    let prefix = format!("{RESULT_PREFIX} {}", op.identity());
    let start = content.find(&prefix)?;
    let rest = content[start + prefix.len()..].trim_start();
    let verdict = rest.split_whitespace().next()?;
    Verdict::parse(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(role: &str, content: &str) -> Value {
        json!({ "role": role, "content": content })
    }

    fn tool_result(content: &str) -> Value {
        message("tool", content)
    }

    fn assistant(content: &str) -> Value {
        message("assistant", content)
    }

    fn user(content: &str) -> Value {
        message("user", content)
    }

    fn tools(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn op(key: ScriptKey, user: &str, op_id: &str, size: usize) -> ScriptedOp {
        ScriptedOp {
            key,
            user: user.to_string(),
            op: op_id.to_string(),
            size_bytes: size,
        }
    }

    #[test]
    fn marker_roundtrips_through_parse() {
        let message = marker_message(ScriptKey::MemoryRoundtrip, "u3", "12", 32768);
        assert_eq!(
            message,
            "ironclaw-stress-tool memory_roundtrip u3__12 32768"
        );
        let parsed = parse_marker(&message).expect("marker parses");
        assert_eq!(parsed.key, ScriptKey::MemoryRoundtrip);
        assert_eq!(parsed.user, "u3");
        assert_eq!(parsed.op, "12");
        assert_eq!(parsed.size_bytes, 32768);
    }

    #[test]
    fn marker_rejects_malformed_input() {
        assert_eq!(parse_marker("not a marker"), None);
        assert_eq!(parse_marker("ironclaw-stress-tool"), None);
        assert_eq!(
            parse_marker("ironclaw-stress-tool unknown_key u0__1 4096"),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u0__1"),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u0__1 0"),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u0__1 4095"),
            None
        );
        assert_eq!(
            parse_marker(&format!(
                "ironclaw-stress-tool memory_roundtrip u0__1 {}",
                MAX_SCRIPTED_DOC_SIZE_BYTES + 1
            )),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u01 4096"),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip __1 4096"),
            None
        );
        // A `/` truncates the read-back token and can never produce a
        // verdict, so the identity is rejected up front.
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u0/x__1 4096"),
            None
        );
        assert_eq!(
            parse_marker("ironclaw-stress-tool memory_roundtrip u0__1/x 4096"),
            None
        );
        // Oversized identity parts are bounded to keep read-back tokens and
        // tool arguments small.
        let oversized_user = "u".repeat(MAX_IDENTITY_PART_LEN + 1);
        assert_eq!(
            parse_marker(&format!(
                "ironclaw-stress-tool memory_roundtrip {oversized_user}__1 4096"
            )),
            None
        );
        let oversized_op = "o".repeat(MAX_IDENTITY_PART_LEN + 1);
        assert_eq!(
            parse_marker(&format!(
                "ironclaw-stress-tool memory_roundtrip u0__{oversized_op} 4096"
            )),
            None
        );
    }

    #[test]
    fn decide_returns_none_without_marker() {
        let messages = vec![user("plain chat message")];
        assert_eq!(
            decide(&messages, &tools(&["builtin__write_file"])),
            ScriptedDecision::None
        );
    }

    #[test]
    fn fresh_memory_roundtrip_emits_write_step() {
        let messages = vec![user(&marker_message(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            4096,
        ))];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.wire_name, "ironclaw__memory__write");
                assert_eq!(spec.arguments["target"], SHARED_MEMORY_TARGET);
                assert_eq!(spec.arguments["append"], false);
                let content = spec.arguments["content"].as_str().expect("string");
                assert_eq!(content.len(), 4096);
                assert!(content.contains(READBACK_MARKER));
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }

    #[test]
    fn memory_roundtrip_read_step_after_one_tool_result() {
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: written"),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.wire_name, "ironclaw__memory__read");
                assert_eq!(spec.arguments["path"], SHARED_MEMORY_TARGET);
            }
            other => panic!("expected read tool call, got {other:?}"),
        }
    }

    #[test]
    fn memory_roundtrip_final_verdict_confirmed() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let token = parsed.readback_token();
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: written"),
            tool_result(&format!("Tool ironclaw.memory.read returned: {token}")),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 confirmed");
            }
            other => panic!("expected final text, got {other:?}"),
        }
    }

    #[test]
    fn verdict_detects_cross_user_leak() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let foreign = format!("{READBACK_MARKER}_u1__3");
        let text = format!("Tool returned: {own} {foreign}");
        assert_eq!(compute_verdict(&parsed, &[&text]), Verdict::Leak);
    }

    #[test]
    fn verdict_detects_same_user_contention() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let text = format!("Tool returned: {own} {same_user}");
        assert_eq!(compute_verdict(&parsed, &[&text]), Verdict::Confirmed);
        let only_same_user = format!("Tool returned: {same_user}");
        assert_eq!(
            compute_verdict(&parsed, &[&only_same_user]),
            Verdict::Contended
        );
    }

    #[test]
    fn verdict_missing_when_no_tokens() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        assert_eq!(compute_verdict(&parsed, &[]), Verdict::Missing);
        let empty = "Tool returned: empty".to_string();
        assert_eq!(compute_verdict(&parsed, &[&empty]), Verdict::Missing);
    }

    #[test]
    fn write_result_echo_does_not_mask_missing_read() {
        // The write tool result echoes the written content (which embeds the
        // operation's own read-back token); the read returns nothing. The
        // verdict must come from the read step alone: Missing, not Confirmed.
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let token = parsed.readback_token();
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let messages = vec![
            user(&marker),
            tool_result(&format!("Tool write returned: wrote {token}")),
            tool_result("Tool read returned: (empty)"),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 missing");
            }
            other => panic!("expected missing verdict, got {other:?}"),
        }
    }

    #[test]
    fn verdict_leak_takes_precedence_over_own_token() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let foreign = format!("{READBACK_MARKER}_u1__3");
        let text = format!("{own} {foreign}");
        assert_eq!(compute_verdict(&parsed, &[&text]), Verdict::Leak);
    }

    #[test]
    fn tool_not_disclosed_starts_with_placeholder_then_undisclosed() {
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let first = vec![user(&marker)];
        assert_eq!(
            decide(&first, &tools(&["ironclaw__memory__read"])),
            ScriptedDecision::Placeholder
        );
        let nudge = vec![
            user(&marker),
            assistant("I'll perform the stress tool action next."),
        ];
        assert_eq!(
            decide(&nudge, &tools(&["ironclaw__memory__read"])),
            ScriptedDecision::Placeholder
        );
        let second_nudge = vec![
            user(&marker),
            assistant("I'll perform the stress tool action next."),
            assistant("I'll perform the stress tool action next."),
        ];
        match decide(&second_nudge, &tools(&["ironclaw__memory__read"])) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 undisclosed");
            }
            other => panic!("expected undisclosed final, got {other:?}"),
        }
    }

    #[test]
    fn completed_operation_is_not_redriven() {
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let token = parsed.readback_token();
        let messages = vec![
            user(&marker),
            tool_result("write ok"),
            tool_result(&format!("read: {token}")),
            assistant("ironclaw-stress-tool result u0__1 confirmed"),
        ];
        assert_eq!(
            decide(
                &messages,
                &tools(&["ironclaw__memory__write", "ironclaw__memory__read"])
            ),
            ScriptedDecision::None
        );
    }

    #[test]
    fn write_file_roundtrip_uses_unique_path_and_steps() {
        let marker = marker_message(ScriptKey::WriteFileRoundtrip, "u2", "7", 8192);
        let messages = vec![user(&marker)];
        match decide(
            &messages,
            &tools(&["builtin__write_file", "builtin__read_file"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.wire_name, "builtin__write_file");
                assert_eq!(spec.arguments["path"], "stress/u2__7.txt");
                assert_eq!(spec.arguments["content"].as_str().unwrap().len(), 8192);
            }
            other => panic!("expected write_file tool call, got {other:?}"),
        }
        let after_write = vec![
            user(&marker),
            tool_result("Tool write_file returned: written"),
        ];
        match decide(
            &after_write,
            &tools(&["builtin__write_file", "builtin__read_file"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.wire_name, "builtin__read_file");
                assert_eq!(spec.arguments["path"], "stress/u2__7.txt");
            }
            other => panic!("expected read_file tool call, got {other:?}"),
        }
    }

    #[test]
    fn wire_name_resolution_accepts_encoded_and_dotted_only() {
        let encoded = tools(&["builtin__write_file"]);
        assert_eq!(
            resolve_wire_name(&encoded, "builtin.write_file").as_deref(),
            Some("builtin__write_file")
        );
        let dotted = tools(&["builtin.write_file"]);
        assert_eq!(
            resolve_wire_name(&dotted, "builtin.write_file").as_deref(),
            Some("builtin.write_file")
        );
        // The bare name is not a candidate: an extension could export a
        // same-named tool and the script would silently bind to it.
        let bare = tools(&["write_file"]);
        assert_eq!(resolve_wire_name(&bare, "builtin.write_file"), None);
        assert_eq!(resolve_wire_name(&tools(&[]), "builtin.write_file"), None);
    }

    #[test]
    fn memory_grow_steps_write_quarter_then_append_three_quarters() {
        let marker = marker_message(ScriptKey::MemoryGrow, "u0", "5", 4096);
        let write = vec![user(&marker)];
        match decide(
            &write,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.arguments["append"], false);
                assert_eq!(spec.arguments["content"].as_str().unwrap().len(), 1024);
            }
            other => panic!("expected initial write, got {other:?}"),
        }
        let after_write = vec![user(&marker), tool_result("write ok")];
        match decide(
            &after_write,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.arguments["append"], true);
                assert_eq!(spec.arguments["content"].as_str().unwrap().len(), 3072);
            }
            other => panic!("expected append, got {other:?}"),
        }
        let after_append = vec![
            user(&marker),
            tool_result("write ok"),
            tool_result("append ok"),
        ];
        match decide(
            &after_append,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.arguments["path"], SHARED_MEMORY_TARGET);
            }
            other => panic!("expected read, got {other:?}"),
        }
    }

    #[test]
    fn memory_mixed_has_four_steps_with_half_sizes() {
        assert_eq!(ScriptKey::MemoryMixed.expected_tool_results(), 4);
        let marker = marker_message(ScriptKey::MemoryMixed, "u1", "2", 32768);
        let messages = vec![user(&marker)];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCall(spec) => {
                assert_eq!(spec.arguments["append"], false);
                assert_eq!(spec.arguments["content"].as_str().unwrap().len(), 16384);
            }
            other => panic!("expected first write, got {other:?}"),
        }
    }

    #[test]
    fn split_writes_preserve_exact_configured_size() {
        // 4097 is not divisible by 4. Chunks derived from cumulative
        // fraction boundaries must still persist exactly the configured
        // size across the split writes.
        for (key, expected_chunks) in [
            (ScriptKey::MemoryGrow, vec![1024, 3073]),
            (ScriptKey::MemoryMixed, vec![2048, 2049]),
        ] {
            let parsed = op(key, "u0", "1", 4097);
            let mut chunks = Vec::new();
            for (index, step) in key.steps().iter().enumerate() {
                if matches!(step.kind, StepKind::MemoryWrite { .. }) {
                    let arguments = build_arguments(&parsed, step, index);
                    chunks.push(arguments["content"].as_str().expect("string").len());
                }
            }
            assert_eq!(chunks, expected_chunks, "chunks for {key:?}");
            assert_eq!(chunks.iter().sum::<usize>(), 4097, "total for {key:?}");
        }
    }

    #[test]
    fn expected_tool_results_per_script() {
        assert_eq!(ScriptKey::WriteFileRoundtrip.expected_tool_results(), 2);
        assert_eq!(ScriptKey::MemoryRoundtrip.expected_tool_results(), 2);
        assert_eq!(ScriptKey::MemoryGrow.expected_tool_results(), 3);
        assert_eq!(ScriptKey::MemoryMixed.expected_tool_results(), 4);
    }

    #[test]
    fn doc_sizes_cycle_in_order() {
        let sizes = vec![4096, 32768, 131072, 1048576];
        assert_eq!(doc_size_for(&sizes, 0), 4096);
        assert_eq!(doc_size_for(&sizes, 3), 1048576);
        assert_eq!(doc_size_for(&sizes, 4), 4096);
        assert_eq!(doc_size_for(&[], 2), 4096);
    }

    #[test]
    fn readback_tokens_extract_from_wrapped_tool_text() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let text = format!("Tool `memory.read` returned: prefix {own} suffix");
        assert_eq!(readback_tokens(&text), vec![own]);
    }

    #[test]
    fn parse_result_verdict_extracts_verdict() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        assert_eq!(
            parse_result_verdict("ironclaw-stress-tool result u0__1 confirmed", &parsed),
            Some(Verdict::Confirmed)
        );
        assert_eq!(
            parse_result_verdict("ironclaw-stress-tool result u0__1 leak", &parsed),
            Some(Verdict::Leak)
        );
        assert_eq!(parse_result_verdict("unrelated text", &parsed), None);
        assert_eq!(
            parse_result_verdict("ironclaw-stress-tool result u0__1 unknown", &parsed),
            None
        );
    }

    #[test]
    fn content_padding_is_exact_and_deterministic() {
        let parsed = op(ScriptKey::MemoryGrow, "u4", "9", 1000);
        let content = scripted_content(&parsed, 1000);
        assert_eq!(content.len(), 1000);
        assert!(content.starts_with(&parsed.readback_token()));
        assert_eq!(scripted_content(&parsed, 1000), content);
        let tiny = scripted_content(&parsed, 4);
        assert_eq!(tiny, parsed.readback_token());
    }

    #[test]
    fn marker_identity_survives_sanitization() {
        assert_eq!(sanitize_path_segment("u0__1"), "u0__1");
        assert_eq!(sanitize_path_segment("h2-3"), "h2-3");
        assert_eq!(sanitize_path_segment("a b/c"), "a-b-c");
    }

    #[test]
    fn message_text_accepts_parts_array() {
        let parts = json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "ironclaw-stress-tool memory_roundtrip u0__1 4096"},
                {"type": "text", "text": " more"}
            ]
        });
        assert_eq!(
            message_text(&parts).unwrap(),
            "ironclaw-stress-tool memory_roundtrip u0__1 4096 more"
        );
        assert_eq!(message_text(&json!({"role": "user"})), None);
    }
}
