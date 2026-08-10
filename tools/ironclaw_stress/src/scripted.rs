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
//! (`IRONCLAW_STRESS_READBACK_<user>__<op>`), a plan-step ordinal token,
//! and the full canonical marker in its content, so a later write call's
//! serialized arguments carry the plan identity even after the original
//! user marker is compacted out of a long sequential plan, and consecutive
//! full-size chunks of one document never present the production
//! no-progress guard with an identical call signature. Every scripted
//! sequence ends
//! with a memory checkpoint step: a plain
//! `ironclaw.memory.read` for documents at or below the inline-safe
//! ceiling, a bounded `ironclaw.memory.search` above it (a full read of a
//! 1 MiB document plus its JSON envelope would exceed the first-party
//! output cap). The verdict is derived from what the tool actually
//! returned through the production path:
//!
//! - `confirmed`: the read returned exactly this operation's token and no
//!   same-user token.
//! - `contended`: the read returned a same-user token (another operation of
//!   the same user overwrote or interleaved the document between write and
//!   read — hot-document contention, expected under concurrent same-user
//!   writers, counted not failed). A text carrying this operation's token
//!   alongside a same-user token is still contended, never confirmed: mixed
//!   content proves another same-user write reached the document.
//! - `leak`: the read returned another user's token (cross-user isolation
//!   violation — a hard failure).
//! - `missing`: the read returned no token at all (durable write lost — a
//!   hard failure).
//! - `undisclosed`: the required tool was never advertised to the model
//!   (progressive-disclosure / agent-surface regression — a hard failure).
//! - `failure`: an aligned tool call returned a structured error observation
//!   (`status: "error"`), so a write/append or checkpoint step cannot be
//!   trusted even when a later checkpoint finds this operation's token (a
//!   hard failure). A write/append step is retried only when the
//!   observation's `recovery.same_call_retry` explicitly permits replaying
//!   the identical call. `allowed` retries immediately;
//!   `allowed_after_delay` must include `retry_after_ms`, and the driver
//!   emits placeholders until that delay expires. The same plan step is
//!   then re-opened with a fresh call id up to [`MAX_WRITE_ATTEMPTS`] total
//!   attempts, so a transient contention error (CAS failure rendered with
//!   `same_call_retry=allowed`) that succeeds on retry resumes the plan.
//!   A `forbidden`, `requires_changed_input`, or `not_useful` constraint —
//!   or an observation whose recovery constraint or required delay is
//!   missing or unparseable — is an immediate sticky failure with no
//!   re-emission, and checkpoint errors stay immediate failures.
//!
//! The module performs no external I/O. Decisions depend on conversation
//! JSON, advertised tool names, and a monotonic instant supplied by the
//! driver; tests inject that instant so delayed recovery stays deterministic.
//!
//! # Stateful driver
//!
//! The pure [`decide_with_op`] derives progress from the conversation, so
//! it stops working once the production agent loop compacts the original
//! user marker and most tool results out of a long sequential plan. The
//! mock sidecar therefore owns a bounded [`ScriptedDriver`]: per-operation
//! sessions keyed by the full scripted identity (script, user, op, size).
//! Each session tracks which plan steps were emitted and which tool
//! results arrived, digests checkpoint observations and structured errors
//! as they arrive (so compaction cannot erase them), tracks the
//! structured-error attempts of the current write/append step (a write
//! whose error observation explicitly allows an identical replay is
//! re-opened for a bounded retry with a fresh call id before any failure
//! verdict), and remembers the id of its last emitted tool call.
//! A request is attributed to its operation
//! from the latest marker in user content, the canonical marker embedded
//! in an assistant tool call's serialized arguments, the recorded call id,
//! or a read-back token in a tool result — in that order. A request whose
//! pending call has no result yet never advances the plan and never
//! re-emits a duplicate call; it gets an interim placeholder instead.
//! Final verdicts remove the session; the driver evicts the oldest session
//! when its capacity is exceeded, so the map stays bounded.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

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
/// a read-back token, a plan-step ordinal token, the canonical marker, and
/// deterministic padding; below 4 KiB the token and marker dominate and
/// sizes lose meaning, and the issue's workloads start at 4 KiB.
pub(crate) const MIN_SCRIPTED_DOC_SIZE_BYTES: usize = 4096;
/// Upper bound for a scripted document size, in bytes.
pub(crate) const MAX_SCRIPTED_DOC_SIZE_BYTES: usize = 8 * 1024 * 1024;
/// Upper bound for the content bytes of one scripted memory write tool
/// call. Provider-emitted tool argument JSON is capped at 64 KiB by the
/// host; 60 KiB of content leaves room for the JSON envelope (keys,
/// quoting, whitespace) inside that cap, so a document larger than one
/// chunk is written as a replace plus bounded appends.
pub(crate) const MAX_MEMORY_WRITE_CHUNK_BYTES: usize = 60 * 1024;
/// Largest scripted memory document that a checkpoint may inline through
/// `ironclaw.memory.read`. A 1 MiB document plus its JSON envelope (keys,
/// quoting, whitespace, tool-result wrapping) exceeds the first-party
/// 1,048,576-byte output cap, so documents above this ceiling are verified
/// with a bounded `ironclaw.memory.search` instead.
pub(crate) const INLINE_SAFE_MEMORY_READ_BYTES: usize = 1024 * 1024 - 4 * 1024;
/// Query used by scripted memory search checkpoints. The read-back marker
/// is the deterministic substring every scripted write embeds, so search
/// snippets expose own, same-user, and foreign-user tokens alike.
///
/// The query is an exact literal the native provider's bounded search
/// preview reproduces verbatim: oversized snippets are returned as bounded
/// excerpts around every exact occurrence, so identity tokens past the
/// first 8192 bytes — e.g. a same-user contender beyond the head of a 1 MiB
/// hot document — stay visible to the classifier. A backend match the
/// provider cannot reproduce literally (stemming or other normalization)
/// degrades to an honest bounded head, never a claimed match. The scripted
/// workload depends on the first behavior, so this constant must stay the
/// literal [`READBACK_MARKER`] and no case/word variant of it.
pub(crate) const MEMORY_SEARCH_QUERY: &str = READBACK_MARKER;
/// Result cap for scripted memory search checkpoints. Bounded so the
/// returned hit list stays far below the output cap while still exposing
/// the operation's own document and any contending same-user writes.
pub(crate) const MEMORY_SEARCH_LIMIT: usize = 20;
/// Number of assistant turns to wait for a scripted tool to be disclosed
/// before declaring the operation `undisclosed`.
pub(crate) const UNDISCLOSED_ATTEMPTS: usize = 2;
/// Total attempts for one scripted write/append step when the observation
/// explicitly permits replaying the identical call. A structured error
/// observation on a write may be a transient contention failure (the host
/// renders such calls with `same_call_retry=allowed` under CAS
/// contention), so the driver re-opens the same step with a fresh call id
/// instead of failing the operation on the first error — but only for
/// `allowed`, or for `allowed_after_delay` with an explicit
/// `retry_after_ms` after that delay has elapsed. After this many failed
/// attempts the step is recorded as failed and the sticky error evidence
/// yields a hard [`Verdict::Failure`]. The bound
/// keeps repeated identical calls below the production no-progress guard
/// and the retry state bounded.
pub(crate) const MAX_WRITE_ATTEMPTS: usize = 3;
/// Upper bound on concurrently active scripted sessions kept by the mock
/// sidecar's [`ScriptedDriver`]. Each session holds only the operation
/// identity, step counters, the last emitted call id, and digested
/// checkpoint evidence (a few hundred bytes), so 256 sessions stay
/// comfortably bounded regardless of how many operations race; when the
/// cap is exceeded the oldest session is evicted and its operation times
/// out driver-side rather than growing the map without limit.
pub(crate) const DEFAULT_SCRIPTED_SESSION_CAPACITY: usize = 256;
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
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScriptKey {
    /// `builtin.write_file` then `builtin.read_file` of the same unique
    /// workspace path.
    #[value(name = "write_file_roundtrip")]
    WriteFileRoundtrip,
    /// `ironclaw.memory.write` of the whole target (one replace plus
    /// zero-or-more appends when the document exceeds one bounded chunk)
    /// then a size-aware checkpoint of the shared relative memory target
    /// (`ironclaw.memory.read`, or `ironclaw.memory.search` above the
    /// inline-safe ceiling).
    #[value(name = "memory_roundtrip")]
    MemoryRoundtrip,
    /// `ironclaw.memory.write` of the quarter (replace plus appends), then
    /// append the three quarters, then a size-aware checkpoint of the
    /// shared target — growing-append slope workload.
    #[value(name = "memory_grow")]
    MemoryGrow,
    /// `ironclaw.memory.write` of the first half, checkpoint, append the
    /// second half, checkpoint of the shared target — mixed read/write
    /// workload.
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
    /// Size-aware: a memory document larger than one write chunk yields one
    /// tool result per chunk plus the read checkpoints.
    pub(crate) fn expected_tool_results(self, size_bytes: usize) -> usize {
        self.steps(size_bytes).len()
    }

    /// Tool-call sequence for one operation of `size_bytes`. Memory
    /// documents are written in chunks of at most
    /// [`MAX_MEMORY_WRITE_CHUNK_BYTES`]: the first chunk of the first
    /// phase replaces, every other chunk appends, and each phase persists
    /// exactly its configured share (roundtrip: whole; grow: quarter then
    /// three quarters; mixed: first half then second half).
    pub(crate) fn steps(self, size_bytes: usize) -> Vec<ScriptStep> {
        let mut steps = match self {
            Self::WriteFileRoundtrip => vec![
                ScriptStep {
                    capability_id: "builtin.write_file",
                    kind: StepKind::WriteFile,
                    step_index: 0,
                },
                ScriptStep {
                    capability_id: "builtin.read_file",
                    kind: StepKind::ReadFile,
                    step_index: 0,
                },
            ],
            Self::MemoryRoundtrip => {
                let mut steps = memory_phase_steps(size_bytes, true);
                steps.push(memory_checkpoint_step(size_bytes));
                steps
            }
            Self::MemoryGrow => {
                let quarter = size_bytes / 4;
                let mut steps = memory_phase_steps(quarter, true);
                steps.extend(memory_phase_steps(size_bytes - quarter, false));
                steps.push(memory_checkpoint_step(size_bytes));
                steps
            }
            Self::MemoryMixed => {
                let first_half = size_bytes / 2;
                let mut steps = memory_phase_steps(first_half, true);
                steps.push(memory_checkpoint_step(size_bytes));
                steps.extend(memory_phase_steps(size_bytes - first_half, false));
                steps.push(memory_checkpoint_step(size_bytes));
                steps
            }
        };
        // Stamp every step with its position in the assembled plan. The
        // ordinal distinguishes write chunks that are otherwise identical
        // (same capability, same chunk size, same read-back token and
        // marker), so repeated full-size appends of one document never
        // share a call signature with the production no-progress guard.
        for (index, step) in steps.iter_mut().enumerate() {
            step.step_index = index;
        }
        steps
    }
}

/// One memory read step of a scripted sequence.
fn memory_read_step() -> ScriptStep {
    ScriptStep {
        capability_id: "ironclaw.memory.read",
        kind: StepKind::MemoryRead,
        // Stamped by `ScriptKey::steps` with the step's plan position.
        step_index: 0,
    }
}

/// Memory checkpoint step for a document of `size_bytes` (the operation's
/// configured size). Documents at or below
/// [`INLINE_SAFE_MEMORY_READ_BYTES`] are verified with
/// `ironclaw.memory.read` of the shared target; larger documents are
/// verified with a bounded `ironclaw.memory.search`, because a full read
/// of a 1 MiB document plus its JSON envelope exceeds the first-party
/// 1,048,576-byte output cap.
fn memory_checkpoint_step(size_bytes: usize) -> ScriptStep {
    if size_bytes <= INLINE_SAFE_MEMORY_READ_BYTES {
        memory_read_step()
    } else {
        ScriptStep {
            capability_id: "ironclaw.memory.search",
            kind: StepKind::MemorySearch,
            // Stamped by `ScriptKey::steps` with the step's plan position.
            step_index: 0,
        }
    }
}

/// Memory write steps persisting exactly `phase_size` bytes: the first
/// chunk replaces when `replace` is set (the operation's first write),
/// every later chunk appends. Each chunk is at most
/// [`MAX_MEMORY_WRITE_CHUNK_BYTES`] so the provider-emitted tool argument
/// JSON stays under the host's 64 KiB cap, and the split never leaves a
/// remainder shorter than the read-back-token-plus-marker bound, so every
/// chunk carries exact scripted content with its identity.
fn memory_phase_steps(phase_size: usize, replace: bool) -> Vec<ScriptStep> {
    bounded_chunks(phase_size, MAX_MEMORY_WRITE_CHUNK_BYTES)
        .into_iter()
        .enumerate()
        .map(|(index, size_bytes)| ScriptStep {
            capability_id: "ironclaw.memory.write",
            kind: StepKind::MemoryWrite {
                append: !(replace && index == 0),
                size_bytes,
            },
            // Stamped by `ScriptKey::steps` with the step's plan position.
            step_index: 0,
        })
        .collect()
}

/// Split `size_bytes` into near-equal chunks summing exactly to
/// `size_bytes`, each at most `max_chunk`. Chunks differ by at most one
/// byte, so a phase never ends in a tiny remainder: the smallest split
/// chunk is `max_chunk / 2` (for a two-chunk split), far above the
/// read-back-token-plus-marker bound.
fn bounded_chunks(size_bytes: usize, max_chunk: usize) -> Vec<usize> {
    if size_bytes == 0 {
        return Vec::new();
    }
    let chunk_count = size_bytes.div_ceil(max_chunk);
    let base = size_bytes / chunk_count;
    let mut chunks = vec![base; chunk_count];
    for chunk in chunks.iter_mut().take(size_bytes % chunk_count) {
        *chunk += 1;
    }
    chunks
}

/// One tool call of a scripted sequence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptStep {
    /// Dotted capability id the step targets (e.g. `builtin.write_file`).
    pub(crate) capability_id: &'static str,
    /// Zero-based position of this step in the operation's full plan,
    /// stamped by [`ScriptKey::steps`]. Write steps embed it as a
    /// deterministic ordinal token in their content, so consecutive
    /// full-size chunks — same capability, same chunk size, same token
    /// and marker — still differ in their serialized arguments.
    pub(crate) step_index: usize,
    pub(crate) kind: StepKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StepKind {
    WriteFile,
    ReadFile,
    MemoryWrite {
        append: bool,
        /// Content bytes for this chunk of the document. Every chunk is at
        /// most [`MAX_MEMORY_WRITE_CHUNK_BYTES`], and the chunks of one
        /// phase sum exactly to that phase's share of the document.
        size_bytes: usize,
    },
    MemoryRead,
    /// Bounded memory search checkpoint for documents above
    /// [`INLINE_SAFE_MEMORY_READ_BYTES`]: queries
    /// [`MEMORY_SEARCH_QUERY`] and caps results at
    /// [`MEMORY_SEARCH_LIMIT`], so the returned snippets stay well below
    /// the output cap while still carrying read-back tokens.
    MemorySearch,
}

/// A parsed scripted operation marker. The full identity (script key, user,
/// op, size) doubles as the session key of the stateful
/// [`ScriptedDriver`], so concurrent operations never share state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// A structured error observation (`status: "error"`) on an aligned
    /// tool call: the write/append or checkpoint step failed, so no
    /// read-back classification is trustworthy.
    Failure,
}

impl Verdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Contended => "contended",
            Self::Leak => "leak",
            Self::Missing => "missing",
            Self::Undisclosed => "undisclosed",
            Self::Failure => "failure",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "confirmed" => Some(Self::Confirmed),
            "contended" => Some(Self::Contended),
            "leak" => Some(Self::Leak),
            "missing" => Some(Self::Missing),
            "undisclosed" => Some(Self::Undisclosed),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }

    /// Whether this verdict is a hard failure for the driver.
    pub(crate) fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Leak | Self::Missing | Self::Undisclosed | Self::Failure
        )
    }
}

/// What the sidecar should answer for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptedDecision {
    /// Emit the next script step as exactly one tool call. The call is the
    /// next step of the operation's plan in order, never batched with
    /// later steps: memory writes, appends, and checkpoints of one
    /// document execute one capability call per model response, so
    /// dependent steps cannot race.
    ToolCalls(Vec<ToolCallSpec>),
    /// Emit the final verdict text.
    FinalText(String),
    /// Keep the current completion request open for the remaining provider
    /// retry delay, then evaluate that same request again. The HTTP mock
    /// consumes this internally and never serializes it as an assistant
    /// completion.
    RetryAfter(Duration),
    /// Emit an interim text response while waiting for tool disclosure.
    Placeholder,
    /// No scripted marker in this conversation; fall through to the default
    /// text path.
    None,
}

/// One tool call to emit, with the exact wire name advertised in the
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
/// array is walked exactly once. Every response emits at most one tool
/// call — the next available step of the operation's plan in order — so
/// writes and checkpoints of one document are strictly serialized and a
/// 1 MiB memory plan (19-20 steps) still fits the production agent loop's
/// default iteration backstop without batching.
#[cfg(test)]
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
    let steps = op.key.steps(op.size_bytes);
    if step_index < steps.len() {
        // One tool call per response: the next plan step is emitted alone,
        // so the capability host executes each write, append, and
        // checkpoint of this document in plan order, one capability call
        // per model response. Dependent replace/append calls therefore
        // cannot execute concurrently. A 1 MiB memory plan is 19-20 steps,
        // far inside the agent loop's production default iteration
        // backstop (>= 1024), so plans are never batched. An unavailable
        // next step is never skipped: the placeholder / undisclosed path
        // below is unchanged.
        let step = &steps[step_index];
        if let Some(wire_name) = resolve_wire_name(available_tool_names, step.capability_id) {
            // A planned chunk size always fits the read-back token plus the
            // canonical marker (the smallest planned chunk, a 1024-byte
            // quarter of a 4 KiB document, is far above the bound for the
            // longest parseable identities), so this is unreachable in
            // production; fail closed by emitting no scripted call rather
            // than writing content that lacks plan identity.
            let Some(arguments) = build_arguments(&op, step) else {
                return (ScriptedDecision::None, Some(op));
            };
            let calls = vec![ToolCallSpec {
                wire_name,
                arguments,
            }];
            return (ScriptedDecision::ToolCalls(calls), Some(op));
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

    // A structured error observation on any aligned tool result (write,
    // append, or checkpoint) is a hard failure: write results are otherwise
    // discarded for the read-back scan (their echoed content would mask
    // `missing`/`contended` verdicts), and a failed write or checkpoint
    // cannot confirm durable state even when a later search finds this
    // operation's token from another chunk. The results are positionally
    // aligned to the plan steps, one tool result per response.
    let aligned_failure = steps
        .iter()
        .zip(&tool_results_after)
        .any(|(_, text)| is_structured_error_result(text));
    if aligned_failure {
        let verdict = result_text(&op, Verdict::Failure);
        return (ScriptedDecision::FinalText(verdict), Some(op));
    }

    // The verdict must come from what the checkpoint steps returned: write
    // tool results may echo the written content, which embeds this
    // operation's read-back token, and would mask `missing`/`contended`
    // verdicts.
    let read_results = steps
        .iter()
        .zip(&tool_results_after)
        .filter(|(step, _)| {
            matches!(
                step.kind,
                StepKind::ReadFile | StepKind::MemoryRead | StepKind::MemorySearch
            )
        })
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

/// Per-operation driver state for the mock sidecar's [`ScriptedDriver`].
/// Everything the driver needs to continue a plan after compaction: how
/// many steps were emitted, which emitted call is awaiting its result, the
/// structured-error attempt count of the current write step (for bounded
/// retries), and the digested evidence of checkpoint steps and structured
/// errors, all recorded as they arrive so compacted-away messages cannot
/// erase them.
#[derive(Debug, Clone)]
pub(crate) struct ScriptedSession {
    /// The operation this session drives. The full identity (script key,
    /// user, op, size) is the session key.
    pub(crate) op: ScriptedOp,
    /// Plan steps whose tool calls have been emitted in mock responses.
    pub(crate) emitted: usize,
    /// Emitted steps whose tool results have been observed in requests.
    /// The plan advances only when `completed == emitted`, so a request
    /// without a new result never skips or duplicates a step.
    pub(crate) completed: usize,
    /// Call id of the most recently emitted step. The mock generates ids
    /// from its request counter and the agent loop echoes them back in
    /// `tool_call_id`, so a compacted request can be attributed to this
    /// session and its pending result matched exactly.
    pub(crate) last_call_id: Option<String>,
    /// Digested evidence of each checkpoint step (read/search), in
    /// checkpoint order, recorded when the result arrives.
    pub(crate) checkpoint_verdicts: Vec<Verdict>,
    /// Any aligned tool result (write, append, or checkpoint) was a
    /// structured error observation whose write/append step failed — a
    /// non-retryable constraint (`forbidden`, `requires_changed_input`,
    /// `not_useful`, missing recovery, or delayed retry without an explicit
    /// delay) or an explicitly allowed replay that exhausted its bounded
    /// retries. Sticky for the operation: a failed write stays a hard
    /// [`Verdict::Failure`] even when a later checkpoint finds this
    /// operation's token. Transient errors that succeed on retry do not set
    /// this flag.
    pub(crate) structured_error: bool,
    /// Structured-error attempts of the current (last emitted) write or
    /// append step, recorded as they arrive. `allowed` retries immediately;
    /// `allowed_after_delay` requires `retry_after_ms` and waits until that
    /// deadline. While below [`MAX_WRITE_ATTEMPTS`] the step stays open
    /// instead of advancing. The counter resets when the step completes
    /// (success or final failure) or a new step is emitted.
    pub(crate) write_attempts: usize,
    /// Whether the last recorded result of the pending write step was a
    /// structured error that explicitly allowed an identical replay, with
    /// attempts remaining, so [`respond_for_session`] re-opens that same
    /// step for another emission (fresh call id) instead of emitting a
    /// placeholder or the next plan step.
    pub(crate) write_retry_pending: bool,
    /// Earliest instant at which an `allowed_after_delay` retry may be
    /// emitted. `None` means an explicitly immediate retry.
    pub(crate) retry_not_before: Option<Instant>,
    /// Interim placeholder responses emitted while the next step's tool is
    /// not yet advertised; reaching [`UNDISCLOSED_ATTEMPTS`] finalizes the
    /// operation as `undisclosed`.
    pub(crate) undisclosed_turns: usize,
    /// Monotonic creation order, used to evict the oldest session when the
    /// driver capacity is exceeded.
    pub(crate) created_seq: u64,
}

impl ScriptedSession {
    fn new(op: ScriptedOp, created_seq: u64) -> Self {
        ScriptedSession {
            op,
            emitted: 0,
            completed: 0,
            last_call_id: None,
            checkpoint_verdicts: Vec::new(),
            structured_error: false,
            write_attempts: 0,
            write_retry_pending: false,
            retry_not_before: None,
            undisclosed_turns: 0,
            created_seq,
        }
    }
}

/// Bounded stateful scripted driver owned by the mock LLM sidecar. Keys
/// sessions by the full scripted identity (script key, user, op, size), so
/// concurrent operations — regular writers and hot writers racing on one
/// thread — never share progress. Each request is attributed to its
/// operation from the latest marker in user content, the canonical marker
/// embedded in an assistant tool call's serialized arguments, a recorded
/// call id, or a read-back token in a tool result, in that order. Final
/// verdicts remove the session; exceeding the capacity evicts the oldest
/// session, keeping the map bounded.
#[derive(Debug)]
pub(crate) struct ScriptedDriver {
    sessions: BTreeMap<ScriptedOp, ScriptedSession>,
    /// Call id -> session key, for attributing compacted requests whose
    /// arguments carry no marker (e.g. a search checkpoint pair). Each
    /// session contributes at most its last emitted call id.
    call_to_session: BTreeMap<String, ScriptedOp>,
    /// Maximum concurrently active sessions; the oldest is evicted first.
    capacity: usize,
    next_seq: u64,
}

impl ScriptedDriver {
    pub(crate) fn new(capacity: usize) -> Self {
        ScriptedDriver {
            sessions: BTreeMap::new(),
            call_to_session: BTreeMap::new(),
            capacity: capacity.max(1),
            next_seq: 0,
        }
    }

    /// Number of currently active sessions.
    #[cfg(test)]
    pub(crate) fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Call id recorded for `op`'s most recently emitted step, if any.
    #[cfg(test)]
    pub(crate) fn last_call_id_for(&self, op: &ScriptedOp) -> Option<&str> {
        self.sessions
            .get(op)
            .and_then(|session| session.last_call_id.as_deref())
    }

    /// Decide what the mock sidecar should answer for a chat completion
    /// request, driving the stateful per-operation session. `next_call_id`
    /// is the id the caller will put on a tool call emitted in this
    /// response, recorded on the session so the following request's tool
    /// result can be matched exactly.
    pub(crate) fn decide(
        &mut self,
        request: &Value,
        available_tool_names: &HashSet<String>,
        next_call_id: &str,
    ) -> (ScriptedDecision, Option<ScriptedOp>) {
        self.decide_at(request, available_tool_names, next_call_id, Instant::now())
    }

    fn decide_at(
        &mut self,
        request: &Value,
        available_tool_names: &HashSet<String>,
        next_call_id: &str,
        now: Instant,
    ) -> (ScriptedDecision, Option<ScriptedOp>) {
        let Some(messages) = request.get("messages").and_then(Value::as_array) else {
            return (ScriptedDecision::None, None);
        };
        let conversation = Conversation::from_messages(messages);

        // Attribute the request to an operation: latest marker in user
        // content, then the canonical marker embedded in assistant tool-call
        // arguments, then the recorded call id / read-back token fallbacks
        // for compacted checkpoint pairs.
        let op = recover_op(messages).or_else(|| self.op_for_request(messages));
        let Some(op) = op else {
            return (ScriptedDecision::None, None);
        };

        let exists = self.sessions.contains_key(&op);
        if !exists {
            // A marker for an operation whose session is gone is either a
            // brand-new operation or a finalized one re-requested: never
            // re-drive a sequence whose verdict text is already in the
            // conversation.
            if conversation_has_result_anywhere(&conversation, &op) {
                return (ScriptedDecision::None, Some(op));
            }
            self.insert_session(op.clone());
        }

        let decision = {
            let session = self.sessions.get_mut(&op).expect("session present");
            advance_session(session, messages, &conversation, now);
            let previous_call_id = session.last_call_id.clone();
            let decision = respond_for_session(session, available_tool_names, next_call_id, now);
            if matches!(decision, ScriptedDecision::ToolCalls(_)) {
                // Keep the call-id index in sync: each session contributes
                // at most its last emitted call id, so the index stays
                // bounded with the sessions.
                if let Some(old) = previous_call_id {
                    self.call_to_session.remove(&old);
                }
                if let Some(new) = session.last_call_id.as_deref() {
                    self.call_to_session
                        .insert(new.to_string(), session.op.clone());
                }
            }
            decision
        };
        if matches!(decision, ScriptedDecision::FinalText(_)) {
            self.finalize_session(&op);
        }
        (decision, Some(op))
    }

    fn insert_session(&mut self, op: ScriptedOp) {
        if self.sessions.len() >= self.capacity {
            // Bound the map: evict the oldest active session. An in-flight
            // operation that loses its slot times out driver-side rather
            // than growing state without limit.
            if let Some(oldest_key) = self
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.created_seq)
                .map(|(key, _)| key.clone())
            {
                self.finalize_session(&oldest_key);
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.sessions
            .insert(op.clone(), ScriptedSession::new(op, seq));
    }

    fn finalize_session(&mut self, key: &ScriptedOp) {
        if let Some(session) = self.sessions.remove(key)
            && let Some(call_id) = session.last_call_id.as_deref()
        {
            self.call_to_session.remove(call_id);
        }
    }

    /// Fallback attribution for requests whose messages carry no parseable
    /// marker: an assistant tool call id this driver emitted (search
    /// checkpoints have no marker in their arguments), then a read-back
    /// token in a tool result. Both are exact identity matches, so
    /// concurrent operations stay isolated; duplicate occurrences of one
    /// operation's token dedupe into a single match.
    fn op_for_request(&self, messages: &[Value]) -> Option<ScriptedOp> {
        for message in messages {
            if message.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
                continue;
            };
            for call in calls {
                let Some(id) = call.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(key) = self.call_to_session.get(id) {
                    return self.sessions.get(key).map(|session| session.op.clone());
                }
            }
        }
        let mut matched: Vec<ScriptedOp> = Vec::new();
        for message in messages {
            if message.get("role").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let Some(text) = message_text(message) else {
                continue;
            };
            for token in readback_tokens(&text) {
                let Some(rest) = token.strip_prefix(READBACK_MARKER) else {
                    continue;
                };
                // The token is `IRONCLAW_STRESS_READBACK_<identity>`: the
                // marker is followed by a single underscore, then the
                // `user__op` identity (matching `ScriptedOp::readback_token`
                // and `classify_checkpoint`'s scan).
                let Some(identity) = rest.strip_prefix('_') else {
                    continue;
                };
                for session in self
                    .sessions
                    .values()
                    .filter(|session| session.op.identity() == identity)
                {
                    let op = session.op.clone();
                    // A token may occur more than once (an observation
                    // preview can echo it several times): duplicate
                    // occurrences of one operation's token are one match.
                    if !matched.contains(&op) {
                        matched.push(op);
                    }
                }
            }
        }
        // A read-back identity is unambiguous for one driver task; require
        // exactly one match so a hypothetical collision never misattributes.
        if matched.len() == 1 {
            matched.pop()
        } else {
            None
        }
    }
}

/// Recover the operation a request drives from parseable markers: the
/// latest marker in a user message, then the latest canonical marker
/// embedded in an assistant tool call's serialized arguments (every
/// scripted write embeds its full marker, so the plan identity survives
/// compaction of the original user message). Tool results are never
/// scanned for markers: an observation preview could echo a marker of a
/// different operation.
fn recover_op(messages: &[Value]) -> Option<ScriptedOp> {
    for message in messages.iter().rev() {
        if let Some(text) = message_text(message)
            && let Some(op) = parse_marker(&text)
        {
            return Some(op);
        }
        let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for call in calls {
            let Some(arguments) = call
                .get("function")
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Some(op) = parse_marker_in_arguments(arguments) {
                return Some(op);
            }
        }
    }
    None
}

/// Parse the canonical marker embedded in serialized tool-call arguments.
/// Scripted write content is `<token> <ordinal> <marker> <padding>`; the
/// token, ordinal, marker, and padding characters are all JSON-safe (ASCII
/// alphanumerics, spaces, `_`, `-`, `.`), so the marker appears literally
/// in the escaped JSON string and needs no unescaping. Returns `None` when
/// the text carries no marker.
fn parse_marker_in_arguments(text: &str) -> Option<ScriptedOp> {
    let start = text.find(MARKER_PREFIX)?;
    let marker = text[start..]
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    parse_marker(&marker)
}

/// Whether the conversation already contains the final result text for
/// this operation (prevents re-driving a completed sequence). The verdict
/// is parsed exactly, so `u0__1` never matches `u0__10`'s result.
fn conversation_has_result_anywhere(conversation: &Conversation, op: &ScriptedOp) -> bool {
    conversation
        .assistant_messages
        .iter()
        .any(|(_, text)| parse_result_verdict(text, op).is_some())
}

/// The only call that can be pending is the last emitted one (the plan
/// never emits a new step until the previous result arrived), so at most
/// one step is completed per request. Prefer an exact `tool_call_id` match;
/// host-normalized ids may differ from the provider id, so otherwise accept
/// the newest message only when it is a tool result in this operation's
/// already-attributed request.
fn advance_session(
    session: &mut ScriptedSession,
    messages: &[Value],
    conversation: &Conversation,
    now: Instant,
) {
    if session.completed >= session.emitted {
        return;
    }
    if session.write_retry_pending {
        // The last result was already consumed. Requests emitted while a
        // delayed retry is waiting carry that same result and must not count
        // it as another failed attempt.
        return;
    }
    let Some(last_call_id) = session.last_call_id.as_deref() else {
        return;
    };
    let mut matched: Option<String> = None;
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if message.get("tool_call_id").and_then(Value::as_str) == Some(last_call_id) {
            matched = message_text(message);
            break;
        }
    }
    let result_text = matched.or_else(|| {
        newest_message_is_tool_result(conversation)
            .then(|| {
                conversation
                    .tool_results
                    .last()
                    .map(|(_, text)| text.clone())
            })
            .flatten()
    });
    if let Some(result_text) = result_text {
        record_step_result(session, &result_text, now);
    }
}

/// The conversation's newest message is a tool result (the pending call's
/// result arrives as the newest message from the agent loop).
fn newest_message_is_tool_result(conversation: &Conversation) -> bool {
    let tool_last = conversation
        .tool_results
        .last()
        .map(|(position, _)| *position);
    let assistant_last = conversation
        .assistant_messages
        .last()
        .map(|(position, _)| *position);
    let user_last = conversation
        .user_messages
        .last()
        .map(|(position, _)| *position);
    match (newest_of(tool_last, assistant_last, user_last), tool_last) {
        (Some(newest), Some(tool_last)) => newest == tool_last,
        _ => false,
    }
}

fn newest_of(a: Option<usize>, b: Option<usize>, c: Option<usize>) -> Option<usize> {
    a.max(b).max(c)
}

/// Record the tool result of the just-completed step (the last emitted
/// one) into the session. Checkpoint results are digested into verdict
/// evidence immediately and stay immediate failures. A structured error on
/// a write/append step is retried as the same plan step with a fresh call id
/// only when recovery is `allowed`, or is `allowed_after_delay` with an
/// explicit `retry_after_ms` after that delay expires, and the attempt count
/// is below [`MAX_WRITE_ATTEMPTS`]. The completed counter does not advance
/// on a retry, and sticky error evidence is set only on the final failed
/// attempt, so a transient contention failure that succeeds on retry resumes
/// normal progression. Any other structured error — `forbidden`,
/// `requires_changed_input`, `not_useful`, missing/unparseable recovery, or
/// delayed recovery without a delay — is an immediate sticky failure: the
/// step is never re-opened.
fn record_step_result(session: &mut ScriptedSession, result_text: &str, now: Instant) {
    let step_index = session.emitted - 1;
    let steps = session.op.key.steps(session.op.size_bytes);
    let step = &steps[step_index];
    match step.kind {
        StepKind::ReadFile | StepKind::MemoryRead | StepKind::MemorySearch => {
            // Checkpoint errors are not retried: the failed read/search
            // cannot confirm durable state, and the hard failure is
            // immediate.
            session
                .checkpoint_verdicts
                .push(classify_checkpoint(&session.op, result_text));
            session.completed += 1;
        }
        StepKind::WriteFile | StepKind::MemoryWrite { .. } => {
            if is_structured_error_result(result_text) {
                if let Some(retry_timing) = structured_error_retry_timing(result_text) {
                    session.write_attempts += 1;
                    if session.write_attempts < MAX_WRITE_ATTEMPTS {
                        let retry_not_before = match retry_timing {
                            RetryTiming::Immediate => Some(None),
                            RetryTiming::After(delay) => now.checked_add(delay).map(Some),
                        };
                        if let Some(retry_not_before) = retry_not_before {
                            // Re-open the same step with a fresh call id.
                            // Delayed recovery remains pending until the
                            // provider-specified deadline.
                            session.write_retry_pending = true;
                            session.retry_not_before = retry_not_before;
                            return;
                        }
                    }
                }
                // The step failed: either every explicitly allowed attempt
                // was exhausted or the observation does not permit an
                // identical replay (`forbidden`, `requires_changed_input`,
                // `not_useful`, or a missing/unparseable recovery
                // constraint). Sticky error evidence, completed advances,
                // and the final verdict is a hard Failure.
                session.structured_error = true;
            }
            // The step completed (success, or a failed step whose sticky
            // evidence was just recorded): clear the per-step attempt
            // state.
            session.write_attempts = 0;
            session.retry_not_before = None;
            session.completed += 1;
        }
    }
}

/// Decide the response for a session whose request has already been
/// processed. Emits exactly one tool call — the next plan step in order,
/// or a re-emission of the pending write step when its last result was a
/// structured error whose observation explicitly allowed an identical
/// replay, with attempts remaining — per response when the previous call's
/// result arrived; a request without a new result never advances and never
/// re-emits the pending call.
fn respond_for_session(
    session: &mut ScriptedSession,
    available_tool_names: &HashSet<String>,
    next_call_id: &str,
    now: Instant,
) -> ScriptedDecision {
    let steps = session.op.key.steps(session.op.size_bytes);
    if session.write_retry_pending {
        if let Some(retry_not_before) = session.retry_not_before
            && now < retry_not_before
        {
            return ScriptedDecision::RetryAfter(retry_not_before.duration_since(now));
        }
        // The pending write step's last result was a structured error with
        // attempts remaining: re-open the same step for another emission
        // with a fresh call id. The completed counter stays put, so a
        // retry never advances the plan or duplicates a step.
        let step = &steps[session.emitted - 1];
        if let Some(wire_name) = resolve_wire_name(available_tool_names, step.capability_id) {
            let Some(arguments) = build_arguments(&session.op, step) else {
                return ScriptedDecision::None;
            };
            session.last_call_id = Some(next_call_id.to_string());
            session.undisclosed_turns = 0;
            session.write_retry_pending = false;
            session.retry_not_before = None;
            return ScriptedDecision::ToolCalls(vec![ToolCallSpec {
                wire_name,
                arguments,
            }]);
        }
        // The tool vanished between the failed attempt and its retry: keep
        // the retry pending (a later request may see it advertised again)
        // and fall through to the undisclosed accounting.
        session.undisclosed_turns += 1;
        if session.undisclosed_turns >= UNDISCLOSED_ATTEMPTS {
            return ScriptedDecision::FinalText(result_text(&session.op, Verdict::Undisclosed));
        }
        return ScriptedDecision::Placeholder;
    }
    if session.completed < session.emitted {
        // The pending call has no result in this request: neither advance
        // nor emit a duplicate call. An interim placeholder keeps the loop
        // moving without duplicating a step.
        return ScriptedDecision::Placeholder;
    }
    if session.emitted < steps.len() {
        let step = &steps[session.emitted];
        if let Some(wire_name) = resolve_wire_name(available_tool_names, step.capability_id) {
            // A planned chunk size always fits the read-back token plus the
            // canonical marker, so this is unreachable in production; fail
            // closed by emitting no scripted call rather than writing
            // content that lacks plan identity.
            let Some(arguments) = build_arguments(&session.op, step) else {
                return ScriptedDecision::None;
            };
            session.last_call_id = Some(next_call_id.to_string());
            session.emitted += 1;
            session.undisclosed_turns = 0;
            session.write_attempts = 0;
            session.write_retry_pending = false;
            session.retry_not_before = None;
            return ScriptedDecision::ToolCalls(vec![ToolCallSpec {
                wire_name,
                arguments,
            }]);
        }
        session.undisclosed_turns += 1;
        if session.undisclosed_turns >= UNDISCLOSED_ATTEMPTS {
            return ScriptedDecision::FinalText(result_text(&session.op, Verdict::Undisclosed));
        }
        return ScriptedDecision::Placeholder;
    }
    // Every step emitted and completed: the verdict comes from the retained
    // checkpoint evidence and the sticky structured-error flag, so a
    // compacted-away failed write or intermediate checkpoint still shapes
    // the final verdict.
    let verdict = if session.structured_error {
        Verdict::Failure
    } else {
        combine_verdicts(session.checkpoint_verdicts.iter().copied())
    };
    ScriptedDecision::FinalText(result_text(&session.op, verdict))
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
#[cfg(test)]
fn find_latest_op(conversation: &Conversation) -> Option<(usize, ScriptedOp)> {
    conversation
        .user_messages
        .iter()
        .filter_map(|(position, text)| parse_marker(text).map(|op| (*position, op)))
        .max_by_key(|(position, _)| *position)
}

/// Whether the conversation already contains the final result text for this
/// operation (prevents re-driving a completed sequence).
#[cfg(test)]
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

/// Whether a tool-result text is a structured error observation. The host
/// renders every capability call as a model-visible observation JSON object
/// whose top-level `status` is `"success"` or `"error"`, serialized
/// compactly into the tool result content, so a failed write, append, or
/// checkpoint carries the literal `"status":"error"` fragment. Whitespace
/// around the colon is tolerated for other producers, and the scan only
/// matches real (unescaped) JSON keys, so an error-shaped string nested
/// inside an observation's quoted preview cannot flip a success verdict.
/// Plain-text error prose is deliberately not treated as structured: the
/// read-back token scan stays the arbiter for unstructured text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryTiming {
    Immediate,
    After(Duration),
}

fn structured_error_observation(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let observation = serde_json::from_str::<Value>(&text[start..=end]).ok()?;
    (observation.get("status").and_then(Value::as_str) == Some("error")).then_some(observation)
}

fn is_structured_error_result(text: &str) -> bool {
    structured_error_observation(text).is_some()
}

/// Parse the recovery contract for an identical write replay. Immediate
/// retries require `allowed`. Delayed retries require both
/// `allowed_after_delay` and an explicit `retry_after_ms`; a missing or
/// malformed delay fails closed rather than being treated as zero.
fn structured_error_retry_timing(text: &str) -> Option<RetryTiming> {
    let observation = structured_error_observation(text)?;
    let recovery = observation.get("recovery")?;
    match recovery.get("same_call_retry").and_then(Value::as_str)? {
        "allowed" => Some(RetryTiming::Immediate),
        "allowed_after_delay" => recovery
            .get("retry_after_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis)
            .map(RetryTiming::After),
        _ => None,
    }
}

/// Derive the read-back verdict from one text per checkpoint step of this
/// operation. Checkpoints are classified independently and the strictest
/// classification wins: a structured error observation on any checkpoint is
/// a hard [`Verdict::Failure`], any foreign-user token is a
/// [`Verdict::Leak`], any checkpoint without a recognized token is
/// [`Verdict::Missing`], any checkpoint carrying a same-user token (with
/// or without this operation's own token) is [`Verdict::Contended`], and
/// [`Verdict::Confirmed`] requires every checkpoint to return exactly this
/// operation's token and no same-user token. An earlier confirming
/// checkpoint therefore cannot mask a lost final durable state or a
/// contested final write.
/// Classify one checkpoint result text on its own. Checkpoints are
/// classified independently and the strictest classification wins across
/// all of them: a structured error observation is a hard
/// [`Verdict::Failure`], any foreign-user token is a [`Verdict::Leak`], a
/// text without a recognized token is [`Verdict::Missing`], a text
/// carrying any same-user token is [`Verdict::Contended`], and a text
/// returning exactly this operation's token with no same-user token is
/// [`Verdict::Confirmed`] (own plus same-user is contended, never
/// confirmed).
pub(crate) fn classify_checkpoint(op: &ScriptedOp, text: &str) -> Verdict {
    if is_structured_error_result(text) {
        return Verdict::Failure;
    }
    let own_token = op.readback_token();
    let mut own_found = false;
    let mut foreign_user_found = false;
    let mut same_user_found = false;
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
    if foreign_user_found {
        Verdict::Leak
    } else if own_found && !same_user_found {
        Verdict::Confirmed
    } else if own_found || same_user_found {
        // Own plus a same-user token is still contention, never confirmed:
        // mixed content proves another same-user write reached the
        // document between this operation's write and read.
        Verdict::Contended
    } else {
        Verdict::Missing
    }
}

/// Combine per-checkpoint verdicts with hard-failure precedence:
/// [`Verdict::Failure`] > [`Verdict::Leak`] > [`Verdict::Missing`] >
/// [`Verdict::Contended`] > [`Verdict::Confirmed`]. An earlier confirming
/// checkpoint therefore cannot mask a lost final durable state or a
/// contested final write. An empty list carries no checkpoint evidence at
/// all and is [`Verdict::Missing`].
pub(crate) fn combine_verdicts(verdicts: impl IntoIterator<Item = Verdict>) -> Verdict {
    let mut combined = None;
    for verdict in verdicts {
        combined = Some(match (combined, verdict) {
            (Some(Verdict::Failure), _) | (_, Verdict::Failure) => Verdict::Failure,
            (Some(Verdict::Leak), _) | (_, Verdict::Leak) => Verdict::Leak,
            (Some(Verdict::Missing), _) | (_, Verdict::Missing) => Verdict::Missing,
            (Some(Verdict::Contended), _) | (_, Verdict::Contended) => Verdict::Contended,
            _ => Verdict::Confirmed,
        });
    }
    combined.unwrap_or(Verdict::Missing)
}

/// Derive the read-back verdict from one text per checkpoint step of this
/// operation. Checkpoints are classified independently and the strictest
/// classification wins: a structured error observation on any checkpoint is
/// a hard [`Verdict::Failure`], any foreign-user token is a
/// [`Verdict::Leak`], any checkpoint without a recognized token is
/// [`Verdict::Missing`], any checkpoint carrying a same-user token (with
/// or without this operation's own token) is [`Verdict::Contended`], and
/// [`Verdict::Confirmed`] requires every checkpoint to return exactly this
/// operation's token and no same-user token. An earlier confirming
/// checkpoint therefore cannot mask a lost final durable state or a
/// contested final write.
#[cfg(test)]
pub(crate) fn compute_verdict(op: &ScriptedOp, tool_result_texts: &[&str]) -> Verdict {
    combine_verdicts(
        tool_result_texts
            .iter()
            .map(|text| classify_checkpoint(op, text)),
    )
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

/// Build the arguments for a script step from the operation marker. Memory
/// write steps carry their chunk size in the generated plan, so the
/// content is exact without cumulative fraction math, and every write
/// embeds the step's plan ordinal so repeated full-size chunks never share
/// a call signature. Write steps return `None` (fail closed) only when the
/// chunk size cannot hold both the read-back token and the canonical
/// marker — impossible for parseable markers and planned chunk sizes, so
/// callers never panic on it.
pub(crate) fn build_arguments(op: &ScriptedOp, step: &ScriptStep) -> Option<Value> {
    match step.kind {
        StepKind::WriteFile => {
            let path = format!("stress/{}.txt", sanitize_path_segment(&op.identity()));
            let content = scripted_content(op, op.size_bytes, step.step_index)?;
            Some(serde_json::json!({ "path": path, "content": content }))
        }
        StepKind::ReadFile => {
            let path = format!("stress/{}.txt", sanitize_path_segment(&op.identity()));
            Some(serde_json::json!({ "path": path }))
        }
        StepKind::MemoryWrite { append, size_bytes } => {
            let content = scripted_content(op, size_bytes, step.step_index)?;
            Some(serde_json::json!({
                "target": SHARED_MEMORY_TARGET,
                "content": content,
                "append": append,
            }))
        }
        StepKind::MemoryRead => Some(serde_json::json!({ "path": SHARED_MEMORY_TARGET })),
        StepKind::MemorySearch => Some(serde_json::json!({
            "query": MEMORY_SEARCH_QUERY,
            "limit": MEMORY_SEARCH_LIMIT,
        })),
    }
}

/// Length of a [`step_ordinal_token`], in bytes. Fixed width keeps the
/// content overhead identical for every step of a plan, so
/// [`scripted_write_min_bytes`] bounds every chunk without knowing the
/// step index. The four-digit field covers plans up to 9999 steps; the
/// largest scripted plan (an 8 MiB mixed document) is about 140 steps.
const STEP_ORDINAL_TOKEN_LEN: usize = 5;

/// Deterministic ordinal token embedded in scripted write content: `s`
/// plus the zero-padded plan-step index. The token is ASCII-safe (so the
/// content stays JSON-safe), fixed-width (so the overhead is
/// index-independent), and distinct for every step of a plan, which makes
/// consecutive full-size chunks — identical in capability, chunk size,
/// read-back token, and marker — still differ in their serialized
/// arguments.
fn step_ordinal_token(step_index: usize) -> String {
    format!("s{step_index:04}")
}

/// Minimum content bytes for a scripted write chunk of `op`: the read-back
/// token, the ordinal token, the canonical marker, and the three
/// separating spaces.
pub(crate) fn scripted_write_min_bytes(op: &ScriptedOp) -> usize {
    op.readback_token().len()
        + STEP_ORDINAL_TOKEN_LEN
        + marker_message(op.key, &op.user, &op.op, op.size_bytes).len()
        + 3
}

/// Build deterministic write content of exactly `size_bytes` carrying the
/// operation's read-back token, the plan-step ordinal of `step_index`,
/// and the full canonical marker (`ironclaw-stress-tool <script>
/// <user>__<op> <size>`, identical to the driver's original user marker).
/// The token and marker let a later write call's serialized arguments
/// still identify the plan after the original user message is compacted
/// out of a long sequential plan; the ordinal makes every chunk of the
/// plan unique, so the production no-progress guard never sees repeated
/// full-size append calls with identical arguments.
///
/// Returns `None` when `size_bytes` cannot hold the token, the ordinal,
/// the marker, and the three separating spaces. Production chunk plans
/// never produce such a size — the smallest planned chunk, a 1024-byte
/// quarter of a 4 KiB document, is far above the ~340-byte bound for the
/// longest parseable identities — so `None` is unreachable through valid
/// markers and the caller fails closed instead of emitting content
/// without identity.
pub(crate) fn scripted_content(
    op: &ScriptedOp,
    size_bytes: usize,
    step_index: usize,
) -> Option<String> {
    let token = op.readback_token();
    let ordinal = step_ordinal_token(step_index);
    let marker = marker_message(op.key, &op.user, &op.op, op.size_bytes);
    // The ordinal token is fixed-width, so the minimum write size is
    // exactly this step's overhead and stays the single size arbiter.
    let overhead = scripted_write_min_bytes(op);
    if size_bytes < overhead {
        return None;
    }
    let padding_len = size_bytes - overhead;
    let mut content = format!("{token} {ordinal} {marker} ");
    content.push_str(&"x".repeat(padding_len));
    Some(content)
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

    /// The agent loop's production default iteration backstop
    /// (`ironclaw_agent_loop::strategies::DEFAULT_ITERATION_BACKSTOP`).
    /// Every scripted plan must complete with one tool call per response
    /// inside this ceiling.
    const PRODUCTION_ITERATION_BACKSTOP: usize = 1024;

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

    /// Extract the canonical marker embedded in a serialized tool-call
    /// argument text and parse it. Scripted write content is
    /// `<token> <marker> <padding>`; the token, marker, and padding
    /// characters are all JSON-safe (ASCII alphanumerics, spaces, `_`,
    /// `-`, `.`), so the marker appears literally in the serialized
    /// arguments and needs no unescaping. Returns `None` when the text
    /// carries no marker. Delegates to the driver's module-level helper
    /// used for compaction recovery.
    fn parse_marker_in_arguments(text: &str) -> Option<ScriptedOp> {
        super::parse_marker_in_arguments(text)
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                let spec = &calls[0];
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                assert_eq!(calls[0].wire_name, "ironclaw__memory__read");
                assert_eq!(calls[0].arguments["path"], SHARED_MEMORY_TARGET);
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
        // Own token alone confirms; own plus a same-user token is
        // contention, never confirmed — mixed content proves another
        // same-user write reached the document.
        let only_own = format!("Tool returned: {own}");
        assert_eq!(compute_verdict(&parsed, &[&only_own]), Verdict::Confirmed);
        let text = format!("Tool returned: {own} {same_user}");
        assert_eq!(compute_verdict(&parsed, &[&text]), Verdict::Contended);
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
    fn structured_error_observation_detected() {
        let success = r#"{"schema_version":1,"status":"success","summary":"Tool completed","detail":{"kind":"result_reference","result_ref":"result:r","byte_len":5,"preview":"abcde","total_bytes":5,"next_offset":null,"item_count":null},"artifacts":[{"artifact_ref":"result:r","summary":"Stored tool result"}],"recovery":null,"trust":"untrusted_tool_output"}"#;
        assert!(!is_structured_error_result(success));
        let error = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        assert!(is_structured_error_result(error));
        // Whitespace around the colon is tolerated (other producers may not
        // serialize compactly).
        assert!(is_structured_error_result(r#"{"status": "error"}"#));
        // Plain error prose is not a structured observation.
        assert!(!is_structured_error_result(
            "Tool ironclaw.memory.write returned: Error: disk full"
        ));
        // A `status` inside a JSON-escaped string (e.g. tool output preview
        // quoted inside the observation) is not the observation's own status.
        let escaped_nested = r#"{"status":"success","detail":{"kind":"result_reference","result_ref":"result:r","preview":"{\"status\":\"error\"}"}}"#;
        assert!(!is_structured_error_result(escaped_nested));
    }

    #[test]
    fn mixed_checkpoints_classified_independently() {
        let parsed = op(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let own = parsed.readback_token();
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let foreign = format!("{READBACK_MARKER}_u1__3");
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        // Every checkpoint confirming is the only path to Confirmed.
        assert_eq!(
            compute_verdict(&parsed, &[own.as_str(), own.as_str()]),
            Verdict::Confirmed
        );
        // An earlier confirming checkpoint must not mask a tokenless final
        // checkpoint: the final durable state is lost.
        assert_eq!(
            compute_verdict(&parsed, &[own.as_str(), ""]),
            Verdict::Missing
        );
        assert_eq!(
            compute_verdict(&parsed, &["", own.as_str()]),
            Verdict::Missing
        );
        // A checkpoint carrying only a same-user token is Contended, even
        // when an earlier checkpoint confirmed.
        assert_eq!(
            compute_verdict(&parsed, &[own.as_str(), same_user.as_str()]),
            Verdict::Contended
        );
        assert_eq!(
            compute_verdict(&parsed, &[same_user.as_str(), own.as_str()]),
            Verdict::Contended
        );
        // A leak in any checkpoint wins over own tokens elsewhere.
        assert_eq!(
            compute_verdict(&parsed, &[own.as_str(), foreign.as_str()]),
            Verdict::Leak
        );
        // A structured error observation on any checkpoint is a hard failure.
        assert_eq!(
            compute_verdict(&parsed, &[own.as_str(), error_obs]),
            Verdict::Failure
        );
        assert_eq!(
            compute_verdict(&parsed, &[error_obs, own.as_str()]),
            Verdict::Failure
        );
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "file plans keep one call per response");
                assert_eq!(calls[0].wire_name, "builtin__write_file");
                assert_eq!(calls[0].arguments["path"], "stress/u2__7.txt");
                assert_eq!(calls[0].arguments["content"].as_str().unwrap().len(), 8192);
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "file plans keep one call per response");
                assert_eq!(calls[0].wire_name, "builtin__read_file");
                assert_eq!(calls[0].arguments["path"], "stress/u2__7.txt");
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                assert_eq!(calls[0].arguments["append"], false);
                assert_eq!(calls[0].arguments["content"].as_str().unwrap().len(), 1024);
            }
            other => panic!("expected initial write, got {other:?}"),
        }
        let after_write = vec![user(&marker), tool_result("write ok")];
        match decide(
            &after_write,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                assert_eq!(calls[0].arguments["append"], true);
                assert_eq!(calls[0].arguments["content"].as_str().unwrap().len(), 3072);
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
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                assert_eq!(calls[0].arguments["path"], SHARED_MEMORY_TARGET);
            }
            other => panic!("expected read, got {other:?}"),
        }
    }

    #[test]
    fn memory_mixed_has_four_steps_with_half_sizes() {
        assert_eq!(ScriptKey::MemoryMixed.expected_tool_results(32768), 4);
        let marker = marker_message(ScriptKey::MemoryMixed, "u1", "2", 32768);
        let messages = vec![user(&marker)];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "short plans keep one call per response");
                assert_eq!(calls[0].arguments["append"], false);
                assert_eq!(calls[0].arguments["content"].as_str().unwrap().len(), 16384);
            }
            other => panic!("expected first write, got {other:?}"),
        }
    }

    #[test]
    fn split_writes_preserve_exact_configured_size() {
        // 4097 is not divisible by 4. Phase boundaries derived from the
        // configured fractions must still persist exactly the configured
        // size across the split writes.
        for (key, expected_chunks) in [
            (ScriptKey::MemoryGrow, vec![1024, 3073]),
            (ScriptKey::MemoryMixed, vec![2048, 2049]),
        ] {
            let parsed = op(key, "u0", "1", 4097);
            let mut chunks = Vec::new();
            for step in key.steps(4097) {
                if matches!(step.kind, StepKind::MemoryWrite { .. }) {
                    let arguments = build_arguments(&parsed, &step).expect("planned chunk fits");
                    chunks.push(arguments["content"].as_str().expect("string").len());
                }
            }
            assert_eq!(chunks, expected_chunks, "chunks for {key:?}");
            assert_eq!(chunks.iter().sum::<usize>(), 4097, "total for {key:?}");
        }
    }

    #[test]
    fn expected_tool_results_per_script() {
        // Small documents fit one chunk per phase, so the counts match the
        // fixed plans: 2, 2, 3, and 4 tool results.
        assert_eq!(ScriptKey::WriteFileRoundtrip.expected_tool_results(4096), 2);
        assert_eq!(ScriptKey::MemoryRoundtrip.expected_tool_results(4096), 2);
        assert_eq!(ScriptKey::MemoryGrow.expected_tool_results(4096), 3);
        assert_eq!(ScriptKey::MemoryMixed.expected_tool_results(4096), 4);
    }

    #[test]
    fn one_mib_memory_plans_are_bounded_and_exact() {
        const MIB: usize = 1024 * 1024;
        for (key, phase_shares, expected_reads) in [
            (ScriptKey::MemoryRoundtrip, vec![MIB], 1),
            (ScriptKey::MemoryGrow, vec![MIB / 4, MIB - MIB / 4], 1),
            (ScriptKey::MemoryMixed, vec![MIB / 2, MIB - MIB / 2], 2),
        ] {
            let parsed = op(key, "u0", "1", MIB);
            let steps = key.steps(MIB);
            assert_eq!(key.expected_tool_results(MIB), steps.len(), "{key:?}");
            let mut writes = Vec::new();
            let mut checkpoints = 0usize;
            for step in &steps {
                match step.kind {
                    StepKind::MemoryWrite { append, size_bytes } => {
                        assert_eq!(step.capability_id, "ironclaw.memory.write", "{key:?}");
                        assert!(
                            size_bytes <= MAX_MEMORY_WRITE_CHUNK_BYTES,
                            "{key:?} chunk of {size_bytes} bytes exceeds the per-call ceiling"
                        );
                        assert!(
                            size_bytes >= scripted_write_min_bytes(&parsed),
                            "{key:?} chunk of {size_bytes} bytes is shorter than its token-plus-marker payload"
                        );
                        let arguments = build_arguments(&parsed, step).expect("planned chunk fits");
                        let content = arguments["content"].as_str().expect("string");
                        assert_eq!(content.len(), size_bytes, "{key:?} content must be exact");
                        assert!(
                            content.contains(&parsed.readback_token()),
                            "{key:?} content must carry the read-back token"
                        );
                        assert!(
                            content.contains(MARKER_PREFIX),
                            "{key:?} content must carry the canonical marker"
                        );
                        assert!(
                            content.contains(&step_ordinal_token(step.step_index)),
                            "{key:?} content must carry its plan-step ordinal"
                        );
                        writes.push((append, size_bytes));
                    }
                    StepKind::MemorySearch => {
                        // A 1 MiB document plus its JSON envelope exceeds
                        // the output cap, so every checkpoint must be a
                        // bounded search, never a full read.
                        assert_eq!(step.capability_id, "ironclaw.memory.search", "{key:?}");
                        let arguments = build_arguments(&parsed, step).expect("search args build");
                        assert_eq!(arguments["query"], MEMORY_SEARCH_QUERY, "{key:?}");
                        assert_eq!(arguments["limit"], MEMORY_SEARCH_LIMIT, "{key:?}");
                        checkpoints += 1;
                    }
                    StepKind::MemoryRead | StepKind::WriteFile | StepKind::ReadFile => {
                        panic!(
                            "{key:?} at 1 MiB must use search checkpoints, got {:?}",
                            step.kind
                        )
                    }
                }
            }
            assert_eq!(checkpoints, expected_reads, "{key:?} checkpoints");
            assert_eq!(
                writes.iter().map(|(_, size)| *size).sum::<usize>(),
                MIB,
                "{key:?} total persisted bytes"
            );
            assert_eq!(
                writes.first().map(|(append, _)| *append),
                Some(false),
                "{key:?} first write must replace"
            );
            assert!(
                writes.iter().skip(1).all(|(append, _)| *append),
                "{key:?} later writes must append"
            );
            // Each phase persists exactly its configured share: the writes
            // before the intermediate read (mixed) or the first
            // `ceil(share / ceiling)` chunks (grow, roundtrip) add up to
            // the phase share.
            let first_phase_written: usize = match key {
                ScriptKey::MemoryMixed => {
                    let first_checkpoint = steps
                        .iter()
                        .position(|step| {
                            matches!(step.kind, StepKind::MemoryRead | StepKind::MemorySearch)
                        })
                        .expect("mixed plan has an intermediate checkpoint");
                    writes
                        .iter()
                        .take(first_checkpoint)
                        .map(|(_, size)| *size)
                        .sum()
                }
                _ => {
                    let phase_chunks = phase_shares[0].div_ceil(MAX_MEMORY_WRITE_CHUNK_BYTES);
                    writes
                        .iter()
                        .take(phase_chunks)
                        .map(|(_, size)| *size)
                        .sum()
                }
            };
            assert_eq!(
                first_phase_written, phase_shares[0],
                "{key:?} first phase bytes"
            );
        }
    }

    /// Drive a scripted plan to completion through `decide`, feeding one
    /// tool result back per emitted call (read steps echo the operation's
    /// read-back token so the final verdict is `confirmed`). Every call
    /// must advance the plan by exactly one step in order and keep its
    /// content within the per-call ceiling, and every response must emit
    /// exactly one call. Returns the per-round call counts.
    fn drive_scripted_plan(key: ScriptKey, user_id: &str, op_id: &str, size: usize) -> Vec<usize> {
        let parsed = op(key, user_id, op_id, size);
        let marker = marker_message(key, user_id, op_id, size);
        let available = tools(&[
            "builtin__write_file",
            "builtin__read_file",
            "ironclaw__memory__write",
            "ironclaw__memory__read",
            "ironclaw__memory__search",
        ]);
        let steps = key.steps(size);
        let mut messages = vec![user(&marker)];
        let mut step_index = 0usize;
        let mut calls_per_round = Vec::new();
        loop {
            match decide(&messages, &available) {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(
                        calls.len(),
                        1,
                        "every response emits exactly one call, got {}",
                        calls.len()
                    );
                    calls_per_round.push(calls.len());
                    for (offset, spec) in calls.iter().enumerate() {
                        let step = &steps[step_index + offset];
                        let expected_wire = resolve_wire_name(&available, step.capability_id)
                            .expect("plan step tool is advertised");
                        assert_eq!(
                            spec.wire_name,
                            expected_wire,
                            "round {} call {offset}",
                            calls_per_round.len()
                        );
                        if let Some(content) = spec.arguments["content"].as_str() {
                            assert!(
                                content.len() <= MAX_MEMORY_WRITE_CHUNK_BYTES,
                                "round {} call {offset} content of {} bytes exceeds the per-call ceiling",
                                calls_per_round.len(),
                                content.len()
                            );
                        }
                        let echoed = match step.kind {
                            StepKind::MemoryRead | StepKind::MemorySearch | StepKind::ReadFile => {
                                parsed.readback_token()
                            }
                            StepKind::MemoryWrite { .. } | StepKind::WriteFile => "ok".to_string(),
                        };
                        messages.push(tool_result(&format!(
                            "Tool {} returned: {echoed}",
                            spec.wire_name
                        )));
                    }
                    step_index += calls.len();
                }
                ScriptedDecision::FinalText(text) => {
                    assert_eq!(
                        text,
                        format!("{RESULT_PREFIX} {} confirmed", parsed.identity()),
                        "final verdict must confirm the read-back"
                    );
                    break;
                }
                other => panic!("unexpected decision {other:?}"),
            }
        }
        assert_eq!(
            step_index,
            steps.len(),
            "the plan must be exhausted before the verdict"
        );
        calls_per_round
    }

    #[test]
    fn one_mib_memory_roundtrip_emits_one_call_per_response() {
        const MIB: usize = 1024 * 1024;
        // 18 write chunks + 1 search checkpoint = 19 steps, each emitted
        // alone: 19 tool-call responses (one call each) before the final
        // verdict, and the search checkpoint follows every write.
        let calls_per_round = drive_scripted_plan(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        assert_eq!(calls_per_round, vec![1; 19]);
    }

    #[test]
    fn one_mib_memory_mixed_emits_one_call_per_response() {
        const MIB: usize = 1024 * 1024;
        // 9 + 9 write chunks and 2 search checkpoints = 20 steps, each
        // emitted alone: 20 tool-call responses (one call each) before the
        // verdict.
        let calls_per_round = drive_scripted_plan(ScriptKey::MemoryMixed, "u1", "2", MIB);
        assert_eq!(calls_per_round, vec![1; 20]);
    }

    #[test]
    fn one_mib_plans_fit_production_iteration_backstop() {
        // The agent loop's default iteration backstop is 1024 (see
        // `ironclaw_agent_loop::strategies::DEFAULT_ITERATION_BACKSTOP`).
        // With one tool call per response, every scripted plan must
        // complete well inside it, so no plan ever needs batching.
        const MIB: usize = 1024 * 1024;
        for key in [
            ScriptKey::MemoryRoundtrip,
            ScriptKey::MemoryGrow,
            ScriptKey::MemoryMixed,
        ] {
            let step_count = key.steps(MIB).len();
            assert!(
                step_count < PRODUCTION_ITERATION_BACKSTOP,
                "{key:?} plan of {step_count} steps must fit the 1024-iteration backstop"
            );
        }
    }

    #[test]
    fn memory_checkpoint_step_switches_at_inline_safe_ceiling() {
        // At or below the ceiling the checkpoint stays a plain read.
        let at_ceiling = memory_checkpoint_step(INLINE_SAFE_MEMORY_READ_BYTES);
        assert!(matches!(at_ceiling.kind, StepKind::MemoryRead));
        assert_eq!(at_ceiling.capability_id, "ironclaw.memory.read");
        // Above the ceiling the checkpoint becomes a bounded search.
        let above_ceiling = memory_checkpoint_step(INLINE_SAFE_MEMORY_READ_BYTES + 1);
        assert!(matches!(above_ceiling.kind, StepKind::MemorySearch));
        assert_eq!(above_ceiling.capability_id, "ironclaw.memory.search");
        // Exactly 1 MiB exceeds the inline-safe ceiling.
        assert!(matches!(
            memory_checkpoint_step(1024 * 1024).kind,
            StepKind::MemorySearch
        ));
        // Small and 128 KiB documents keep the read shape.
        assert!(matches!(
            memory_checkpoint_step(MIN_SCRIPTED_DOC_SIZE_BYTES).kind,
            StepKind::MemoryRead
        ));
        assert!(matches!(
            memory_checkpoint_step(128 * 1024).kind,
            StepKind::MemoryRead
        ));
    }

    #[test]
    fn sub_ceiling_plans_keep_memory_read_checkpoints() {
        const SIZE: usize = 128 * 1024;
        for key in [
            ScriptKey::MemoryRoundtrip,
            ScriptKey::MemoryGrow,
            ScriptKey::MemoryMixed,
        ] {
            let steps = key.steps(SIZE);
            for step in &steps {
                if matches!(step.kind, StepKind::MemoryRead | StepKind::MemorySearch) {
                    assert!(
                        matches!(step.kind, StepKind::MemoryRead),
                        "{key:?} at {SIZE} bytes must keep memory.read checkpoints"
                    );
                    assert_eq!(step.capability_id, "ironclaw.memory.read", "{key:?}");
                }
            }
        }
    }

    #[test]
    fn one_mib_roundtrip_checkpoint_emits_search_call() {
        const MIB: usize = 1024 * 1024;
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let mut messages = vec![user(&marker)];
        for _ in 0..18 {
            messages.push(tool_result("Tool ironclaw.memory.write returned: ok"));
        }
        match decide(
            &messages,
            &tools(&[
                "ironclaw__memory__write",
                "ironclaw__memory__read",
                "ironclaw__memory__search",
            ]),
        ) {
            ScriptedDecision::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1, "one checkpoint remains after the writes");
                assert_eq!(calls[0].wire_name, "ironclaw__memory__search");
                assert_eq!(calls[0].arguments["query"], MEMORY_SEARCH_QUERY);
                assert_eq!(calls[0].arguments["limit"], MEMORY_SEARCH_LIMIT);
            }
            other => panic!("expected the search checkpoint call, got {other:?}"),
        }
    }

    /// Drive a 1 MiB memory roundtrip through `decide` with all 18 write
    /// tool results and one search tool result, returning the verdict the
    /// sidecar derives from the search output.
    fn one_mib_search_verdict(user_id: &str, op_id: &str, search_text: &str) -> Verdict {
        const MIB: usize = 1024 * 1024;
        let parsed = op(ScriptKey::MemoryRoundtrip, user_id, op_id, MIB);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, user_id, op_id, MIB);
        let mut messages = vec![user(&marker)];
        for _ in 0..18 {
            messages.push(tool_result("Tool ironclaw.memory.write returned: ok"));
        }
        messages.push(tool_result(&format!(
            "Tool ironclaw.memory.search returned: {search_text}"
        )));
        match decide(
            &messages,
            &tools(&[
                "ironclaw__memory__write",
                "ironclaw__memory__read",
                "ironclaw__memory__search",
            ]),
        ) {
            ScriptedDecision::FinalText(text) => {
                parse_result_verdict(&text, &parsed).expect("final text carries a verdict")
            }
            other => panic!("expected final verdict, got {other:?}"),
        }
    }

    #[test]
    fn search_output_tokens_drive_readback_verdicts() {
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 1024 * 1024);
        let own = parsed.readback_token();
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let foreign = format!("{READBACK_MARKER}_u1__3");
        // Own token in the search snippets confirms the write.
        assert_eq!(one_mib_search_verdict("u0", "1", &own), Verdict::Confirmed);
        // Only another same-user operation's token: hot-document contention.
        assert_eq!(
            one_mib_search_verdict("u0", "1", &same_user),
            Verdict::Contended
        );
        // A foreign user's token in the search output is an isolation leak.
        assert_eq!(one_mib_search_verdict("u0", "1", &foreign), Verdict::Leak);
        // No tokens at all: the write was lost.
        assert_eq!(
            one_mib_search_verdict("u0", "1", "no matches"),
            Verdict::Missing
        );
        // Own plus a same-user token in one result set: mixed content is
        // contention, never confirmed; a foreign token still leaks.
        assert_eq!(
            one_mib_search_verdict("u0", "1", &format!("{own} {same_user}")),
            Verdict::Contended
        );
        assert_eq!(
            one_mib_search_verdict("u0", "1", &format!("{own} {foreign}")),
            Verdict::Leak
        );
    }

    #[test]
    fn bounded_excerpt_shape_with_contender_past_8192_is_contended() {
        // Phase 1B false-confirmed regression: a 1 MiB hot document carries
        // this operation's token at its head and a same-user contender's
        // token only past the first 8192 bytes. The native provider's
        // bounded search preview returns excerpts around every exact-literal
        // query occurrence (own at the head, contender in later excerpts,
        // joined by an ellipsis delimiter, all under the snippet cap), so
        // the classifier must see own + same-user and report `contended` —
        // never the head-only `confirmed` a naive head cut would produce.
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 1024 * 1024);
        let own = parsed.readback_token();
        let contender_past_cap = format!("{READBACK_MARKER}_u0__2");
        let contender_tail = format!("{READBACK_MARKER}_u0__3");
        // The provider's excerpt shape: bounded windows around each
        // exact-literal occurrence, joined by the ellipsis delimiter.
        let excerpt_own = format!("… {own} s0001 ironclaw-stress-tool …");
        let excerpt_contender = format!("… {contender_past_cap} s0007 ironclaw-stress-tool …");
        let excerpt_tail = format!("… {contender_tail} s0012 ironclaw-stress-tool …");
        let shape = format!("{excerpt_own}\n…\n{excerpt_contender}\n…\n{excerpt_tail}");
        assert!(
            shape.len() <= 8192,
            "shape must stay within the snippet cap, got {} bytes",
            shape.len()
        );
        assert_eq!(
            one_mib_search_verdict("u0", "1", &shape),
            Verdict::Contended
        );
        // Counter-scenario: the own-only shape (what the old head cut
        // returned, and what a contender-free document yields) confirms.
        assert_eq!(
            one_mib_search_verdict("u0", "1", &excerpt_own),
            Verdict::Confirmed
        );
    }

    #[test]
    fn mixed_first_checkpoint_own_final_empty_is_missing() {
        let parsed = op(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let own = parsed.readback_token();
        let marker = marker_message(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: wrote first half"),
            tool_result(&format!("Tool ironclaw.memory.read returned: {own}")),
            tool_result("Tool ironclaw.memory.write returned: appended second half"),
            tool_result("Tool ironclaw.memory.read returned: (empty)"),
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
    fn mixed_first_checkpoint_own_final_same_user_only_is_contended() {
        let parsed = op(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let own = parsed.readback_token();
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let marker = marker_message(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: wrote first half"),
            tool_result(&format!("Tool ironclaw.memory.read returned: {own}")),
            tool_result("Tool ironclaw.memory.write returned: appended second half"),
            tool_result(&format!("Tool ironclaw.memory.read returned: {same_user}")),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 contended");
            }
            other => panic!("expected contended verdict, got {other:?}"),
        }
    }

    #[test]
    fn failed_append_with_own_checkpoint_is_failure() {
        let parsed = op(ScriptKey::MemoryGrow, "u0", "1", 4096);
        let own = parsed.readback_token();
        let marker = marker_message(ScriptKey::MemoryGrow, "u0", "1", 4096);
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: wrote quarter"),
            tool_result(&format!("Tool ironclaw.memory.write returned: {error_obs}")),
            tool_result(&format!("Tool ironclaw.memory.read returned: {own}")),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 failure");
            }
            other => panic!("expected failure verdict, got {other:?}"),
        }
    }

    #[test]
    fn failed_checkpoint_result_is_failure_not_missing() {
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        let messages = vec![
            user(&marker),
            tool_result("Tool ironclaw.memory.write returned: wrote ok"),
            tool_result(&format!("Tool ironclaw.memory.read returned: {error_obs}")),
        ];
        match decide(
            &messages,
            &tools(&["ironclaw__memory__write", "ironclaw__memory__read"]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 failure");
            }
            other => panic!("expected failure verdict, got {other:?}"),
        }
    }

    #[test]
    fn failed_mid_plan_append_with_own_search_is_failure() {
        // A 1 MiB roundtrip emits one call per response. The tenth write
        // fails with a structured error observation, and the final search
        // still returns the operation's own token. The failed append must
        // not be masked by the own token in the search output: hard
        // Failure.
        const MIB: usize = 1024 * 1024;
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let own = parsed.readback_token();
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        let mut messages = vec![user(&marker)];
        for index in 0..18 {
            let text = if index == 9 {
                error_obs
            } else {
                "Tool ironclaw.memory.write returned: ok"
            };
            messages.push(tool_result(text));
        }
        messages.push(tool_result(&format!(
            "Tool ironclaw.memory.search returned: {own}"
        )));
        match decide(
            &messages,
            &tools(&[
                "ironclaw__memory__write",
                "ironclaw__memory__read",
                "ironclaw__memory__search",
            ]),
        ) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 failure");
            }
            other => panic!("expected failure verdict, got {other:?}"),
        }
    }

    #[test]
    fn small_and_file_plans_emit_one_call_per_response() {
        for (key, size) in [
            (ScriptKey::WriteFileRoundtrip, 4096),
            (ScriptKey::MemoryRoundtrip, 4096),
            (ScriptKey::MemoryGrow, 4096),
            (ScriptKey::MemoryMixed, 4096),
        ] {
            let calls_per_round = drive_scripted_plan(key, "u0", "1", size);
            assert_eq!(
                calls_per_round,
                vec![1; key.steps(size).len()],
                "{key:?} must stay one call per response"
            );
        }
    }

    #[test]
    fn long_plan_halts_before_unavailable_step_then_undisclosed() {
        // 8 write chunks + 1 read = 9 steps; only the write tool is
        // disclosed. Each response emits exactly the next write (never
        // skipping one), halts exactly at the unavailable read, then
        // follows the placeholder / undisclosed path.
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 450_000);
        let write_only = tools(&["ironclaw__memory__write"]);
        let mut messages = vec![user(&marker)];
        let mut calls_per_round = Vec::new();
        for _ in 0..8 {
            match decide(&messages, &write_only) {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(calls.len(), 1, "exactly one call per response");
                    calls_per_round.push(calls.len());
                    assert_eq!(calls[0].wire_name, "ironclaw__memory__write");
                    messages.push(tool_result(&format!(
                        "Tool {} returned: ok",
                        calls[0].wire_name
                    )));
                }
                other => panic!("expected the next write call, got {other:?}"),
            }
        }
        assert_eq!(
            calls_per_round,
            vec![1; 8],
            "the eight writes are emitted one per response"
        );
        // The read is still not disclosed: placeholder, then undisclosed.
        assert_eq!(
            decide(&messages, &write_only),
            ScriptedDecision::Placeholder
        );
        messages.push(assistant("I'll perform the stress tool action next."));
        assert_eq!(
            decide(&messages, &write_only),
            ScriptedDecision::Placeholder
        );
        messages.push(assistant("I'll perform the stress tool action next."));
        match decide(&messages, &write_only) {
            ScriptedDecision::FinalText(text) => {
                assert_eq!(text, "ironclaw-stress-tool result u0__1 undisclosed");
            }
            other => panic!("expected undisclosed final, got {other:?}"),
        }
    }

    #[test]
    fn oversized_odd_memory_plan_has_no_tiny_remainder() {
        // 1 MiB + 3: no phase divides evenly. Every chunk must stay within
        // the per-call ceiling, no chunk may be shorter than the read-back
        // token, and the writes must persist exactly the configured size.
        let size = 1024 * 1024 + 3;
        let parsed = op(ScriptKey::MemoryGrow, "u0", "1", size);
        let mut written = 0usize;
        let mut seen_replace = false;
        for step in ScriptKey::MemoryGrow.steps(size) {
            if let StepKind::MemoryWrite { append, size_bytes } = step.kind {
                assert!(size_bytes <= MAX_MEMORY_WRITE_CHUNK_BYTES);
                assert!(size_bytes >= scripted_write_min_bytes(&parsed));
                let arguments = build_arguments(&parsed, &step).expect("planned chunk fits");
                let content = arguments["content"].as_str().expect("string");
                assert_eq!(content.len(), size_bytes);
                assert!(content.contains(&parsed.readback_token()));
                assert!(content.contains(MARKER_PREFIX));
                assert!(content.contains(&step_ordinal_token(step.step_index)));
                if seen_replace {
                    assert!(append, "all writes after the first must append");
                } else {
                    assert!(!append, "the first write must replace");
                    seen_replace = true;
                }
                written += size_bytes;
            }
        }
        assert_eq!(written, size);
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
    fn write_content_is_exact_and_carries_token_plus_marker() {
        let parsed = op(ScriptKey::MemoryGrow, "u4", "9", 4096);
        let marker = marker_message(parsed.key, &parsed.user, &parsed.op, parsed.size_bytes);
        let content = scripted_content(&parsed, 1000, 7).expect("1000 bytes fit token and marker");
        assert_eq!(content.len(), 1000);
        assert!(content.starts_with(&parsed.readback_token()));
        assert!(content.contains(&marker));
        assert!(content.contains(&step_ordinal_token(7)));
        // Deterministic: identical input yields identical content.
        assert_eq!(scripted_content(&parsed, 1000, 7), Some(content.clone()));
        // A different plan step yields a different ordinal, so equal-size
        // chunks never share content.
        assert_ne!(scripted_content(&parsed, 1000, 8), Some(content));
        // The smallest chunk that fits is exactly the
        // token+ordinal+marker overhead.
        let min = scripted_write_min_bytes(&parsed);
        let minimal = scripted_content(&parsed, min, 7).expect("exact overhead fits");
        assert_eq!(minimal.len(), min);
        assert!(minimal.contains(READBACK_MARKER));
        assert!(minimal.contains(MARKER_PREFIX));
        assert!(minimal.contains(&step_ordinal_token(7)));
        // Any smaller size cannot carry all three identities: fail closed
        // with `None`, never a truncated or identity-less chunk.
        assert_eq!(scripted_content(&parsed, min - 1, 7), None);
        assert_eq!(scripted_content(&parsed, 4, 7), None);
        assert_eq!(scripted_content(&parsed, 0, 7), None);
    }

    #[test]
    fn parse_marker_recovers_op_from_serialized_write_arguments() {
        let parsed = op(ScriptKey::MemoryMixed, "u7", "h2-3", 32768);
        let step = ScriptStep {
            capability_id: "ironclaw.memory.write",
            kind: StepKind::MemoryWrite {
                append: true,
                size_bytes: 16384,
            },
            step_index: 1,
        };
        let arguments = build_arguments(&parsed, &step).expect("planned chunk fits");
        let serialized = serde_json::to_string(&arguments).expect("serializes");
        assert!(serialized.contains(&marker_message(
            parsed.key,
            &parsed.user,
            &parsed.op,
            parsed.size_bytes
        )));
        assert_eq!(
            parse_marker_in_arguments(&serialized),
            Some(parsed.clone()),
            "the canonical marker inside a later write call's arguments \
             recovers the full plan identity"
        );
        assert_eq!(parse_marker_in_arguments("no marker here"), None);
        assert_eq!(parse_marker_in_arguments(&parsed.readback_token()), None);
    }

    #[test]
    fn four_kib_and_fraction_chunks_fit_token_and_marker() {
        let parsed = op(ScriptKey::MemoryGrow, "u0", "1", 4096);
        assert!(
            scripted_write_min_bytes(&parsed) <= 1024,
            "even the smallest planned chunk (a 1024-byte quarter of 4 KiB) \
             must fit token plus marker"
        );
        for size in [1024, 2048, 3072, 4096, MAX_MEMORY_WRITE_CHUNK_BYTES] {
            let content = scripted_content(&parsed, size, 0).expect("chunk fits both");
            assert_eq!(content.len(), size, "chunk of {size} bytes must stay exact");
            assert!(content.contains(READBACK_MARKER), "{size}: token present");
            assert!(content.contains(MARKER_PREFIX), "{size}: marker present");
        }
    }

    #[test]
    fn every_write_chunk_embeds_parseable_marker() {
        for (key, size) in [
            (ScriptKey::MemoryRoundtrip, 4096usize),
            (ScriptKey::MemoryGrow, 4096usize),
            (ScriptKey::MemoryMixed, 4096usize),
            (ScriptKey::MemoryRoundtrip, 1024 * 1024),
            (ScriptKey::MemoryGrow, 1024 * 1024 + 3),
            (ScriptKey::MemoryMixed, 1024 * 1024),
        ] {
            let parsed = op(key, "u0", "1", size);
            let steps = key.steps(size);
            for step in &steps {
                let StepKind::MemoryWrite { size_bytes, .. } = step.kind else {
                    continue;
                };
                let arguments = build_arguments(&parsed, step).expect("planned chunk fits");
                let content = arguments["content"].as_str().expect("string");
                assert_eq!(content.len(), size_bytes, "{key:?} chunk exact size");
                assert!(
                    content.contains(&parsed.readback_token()),
                    "{key:?} chunk carries the read-back token"
                );
                assert!(
                    content.contains(&step_ordinal_token(step.step_index)),
                    "{key:?} chunk carries its plan-step ordinal"
                );
                let recovered = parse_marker_in_arguments(
                    &serde_json::to_string(&arguments).expect("serializes"),
                )
                .expect("{key:?} chunk carries a parseable marker");
                assert_eq!(recovered, parsed, "{key:?} chunk identity");
            }
        }
    }

    #[test]
    fn equal_size_append_chunks_have_distinct_content() {
        // A document that is an exact multiple of the per-call ceiling
        // splits into chunks of exactly MAX_MEMORY_WRITE_CHUNK_BYTES —
        // identical sizes, identical token and marker. Only the plan-step
        // ordinal keeps the content (and the serialized arguments) of
        // every chunk distinct, which is what the production no-progress
        // guard needs to keep the run alive past nine writes.
        let size = 5 * MAX_MEMORY_WRITE_CHUNK_BYTES;
        for key in [
            ScriptKey::MemoryRoundtrip,
            ScriptKey::MemoryGrow,
            ScriptKey::MemoryMixed,
        ] {
            let parsed = op(key, "u0", "1", size);
            let write_steps = key
                .steps(size)
                .into_iter()
                .filter(|step| matches!(step.kind, StepKind::MemoryWrite { .. }))
                .collect::<Vec<_>>();
            assert!(write_steps.len() >= 4, "{key:?} plan has multiple chunks");
            let mut contents = Vec::new();
            let mut serialized = Vec::new();
            for step in &write_steps {
                let StepKind::MemoryWrite { size_bytes, .. } = step.kind else {
                    unreachable!()
                };
                assert!(
                    size_bytes >= scripted_write_min_bytes(&parsed),
                    "{key:?} size minimum must stay below every chunk"
                );
                let arguments = build_arguments(&parsed, step).expect("planned chunk fits");
                let content = arguments["content"].as_str().expect("string");
                assert_eq!(content.len(), size_bytes, "{key:?} content stays exact");
                assert!(
                    content.contains(&parsed.readback_token()),
                    "{key:?} content carries the read-back token"
                );
                assert!(
                    content.contains(MARKER_PREFIX),
                    "{key:?} content carries the canonical marker"
                );
                assert!(
                    content.contains(&step_ordinal_token(step.step_index)),
                    "{key:?} content carries its plan-step ordinal"
                );
                contents.push(content.to_string());
                serialized.push(serde_json::to_string(&arguments).expect("serializes"));
            }
            // Any two chunks of the same size must differ in content and in
            // serialized arguments — the no-progress guard compares the
            // full call signature.
            for i in 0..contents.len() {
                for j in (i + 1)..contents.len() {
                    let same_size = matches!(
                        (write_steps[i].kind, write_steps[j].kind),
                        (
                            StepKind::MemoryWrite { size_bytes: a, .. },
                            StepKind::MemoryWrite { size_bytes: b, .. },
                        ) if a == b
                    );
                    if same_size {
                        assert_ne!(
                            contents[i], contents[j],
                            "{key:?} equal-size chunks {i} and {j} must differ in content"
                        );
                        assert_ne!(
                            serialized[i], serialized[j],
                            "{key:?} equal-size calls {i} and {j} must differ in arguments"
                        );
                    }
                }
            }
        }
        // Acceptance shape: a roundtrip document of five full-size chunks
        // is one replace plus four consecutive 60 KiB appends. Every call's
        // arguments differ while the marker and read-back token stay
        // identical.
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", size);
        let writes = ScriptKey::MemoryRoundtrip
            .steps(size)
            .into_iter()
            .filter(|step| matches!(step.kind, StepKind::MemoryWrite { .. }))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 5, "five full-size chunks");
        let mut serialized = Vec::new();
        for (index, step) in writes.iter().enumerate() {
            let StepKind::MemoryWrite { append, size_bytes } = step.kind else {
                unreachable!()
            };
            assert_eq!(size_bytes, MAX_MEMORY_WRITE_CHUNK_BYTES, "full-size chunk");
            assert_eq!(append, index > 0, "replace then appends");
            let arguments = build_arguments(&parsed, step).expect("planned chunk fits");
            let content = arguments["content"].as_str().expect("string");
            assert!(
                content.contains(&parsed.readback_token()),
                "token identical"
            );
            assert!(content.contains(MARKER_PREFIX), "marker identical");
            serialized.push(serde_json::to_string(&arguments).expect("serializes"));
        }
        for i in 0..serialized.len() {
            for j in (i + 1)..serialized.len() {
                assert_ne!(
                    serialized[i], serialized[j],
                    "append calls {i} and {j} differ"
                );
            }
        }
    }

    #[test]
    fn one_mib_append_calls_have_distinct_arguments() {
        // The production failure: a 1 MiB plan's consecutive full-size
        // chunks were byte-identical (same capability, same size, same
        // token and marker), so the no-progress guard ended the run after
        // nine calls. Every chunk must now differ in its serialized
        // arguments while still carrying the identical operation marker
        // and read-back identity.
        const MIB: usize = 1024 * 1024;
        for key in [
            ScriptKey::MemoryRoundtrip,
            ScriptKey::MemoryGrow,
            ScriptKey::MemoryMixed,
        ] {
            let parsed = op(key, "u0", "1", MIB);
            let marker = marker_message(key, "u0", "1", MIB);
            let writes = key
                .steps(MIB)
                .into_iter()
                .filter(|step| matches!(step.kind, StepKind::MemoryWrite { .. }))
                .collect::<Vec<_>>();
            assert!(
                writes.len() > 9,
                "{key:?} 1 MiB plan exceeds the guard's nine calls"
            );
            let mut serialized = Vec::new();
            for step in &writes {
                let StepKind::MemoryWrite { size_bytes, .. } = step.kind else {
                    unreachable!()
                };
                assert!(
                    size_bytes >= scripted_write_min_bytes(&parsed),
                    "{key:?} size minimum stays below every chunk"
                );
                let arguments = build_arguments(&parsed, step).expect("planned chunk fits");
                let content = arguments["content"].as_str().expect("string");
                assert_eq!(content.len(), size_bytes, "{key:?} chunk stays exact");
                assert!(
                    content.contains(&parsed.readback_token()),
                    "{key:?} chunk keeps the read-back identity"
                );
                assert!(
                    content.contains(&marker),
                    "{key:?} chunk keeps the identical operation marker"
                );
                serialized.push(serde_json::to_string(&arguments).expect("serializes"));
            }
            // Consecutive write calls (the sequence the guard observes)
            // must have pairwise-distinct arguments.
            for pair in serialized.windows(2) {
                assert_ne!(pair[0], pair[1], "{key:?} consecutive calls must differ");
            }
        }
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

    fn tool_call_message(id: &str, spec: &ToolCallSpec) -> Value {
        json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": id,
                "type": "function",
                "function": {
                    "name": spec.wire_name,
                    "arguments": serde_json::to_string(&spec.arguments).expect("arguments json")
                }
            }]
        })
    }

    fn tool_result_message(id: &str, text: &str) -> Value {
        json!({ "role": "tool", "tool_call_id": id, "content": text })
    }

    /// Drive a plan through a compacting [`ScriptedDriver`]: the first
    /// request carries the original user marker, and every later request
    /// retains only the previous assistant tool call and its result — the
    /// exact shape the production agent loop leaves after compaction.
    /// `result_for(step, index)` maps each plan step to the tool-result
    /// text its call receives. Every response must emit exactly one call in
    /// plan order; the loop ends when the driver finalizes. Returns the
    /// driver (post-finalize, so state removal is observable) and the
    /// final decision.
    fn drive_compacted_plan(
        key: ScriptKey,
        user_id: &str,
        op_id: &str,
        size: usize,
        result_for: &dyn Fn(&ScriptStep, usize) -> String,
    ) -> (ScriptedDriver, ScriptedDecision, usize) {
        let available = tools(&[
            "builtin__write_file",
            "builtin__read_file",
            "ironclaw__memory__write",
            "ironclaw__memory__read",
            "ironclaw__memory__search",
        ]);
        let parsed = op(key, user_id, op_id, size);
        let marker = marker_message(key, user_id, op_id, size);
        let steps = key.steps(size);
        let mut driver = ScriptedDriver::new(16);
        let mut messages = vec![user(&marker)];
        let mut round = 0usize;
        loop {
            let (decision, decided_op) = driver.decide(
                &json!({ "messages": messages.clone() }),
                &available,
                &format!("call-{round}"),
            );
            assert_eq!(decided_op.as_ref(), Some(&parsed), "round {round}");
            round += 1;
            match decision {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(calls.len(), 1, "round {}: plans never batch", round - 1);
                    let spec = calls[0].clone();
                    // The driver re-emits the same plan step when a
                    // write/append result is a structured error (bounded
                    // retry), so index the plan by the session's emitted
                    // counter, not a local round counter.
                    let step_index = driver
                        .sessions
                        .get(&parsed)
                        .expect("session active while a call is emitted")
                        .emitted
                        .checked_sub(1)
                        .expect("a tool call implies an emitted step");
                    let step = steps[step_index];
                    let expected_wire = resolve_wire_name(&available, step.capability_id)
                        .expect("plan step tool is advertised");
                    assert_eq!(spec.wire_name, expected_wire, "round {}", round - 1);
                    if let Some(content) = spec.arguments["content"].as_str() {
                        assert!(
                            content.len() <= MAX_MEMORY_WRITE_CHUNK_BYTES,
                            "round {}: chunk of {} bytes exceeds the per-call ceiling",
                            round - 1,
                            content.len()
                        );
                    }
                    let id = format!("call-{}", round - 1);
                    // Compaction: only the last assistant/tool pair survives.
                    messages = vec![
                        tool_call_message(&id, &spec),
                        tool_result_message(&id, &result_for(&step, step_index)),
                    ];
                }
                other => return (driver, other, round - 1),
            }
        }
    }

    #[test]
    fn compacted_one_mib_roundtrip_reaches_confirmed_in_19_singleton_calls() {
        const MIB: usize = 1024 * 1024;
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        assert_eq!(
            ScriptKey::MemoryRoundtrip.steps(MIB).len(),
            19,
            "18 writes plus one search checkpoint"
        );
        let own = parsed.readback_token();
        let (driver, decision, rounds) = drive_compacted_plan(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            MIB,
            &|step, _| match step.kind {
                StepKind::MemorySearch => own.clone(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} confirmed", parsed.identity()))
        );
        assert_eq!(
            rounds, 19,
            "one singleton call per round, 19 before the verdict"
        );
        assert!(driver.sessions.is_empty(), "finalized sessions are removed");
        assert!(
            driver.call_to_session.is_empty(),
            "finalized sessions release their call ids"
        );
    }

    #[test]
    fn compacted_one_mib_failed_write_still_yields_failure() {
        // The tenth of the 1 MiB plan's writes fails with a structured
        // error observation; the final search checkpoint still returns this
        // operation's own token. The failed write is compacted away before
        // the final round, but the sticky error evidence must still produce
        // a hard Failure.
        const MIB: usize = 1024 * 1024;
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let own = parsed.readback_token();
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;
        let (driver, decision, _) = drive_compacted_plan(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            MIB,
            &|step, index| match step.kind {
                StepKind::MemorySearch => own.clone(),
                StepKind::MemoryWrite { .. } if index == 9 => error_obs.to_string(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} failure", parsed.identity()))
        );
        assert!(driver.sessions.is_empty());
    }

    #[test]
    fn compacted_mixed_plan_intermediate_checkpoint_shapes_final_verdict() {
        // memory_mixed: write, read (checkpoint 1), write, read (checkpoint
        // 2). Checkpoint 1's result text is compacted away before the
        // verdict round, but its digested evidence must still be weighted.
        let parsed = op(ScriptKey::MemoryMixed, "u0", "1", 4096);
        let own = parsed.readback_token();
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let empty = "Tool ironclaw.memory.read returned: (empty)";
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":null,"trust":"untrusted_tool_output"}"#;

        // First checkpoint confirmed, final checkpoint empty: Missing, not
        // Confirmed — the intermediate checkpoint still matters.
        let (driver, decision, _) = drive_compacted_plan(
            ScriptKey::MemoryMixed,
            "u0",
            "1",
            4096,
            &|step, index| match step.kind {
                StepKind::MemoryRead if index == 1 => own.clone(),
                StepKind::MemoryRead => empty.to_string(),
                _ => "ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} missing", parsed.identity()))
        );
        assert!(driver.sessions.is_empty());

        // Final checkpoint carrying only a same-user token: Contended.
        let (_, decision, _) = drive_compacted_plan(
            ScriptKey::MemoryMixed,
            "u0",
            "1",
            4096,
            &|step, index| match step.kind {
                StepKind::MemoryRead if index == 1 => own.clone(),
                StepKind::MemoryRead => same_user.clone(),
                _ => "ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} contended", parsed.identity()))
        );

        // Both checkpoints confirm: Confirmed.
        let (_, decision, _) = drive_compacted_plan(
            ScriptKey::MemoryMixed,
            "u0",
            "1",
            4096,
            &|step, _| match step.kind {
                StepKind::MemoryRead => own.clone(),
                _ => "ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} confirmed", parsed.identity()))
        );

        // The second write fails with a structured error observation and is
        // compacted away; the final checkpoint returns the own token. The
        // sticky error evidence still yields a hard Failure.
        let (driver, decision, _) = drive_compacted_plan(
            ScriptKey::MemoryMixed,
            "u0",
            "1",
            4096,
            &|step, index| match step.kind {
                StepKind::MemoryWrite { .. } if index == 2 => error_obs.to_string(),
                StepKind::MemoryRead => own.clone(),
                _ => "ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} failure", parsed.identity()))
        );
        assert!(driver.sessions.is_empty());
    }

    #[test]
    fn driver_recovers_operation_from_tool_call_arguments() {
        // The original user marker is compacted away; only the assistant
        // tool call (whose arguments embed the canonical marker) and its
        // result remain. The driver must recover the operation from the
        // serialized arguments and continue the plan.
        let available = tools(&["ironclaw__memory__write", "ironclaw__memory__read"]);
        let mut driver = ScriptedDriver::new(8);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let (decision, _) = driver.decide(
            &json!({ "messages": vec![user(&marker)] }),
            &available,
            "call-0",
        );
        let ScriptedDecision::ToolCalls(calls) = decision else {
            panic!("expected the first write call, got {decision:?}");
        };
        let spec = calls[0].clone();
        let messages = vec![
            tool_call_message("call-0", &spec),
            tool_result_message("call-0", "Tool ironclaw.memory.write returned: ok"),
        ];
        match driver.decide(&json!({ "messages": messages }), &available, "call-1") {
            (ScriptedDecision::ToolCalls(calls), Some(decided)) => {
                assert_eq!(
                    decided,
                    op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096),
                    "identity recovered from the embedded marker"
                );
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].wire_name, "ironclaw__memory__read");
            }
            other => panic!("expected the read call, got {other:?}"),
        }
    }

    #[test]
    fn pending_call_without_result_never_advances_or_duplicates() {
        let available = tools(&["ironclaw__memory__write", "ironclaw__memory__read"]);
        let mut driver = ScriptedDriver::new(8);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let (decision, _) = driver.decide(
            &json!({ "messages": vec![user(&marker)] }),
            &available,
            "call-0",
        );
        let ScriptedDecision::ToolCalls(calls) = decision else {
            panic!("expected the first write call, got {decision:?}");
        };
        let spec = calls[0].clone();

        // The assistant tool call echoes but no tool result follows:
        // neither advance nor a duplicate call.
        let messages = vec![tool_call_message("call-0", &spec)];
        assert_eq!(
            driver.decide(&json!({ "messages": messages }), &available, "call-1"),
            (ScriptedDecision::Placeholder, Some(parsed.clone())),
            "a request without a new result must not emit a duplicate step"
        );
        let session = driver.sessions.values().next().expect("session exists");
        assert_eq!(session.emitted, 1, "no duplicate step emitted");
        assert_eq!(session.completed, 0, "no advance without a result");

        // The result arrives: exactly one advance and the next step emits.
        let messages = vec![
            tool_call_message("call-0", &spec),
            tool_result_message("call-0", "Tool ironclaw.memory.write returned: ok"),
        ];
        match driver.decide(&json!({ "messages": messages }), &available, "call-2") {
            (ScriptedDecision::ToolCalls(calls), _) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].wire_name, "ironclaw__memory__read");
            }
            other => panic!("expected the read call, got {other:?}"),
        }
        let session = driver.sessions.values().next().expect("session exists");
        assert_eq!(session.emitted, 2);
        assert_eq!(session.completed, 1);
    }

    #[test]
    fn concurrent_operations_remain_isolated() {
        // Two hot-writer operations sharing the same user (u0__1 and
        // u0__2) race through one driver. Interleaved requests must drive
        // each session independently and finalize each with its own
        // verdict.
        let available = tools(&["ironclaw__memory__write", "ironclaw__memory__read"]);
        let mut driver = ScriptedDriver::new(8);
        let op_a = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let op_b = op(ScriptKey::MemoryRoundtrip, "u0", "2", 4096);
        let marker_a = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let marker_b = marker_message(ScriptKey::MemoryRoundtrip, "u0", "2", 4096);
        let mut a_messages = vec![user(&marker_a)];
        let mut b_messages = vec![user(&marker_b)];
        let mut a_step = 0usize;
        let mut b_step = 0usize;
        let mut a_done = false;
        let mut b_done = false;
        let mut finals = Vec::new();
        let mut round = 0usize;
        while !(a_done && b_done) {
            if !a_done {
                let (decision, decided) = driver.decide(
                    &json!({ "messages": a_messages.clone() }),
                    &available,
                    &format!("a-{round}"),
                );
                assert_eq!(decided.as_ref(), Some(&op_a), "A round {round}");
                round += 1;
                let id = format!("a-{}", round - 1);
                match decision {
                    ScriptedDecision::ToolCalls(calls) => {
                        assert_eq!(calls.len(), 1, "A round {}: never batched", round - 1);
                        let spec = calls[0].clone();
                        let echoed = if a_step == 1 {
                            op_a.readback_token()
                        } else {
                            "ok".to_string()
                        };
                        a_messages = vec![
                            tool_call_message(&id, &spec),
                            tool_result_message(&id, &echoed),
                        ];
                        a_step += 1;
                    }
                    ScriptedDecision::FinalText(text) => {
                        finals.push(text);
                        a_done = true;
                    }
                    other => panic!("A round {}: unexpected {other:?}", round - 1),
                }
            }
            if !b_done {
                let (decision, decided) = driver.decide(
                    &json!({ "messages": b_messages.clone() }),
                    &available,
                    &format!("b-{round}"),
                );
                assert_eq!(decided.as_ref(), Some(&op_b), "B round {round}");
                round += 1;
                let id = format!("b-{}", round - 1);
                match decision {
                    ScriptedDecision::ToolCalls(calls) => {
                        assert_eq!(calls.len(), 1, "B round {}: never batched", round - 1);
                        let spec = calls[0].clone();
                        let echoed = if b_step == 1 {
                            op_b.readback_token()
                        } else {
                            "ok".to_string()
                        };
                        b_messages = vec![
                            tool_call_message(&id, &spec),
                            tool_result_message(&id, &echoed),
                        ];
                        b_step += 1;
                    }
                    ScriptedDecision::FinalText(text) => {
                        finals.push(text);
                        b_done = true;
                    }
                    other => panic!("B round {}: unexpected {other:?}", round - 1),
                }
            }
        }
        assert_eq!(
            driver.active_sessions(),
            0,
            "both finalized sessions are removed"
        );
        assert_eq!(
            finals,
            vec![
                format!("{RESULT_PREFIX} u0__1 confirmed"),
                format!("{RESULT_PREFIX} u0__2 confirmed"),
            ]
        );
    }

    #[test]
    fn driver_bounds_active_sessions_and_evicts_oldest() {
        let mut driver = ScriptedDriver::new(2);
        let available = tools(&["ironclaw__memory__write"]);
        for index in 0..4 {
            let user_id = format!("u{index}");
            let marker = marker_message(ScriptKey::MemoryRoundtrip, &user_id, "1", 4096);
            let (decision, _) = driver.decide(
                &json!({ "messages": vec![user(&marker)] }),
                &available,
                &format!("call-{index}"),
            );
            assert!(matches!(decision, ScriptedDecision::ToolCalls(_)));
        }
        assert_eq!(driver.sessions.len(), 2, "capacity bounds the map");
        assert_eq!(
            driver.call_to_session.len(),
            2,
            "the call-id index stays bounded with the sessions"
        );
        // The two oldest operations were evicted first.
        let remaining: Vec<String> = driver.sessions.keys().map(|op| op.user.clone()).collect();
        assert!(
            remaining.contains(&"u2".to_string()) && remaining.contains(&"u3".to_string()),
            "oldest sessions evicted, got {remaining:?}"
        );
    }

    /// Drive one operation through a fresh compacting [`ScriptedDriver`]
    /// with a per-attempt result provider. `result_for(step, index,
    /// attempt)` maps each plan step and attempt ordinal (1-based) to the
    /// tool-result text its call receives: a write/append step is emitted
    /// once per attempt, so a provider that returns a structured error
    /// observation for `attempt == 1` exercises the bounded retry, while
    /// checkpoint steps always resolve on their single attempt. Every
    /// response must emit exactly one call; the loop ends when the driver
    /// finalizes. Returns the driver (post-finalize, so state removal is
    /// observable), the final decision, and the round of the final
    /// decision (one per response, so the retry count is observable).
    fn drive_retrying_plan(
        key: ScriptKey,
        user_id: &str,
        op_id: &str,
        size: usize,
        result_for: &dyn Fn(&ScriptStep, usize, usize) -> String,
    ) -> (ScriptedDriver, ScriptedDecision, usize) {
        let available = tools(&[
            "builtin__write_file",
            "builtin__read_file",
            "ironclaw__memory__write",
            "ironclaw__memory__read",
            "ironclaw__memory__search",
        ]);
        let parsed = op(key, user_id, op_id, size);
        let marker = marker_message(key, user_id, op_id, size);
        let steps = key.steps(size);
        let mut driver = ScriptedDriver::new(16);
        let mut messages = vec![user(&marker)];
        let mut round = 0usize;
        loop {
            let (decision, decided_op) = driver.decide(
                &json!({ "messages": messages.clone() }),
                &available,
                &format!("call-{round}"),
            );
            assert_eq!(decided_op.as_ref(), Some(&parsed), "round {round}");
            round += 1;
            match decision {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(calls.len(), 1, "round {}: plans never batch", round - 1);
                    let spec = calls[0].clone();
                    let session = driver
                        .sessions
                        .get(&parsed)
                        .expect("session active while a call is emitted");
                    let step_index = session
                        .emitted
                        .checked_sub(1)
                        .expect("a tool call implies an emitted step");
                    // The attempt about to be emitted is one more than the
                    // structured-error attempts already recorded for the
                    // current step.
                    let attempt = session.write_attempts + 1;
                    let id = format!("call-{}", round - 1);
                    messages = vec![
                        tool_call_message(&id, &spec),
                        tool_result_message(
                            &id,
                            &result_for(&steps[step_index], step_index, attempt),
                        ),
                    ];
                }
                other => return (driver, other, round - 1),
            }
        }
    }

    #[test]
    fn driver_retries_failed_write_once_then_confirms() {
        // The write's first attempt returns a structured error observation
        // whose recovery explicitly permits an identical replay (the
        // transient CAS-contention shape, `same_call_retry=allowed`); the
        // retry succeeds and the checkpoint returns this operation's
        // token: Confirmed, with exactly one retry round.
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;
        let (driver, decision, rounds) = drive_retrying_plan(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            4096,
            &|step, _, attempt| match step.kind {
                StepKind::MemoryRead => own.clone(),
                StepKind::MemoryWrite { .. } if attempt == 1 => error_obs.to_string(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} confirmed", parsed.identity()))
        );
        assert_eq!(
            rounds, 3,
            "write, retry, read: three responses before the verdict"
        );
        assert!(driver.sessions.is_empty(), "finalized sessions are removed");
        assert!(
            driver.call_to_session.is_empty(),
            "finalized sessions release their call ids"
        );
    }

    #[test]
    fn driver_waits_for_retry_after_delay_before_reemitting() {
        let available = tools(&["ironclaw__memory__write", "ironclaw__memory__read"]);
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let mut driver = ScriptedDriver::new(8);
        let started = std::time::Instant::now();

        let (first, _) = driver.decide_at(
            &json!({ "messages": [user(&marker)] }),
            &available,
            "call-0",
            started,
        );
        let ScriptedDecision::ToolCalls(calls) = first else {
            panic!("expected initial write");
        };
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"transient"},"artifacts":[],"recovery":{"same_call_retry":"allowed_after_delay","recovery_hint":"wait_then_retry","retry_after_ms":250},"trust":"untrusted_tool_output"}"#;
        let request = json!({
            "messages": [
                tool_call_message("call-0", &calls[0]),
                tool_result_message("call-0", error_obs),
            ]
        });

        assert_eq!(
            driver.decide_at(&request, &available, "call-1", started),
            (
                ScriptedDecision::RetryAfter(std::time::Duration::from_millis(250)),
                Some(parsed.clone()),
            ),
        );
        assert_eq!(
            driver.decide_at(
                &request,
                &available,
                "call-2",
                started + std::time::Duration::from_millis(249),
            ),
            (
                ScriptedDecision::RetryAfter(std::time::Duration::from_millis(1)),
                Some(parsed.clone()),
            ),
        );
        let (retry, _) = driver.decide_at(
            &request,
            &available,
            "call-3",
            started + std::time::Duration::from_millis(250),
        );
        assert!(
            matches!(&retry, ScriptedDecision::ToolCalls(calls) if calls.len() == 1),
            "the identical write may be re-emitted only after the delay"
        );
    }

    #[test]
    fn driver_non_retryable_write_errors_fail_immediately() {
        // A structured error observation whose recovery does not permit an
        // immediate identical replay is a sticky failure on the first
        // attempt. This includes delayed retry without `retry_after_ms`,
        // because a missing provider delay never means "retry immediately".
        // The step is never re-opened (`rounds == 2`: write then read), and
        // the final verdict is a hard Failure even though the checkpoint
        // returns this operation's own token.
        for (label, recovery) in [
            (
                "forbidden",
                r#""recovery":{"same_call_retry":"forbidden","recovery_hint":"revise_approach"}"#,
            ),
            (
                "requires_changed_input",
                r#""recovery":{"same_call_retry":"requires_changed_input","recovery_hint":"correct_arguments_before_retry"}"#,
            ),
            (
                "not_useful",
                r#""recovery":{"same_call_retry":"not_useful","recovery_hint":"respect_failure_constraint"}"#,
            ),
            ("missing recovery", r#""recovery":null"#),
            (
                "delayed retry without delay",
                r#""recovery":{"same_call_retry":"allowed_after_delay","recovery_hint":"wait_then_retry"}"#,
            ),
        ] {
            let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
            let own = parsed.readback_token();
            let error_obs = format!(
                r#"{{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{{"kind":"generic_failure","failure_kind":"backend"}},"artifacts":[],{recovery},"trust":"untrusted_tool_output"}}"#
            );
            let (driver, decision, rounds) = drive_retrying_plan(
                ScriptKey::MemoryRoundtrip,
                "u0",
                "1",
                4096,
                &|step, _, _| match step.kind {
                    StepKind::MemoryRead => own.clone(),
                    _ => error_obs.clone(),
                },
            );
            assert_eq!(
                decision,
                ScriptedDecision::FinalText(format!(
                    "{RESULT_PREFIX} {} failure",
                    parsed.identity()
                )),
                "{label}: a non-retryable write error is an immediate sticky failure"
            );
            assert_eq!(rounds, 2, "{label}: write then read, with no retry round");
            assert!(driver.sessions.is_empty(), "{label}");
        }
    }

    #[test]
    fn driver_op_for_request_dedupes_repeated_readback_tokens() {
        // A request can carry the same read-back token more than once (an
        // observation preview echoing a write result twice, or two tool
        // results in one request). The fallback attribution must dedupe
        // duplicate occurrences of one operation's token: two occurrences
        // of the same token, one session, exactly one match.
        let available = tools(&["ironclaw__memory__write"]);
        let mut driver = ScriptedDriver::new(8);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let (decision, _) = driver.decide(
            &json!({ "messages": vec![user(&marker)] }),
            &available,
            "call-0",
        );
        assert!(matches!(decision, ScriptedDecision::ToolCalls(_)));
        let token = parsed.readback_token();
        // One tool result with the token twice.
        let one_message = vec![
            json!({ "role": "tool", "tool_call_id": "call-0", "content": format!("{token} {token}") }),
        ];
        assert_eq!(
            driver.op_for_request(&one_message),
            Some(parsed.clone()),
            "duplicate occurrences of one token dedupe to one match"
        );
        // Two tool results each carrying the same token.
        let two_messages = vec![
            json!({ "role": "tool", "tool_call_id": "call-0", "content": token.clone() }),
            json!({ "role": "tool", "tool_call_id": "call-0", "content": token }),
        ];
        assert_eq!(
            driver.op_for_request(&two_messages),
            Some(parsed.clone()),
            "the same token across messages is still one match"
        );
    }

    #[test]
    fn driver_retries_failed_append_then_contended() {
        // memory_grow: write the quarter, append the three quarters. The
        // append fails once under contention (its observation explicitly
        // allows an identical replay) and succeeds on retry; the
        // checkpoint returns only a same-user token (a hot-writer race
        // overwrote the document): Contended, not Failure.
        let parsed = op(ScriptKey::MemoryGrow, "u0", "1", 4096);
        let same_user = format!("{READBACK_MARKER}_u0__2");
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;
        let (driver, decision, rounds) = drive_retrying_plan(
            ScriptKey::MemoryGrow,
            "u0",
            "1",
            4096,
            &|step, index, attempt| match step.kind {
                StepKind::MemoryRead => same_user.clone(),
                StepKind::MemoryWrite { .. } if index == 1 && attempt == 1 => error_obs.to_string(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} contended", parsed.identity()))
        );
        assert_eq!(
            rounds, 4,
            "write, append, append retry, read: four responses before the verdict"
        );
        assert!(driver.sessions.is_empty());
    }

    #[test]
    fn driver_three_write_failures_end_failure() {
        // Every attempt of the write fails with an observation that
        // explicitly allows an identical replay: the step advances only on
        // the third failed attempt, the sticky error evidence survives, and
        // the final verdict is a hard Failure. `rounds` proves exactly
        // three write attempts — a fourth attempt would add a round, and a
        // non-retryable constraint would end after one.
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let own = parsed.readback_token();
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;
        let (driver, decision, rounds) = drive_retrying_plan(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            4096,
            &|step, _, attempt| match step.kind {
                StepKind::MemoryRead => own.clone(),
                StepKind::MemoryWrite { .. } if attempt <= 3 => error_obs.to_string(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} failure", parsed.identity()))
        );
        assert_eq!(
            rounds, 4,
            "three write attempts then the read: four responses before the verdict"
        );
        assert!(driver.sessions.is_empty(), "finalized sessions are removed");
        assert!(driver.call_to_session.is_empty());
    }

    #[test]
    fn driver_checkpoint_failure_stays_immediate() {
        // A checkpoint (read) structured error is not retried even when
        // the observation explicitly allows an identical replay: the very
        // next response is the final Failure, and only one read call was
        // ever emitted (`rounds == 1`).
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;
        let (driver, decision, rounds) = drive_retrying_plan(
            ScriptKey::MemoryRoundtrip,
            "u0",
            "1",
            4096,
            &|step, _, _| match step.kind {
                StepKind::MemoryRead => error_obs.to_string(),
                _ => "Tool ironclaw.memory.write returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} failure", parsed.identity()))
        );
        assert_eq!(
            rounds, 2,
            "write, read, then immediate Failure: no read retry round"
        );
        assert!(driver.sessions.is_empty());
    }

    #[test]
    fn driver_retries_failed_write_file_once_then_confirms() {
        // write_file_roundtrip: the write_file call fails once with an
        // observation that explicitly allows an identical replay and
        // succeeds on retry; the read returns this operation's token:
        // Confirmed.
        let parsed = op(ScriptKey::WriteFileRoundtrip, "u2", "7", 8192);
        let own = parsed.readback_token();
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;
        let (driver, decision, rounds) = drive_retrying_plan(
            ScriptKey::WriteFileRoundtrip,
            "u2",
            "7",
            8192,
            &|step, _, attempt| match step.kind {
                StepKind::ReadFile => own.clone(),
                StepKind::WriteFile if attempt == 1 => error_obs.to_string(),
                _ => "Tool builtin.write_file returned: ok".to_string(),
            },
        );
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} confirmed", parsed.identity()))
        );
        assert_eq!(
            rounds, 3,
            "write_file, retry, read: three responses before the verdict"
        );
        assert!(driver.sessions.is_empty());
        assert!(driver.call_to_session.is_empty());
    }

    #[test]
    fn driver_retry_uses_fresh_call_id_and_clears_stale_mapping() {
        // Mid-flight assertions for the retry: the same step is re-emitted
        // with a fresh call id, the completed counter does not advance, the
        // attempt count records the failure, and the call-id index drops
        // the stale id without leaking it. The error observation must
        // explicitly allow the identical replay, or no retry would occur.
        let available = tools(&["ironclaw__memory__write", "ironclaw__memory__read"]);
        let mut driver = ScriptedDriver::new(8);
        let marker = marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let parsed = op(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let error_obs = r#"{"schema_version":1,"status":"error","summary":"the tool call failed","detail":{"kind":"generic_failure","failure_kind":"backend"},"artifacts":[],"recovery":{"same_call_retry":"allowed","recovery_hint":"wait_then_retry"},"trust":"untrusted_tool_output"}"#;

        // Round 0: the first write emission.
        let (decision, _) = driver.decide(
            &json!({ "messages": vec![user(&marker)] }),
            &available,
            "call-0",
        );
        let ScriptedDecision::ToolCalls(calls) = decision else {
            panic!("expected the first write call, got {decision:?}");
        };
        let spec = calls[0].clone();
        assert_eq!(
            driver.call_to_session.get("call-0"),
            Some(&parsed),
            "the emitted call id is indexed"
        );

        // Round 1: the write result is a structured error; the same step is
        // re-opened with a fresh call id, `completed` does not advance, and
        // the stale id leaves the index.
        let messages = vec![
            tool_call_message("call-0", &spec),
            tool_result_message("call-0", error_obs),
        ];
        let (decision, _) = driver.decide(&json!({ "messages": messages }), &available, "call-1");
        let ScriptedDecision::ToolCalls(calls) = decision else {
            panic!("expected the retried write call, got {decision:?}");
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].wire_name, "ironclaw__memory__write", "same step");
        assert_eq!(calls[0].arguments, spec.arguments, "same step arguments");
        let session = driver.sessions.get(&parsed).expect("session active");
        assert_eq!(session.emitted, 1, "no new plan step on a retry");
        assert_eq!(session.completed, 0, "no advance on a failed attempt");
        assert_eq!(session.write_attempts, 1, "attempt recorded");
        assert_eq!(session.last_call_id.as_deref(), Some("call-1"));
        assert_eq!(driver.call_to_session.len(), 1, "index stays bounded");
        assert!(
            !driver.call_to_session.contains_key("call-0"),
            "the stale call id is released"
        );
        assert_eq!(driver.call_to_session.get("call-1"), Some(&parsed));

        // Round 2: the retry succeeds; the step advances, attempts reset,
        // and the next plan step (the read) emits with yet another id.
        let messages = vec![
            tool_call_message("call-1", &spec),
            tool_result_message("call-1", "Tool ironclaw.memory.write returned: ok"),
        ];
        let (decision, _) = driver.decide(&json!({ "messages": messages }), &available, "call-2");
        let ScriptedDecision::ToolCalls(calls) = decision else {
            panic!("expected the read call, got {decision:?}");
        };
        assert_eq!(calls[0].wire_name, "ironclaw__memory__read");
        let session = driver.sessions.get(&parsed).expect("session active");
        assert_eq!(session.emitted, 2);
        assert_eq!(session.completed, 1, "success advances exactly one step");
        assert_eq!(session.write_attempts, 0, "attempts cleared on success");
        assert_eq!(session.last_call_id.as_deref(), Some("call-2"));
        assert!(!driver.call_to_session.contains_key("call-1"));

        // Round 3: the checkpoint confirms; the operation finalizes and
        // releases its session and call id.
        let spec = calls[0].clone();
        let messages = vec![
            tool_call_message("call-2", &spec),
            tool_result_message("call-2", &parsed.readback_token()),
        ];
        let (decision, _) = driver.decide(&json!({ "messages": messages }), &available, "call-3");
        assert_eq!(
            decision,
            ScriptedDecision::FinalText(format!("{RESULT_PREFIX} {} confirmed", parsed.identity()))
        );
        assert!(driver.sessions.is_empty(), "finalized sessions are removed");
        assert!(
            driver.call_to_session.is_empty(),
            "finalized sessions release their call ids"
        );
    }
}
