//! IronClaw memory service contract for Reborn.
//!
//! This module owns the provider-neutral, host-facing IronClaw memory
//! operation shapes and the [`MemoryService`] trait. The default native
//! adapter and its storage behavior live in the `ironclaw_memory_native`
//! provider crate.

use std::fmt;

use async_trait::async_trait;
use chrono_tz::Tz;
use ironclaw_host_api::{ids::CorrelationId, resource::ResourceScope};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::metadata::DocumentMetadata;

const MAX_LOCALE_LEN: usize = 35;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInvocation {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceSearchRequest {
    pub query: String,
    pub limit: usize,
}

impl MemoryServiceSearchRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        let query = search_query(input)?.to_string();
        let limit = optional_u64(input, "limit").unwrap_or(5).clamp(1, 20) as usize;
        Ok(Self { query, limit })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceSearchResult {
    pub content: String,
    pub score: f32,
    pub path: String,
    pub is_hybrid_match: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceSearchResponse {
    pub query: String,
    pub results: Vec<MemoryServiceSearchResult>,
}

impl MemoryServiceSearchResponse {
    pub fn result_count(&self) -> usize {
        self.results.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryServiceWriteRequest {
    pub target: String,
    pub content: String,
    pub append: bool,
    /// Optional hash returned by a prior read. Providers that support
    /// conditional writes reject the write when the document has changed.
    pub expected_content_hash: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub replace_all: bool,
    pub metadata: Option<DocumentMetadata>,
    pub timezone: Option<String>,
}

impl MemoryServiceWriteRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        // Lenient parsing matching the pre-lift host `parse_write_command`: an
        // explicit JSON `null` target is treated as omitted (defaults to the
        // daily log), but any other present-but-wrong-typed `target` (number,
        // bool, object, array) is rejected. Most other present-but-wrong-typed
        // optional fields retain their historical coercion. The exception is
        // `expected_content_hash`: silently coercing a malformed expectation to
        // `None` would turn a conditional write into an unconditional one.
        // `new_string`/`timezone` are only consulted by the native write path
        // when relevant (patch / daily_log), preserving origin semantics.
        let target = match input.get("target") {
            Some(Value::String(target)) => target.to_string(),
            Some(Value::Null) | None => "daily_log".to_string(),
            Some(_) => return Err(MemoryServiceError::input()),
        };
        // Provider-neutral containment: reject a target that would escape the
        // scoped memory mount before it reaches any provider. The model-facing
        // `document-write` input schema advertises the same `not` pattern, but
        // that schema is only surfaced to the model — it is not host-validated
        // against the actual tool arguments — and a swapped provider may use the
        // target verbatim (the mem0 adapter stores it as a memory metadata tag).
        // The native filesystem provider keeps its own stricter
        // `reject_local_or_traversal_path` as defense in depth; enforcing here
        // closes the tool-surface path for every bound provider.
        reject_out_of_scope_target(&target)?;
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let expected_content_hash = match input.get("expected_content_hash") {
            Some(Value::String(hash)) => Some(hash.to_string()),
            Some(Value::Null) | None => None,
            Some(_) => return Err(MemoryServiceError::input()),
        };
        let old_string = input
            .get("old_string")
            .and_then(Value::as_str)
            .map(str::to_string);
        let new_string = input
            .get("new_string")
            .and_then(Value::as_str)
            .map(str::to_string);
        let append = if target == "daily_log" {
            true
        } else {
            input.get("append").and_then(Value::as_bool).unwrap_or(true)
        };
        let replace_all = input
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let metadata = input
            .get("metadata")
            .filter(|metadata| metadata.is_object())
            .map(DocumentMetadata::from_value);
        let timezone = input
            .get("timezone")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(Self {
            target,
            content,
            expected_content_hash,
            append,
            old_string,
            new_string,
            replace_all,
            metadata,
            timezone,
        })
    }
}

/// Outcome class of a memory write operation.
///
/// Status of a `profile_set` operation. The native provider only ever reports
/// success (`ok`); a failed write surfaces as an error, not a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProfileSetStatus {
    Ok,
}

/// Serializes to stable snake_case wire strings. `Conflict` is a correctable,
/// model-visible outcome; storage and host failures remain errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteStatus {
    Cleared,
    Conflict,
    Written,
    Patched,
}

impl MemoryWriteStatus {
    /// The stable wire string for this status (matches serde snake_case output).
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            MemoryWriteStatus::Cleared => "cleared",
            MemoryWriteStatus::Conflict => "conflict",
            MemoryWriteStatus::Written => "written",
            MemoryWriteStatus::Patched => "patched",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceWriteResponse {
    pub status: MemoryWriteStatus,
    pub path: String,
    pub append: bool,
    pub content_length: usize,
    pub replacements: Option<usize>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceReadRequest {
    pub path: String,
}

impl MemoryServiceReadRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        if input.get("version").is_some()
            || input.get("list_versions").and_then(Value::as_bool) == Some(true)
        {
            return Err(MemoryServiceError::input());
        }
        Ok(Self {
            path: required_str(input, "path")?.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceReadResponse {
    pub path: String,
    pub content: String,
    pub word_count: usize,
    /// SHA-256 hash of `content`, used for conditional follow-up writes.
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceTreeRequest {
    pub path: String,
    pub depth: usize,
}

impl MemoryServiceTreeRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let depth = optional_u64(input, "depth").unwrap_or(1).clamp(1, 10) as usize;
        Ok(Self { path, depth })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceTreeResponse {
    pub entries: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceProfileSetRequest {
    pub fields: Map<String, Value>,
}

impl MemoryServiceProfileSetRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        Ok(Self {
            fields: validated_profile_fields(input)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceProfileSetResponse {
    pub status: MemoryProfileSetStatus,
}

/// Response for a provider-neutral profile-document read.
///
/// The provider resolves the profile document's scope/path (keyed to the human
/// user at `agent=None, project=None`) and reads its raw bytes. The host parses,
/// size-caps, and validates them; the provider does not interpret the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceProfileReadResponse {
    /// Raw profile-document bytes for the run owner, or `None` if no profile
    /// document exists.
    pub document: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceContextRequest {
    pub query: String,
    pub max_snippets: usize,
    pub context_profile_id: MemoryContextProfileId,
}

/// Context-profile ids that disable memory-context retrieval entirely. Shared by
/// the host gate and the native provider's defense-in-depth check so the two
/// cannot desynchronize (see [`memory_context_disabled`]).
pub const MEMORY_DISABLED_CONTEXT_ALIASES: &[&str] = &[
    "memory_disabled",
    "memory-disabled",
    "disabled_context",
    "context_disabled",
];

/// Returns true if `context_profile_id` names a disabled memory-context profile.
/// The single source of truth for both the host gate and the provider check.
pub fn memory_context_disabled(context_profile_id: &str) -> bool {
    MEMORY_DISABLED_CONTEXT_ALIASES.contains(&context_profile_id)
}

/// Memory-owned context profile identifier.
///
/// Flows host → provider across the memory service boundary. Free-form profile
/// id (e.g. `"default"` and the disabled-context aliases), so validation is
/// minimal: non-empty. Constructed via [`MemoryContextProfileId::new`] or wire
/// deserialization, both routed through the same `validate`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct MemoryContextProfileId(String);

impl MemoryContextProfileId {
    fn validate(value: &str) -> Result<(), MemoryServiceError> {
        if value.is_empty() {
            return Err(MemoryServiceError::input());
        }
        Ok(())
    }

    pub fn new(raw: impl Into<String>) -> Result<Self, MemoryServiceError> {
        let value = raw.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for MemoryContextProfileId {
    type Error = MemoryServiceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for MemoryContextProfileId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemoryContextProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<MemoryContextProfileId> for String {
    fn from(id: MemoryContextProfileId) -> Self {
        id.0
    }
}
// Deliberately no `From<String>` / `From<&str>` — infallible conversion would
// silently bypass validation.
// Deliberately no `Deref<Target = str>` — auto-deref would let `&id` silently
// coerce to `&str`, the implicit-conversion pattern this rule prevents.

/// A raw memory-context candidate returned by a [`MemoryService`] provider.
///
/// The provider returns the *unsanitized* snippet body plus the resolved
/// scope/path components the host needs to build the model-visible reference.
/// The host — not the provider — sanitizes the text, wraps it in the
/// untrusted-memory envelope, hashes the `memory-snippet:*` reference, and
/// enforces every model-visible budget. A provider therefore cannot bypass host
/// prompt safety by pre-sanitizing, pre-wrapping, or forging a reference: the
/// host is the sole constructor of admitted loop-context snippets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceContextSnippet {
    /// Resolved memory scope/path components. The host hashes
    /// `[tenant_id, user_id, agent_id?, project_id?, relative_path]` into the
    /// stable `memory-snippet:*` display reference.
    pub tenant_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub relative_path: String,
    /// Raw, unsanitized snippet body. The host strips control characters,
    /// truncates, wraps it in the untrusted envelope, and runs the prompt-safety
    /// denylist before it can enter model context.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryServiceErrorKind {
    Input,
    Operation,
    Unavailable,
}

/// A memory service failure.
///
/// `kind` + `message` are the sanitized, user-/model-safe surface. `source`
/// carries the underlying backend cause (filesystem/JSON/UTF-8/CAS error) so the
/// host can log and correlate the real failure — it is never rendered into the
/// user-facing `Display`. Construct operation/unavailable failures from a backend
/// error with [`MemoryServiceError::operation_from`] /
/// [`MemoryServiceError::unavailable_from`] rather than dropping the cause.
#[derive(Debug)]
pub struct MemoryServiceError {
    kind: MemoryServiceErrorKind,
    message: &'static str,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl std::fmt::Display for MemoryServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "IronClaw memory {:?}: {}",
            self.kind, self.message
        )
    }
}

impl std::error::Error for MemoryServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl MemoryServiceError {
    pub fn input() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Input,
            message: "invalid memory request",
            source: None,
        }
    }

    pub fn operation() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Operation,
            message: "memory operation failed",
            source: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Unavailable,
            message: "memory provider unavailable",
            source: None,
        }
    }

    /// Input rejection that preserves the underlying validation cause for
    /// logging (the model-visible classification stays `Input`).
    pub fn input_from(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind: MemoryServiceErrorKind::Input,
            message: "invalid memory request",
            source: Some(Box::new(source)),
        }
    }

    /// Operation failure that preserves the underlying backend cause for logging.
    pub fn operation_from(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind: MemoryServiceErrorKind::Operation,
            message: "memory operation failed",
            source: Some(Box::new(source)),
        }
    }

    /// Provider-unavailable failure that preserves the underlying backend cause.
    pub fn unavailable_from(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind: MemoryServiceErrorKind::Unavailable,
            message: "memory provider unavailable",
            source: Some(Box::new(source)),
        }
    }

    pub fn kind(&self) -> MemoryServiceErrorKind {
        self.kind
    }
}

/// Role of a single message in an interaction exchange handed to a provider's
/// [`MemoryService::record_interaction`]. Typed (not a raw `String`) so a caller
/// cannot pass an unknown role; serializes snake_case for any provider that
/// forwards the `{role, content}` shape on the wire (mirrors mem0's message
/// shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryInteractionRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MemoryInteractionRole {
    /// Stable string form, matching the serde snake_case wire output.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryInteractionRole::User => "user",
            MemoryInteractionRole::Assistant => "assistant",
            MemoryInteractionRole::System => "system",
            MemoryInteractionRole::Tool => "tool",
        }
    }
}

/// One message in an interaction exchange passed to
/// [`MemoryService::record_interaction`].
///
/// `name` is the optional per-message actor label (mem0's message `name`, which a
/// provider may map to a per-memory `actor_id`): the human `user_id` for a user
/// message, the `agent_id` for an assistant message, `None` for a tool message.
/// Provider-neutral and opaque — the native provider stores it verbatim in the
/// transcript heading; a mem0 provider forwards it as the message `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInteractionMessage {
    pub role: MemoryInteractionRole,
    pub content: String,
    /// Optional actor label (mem0 message `name` → per-memory `actor_id`): user
    /// `user_id` / assistant `agent_id` / `None` for a tool message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Request for [`MemoryService::record_interaction`]: the raw interaction DATA.
///
/// Mirrors `mem0.add(messages=[...], metadata=...)`. The host passes the messages
/// and free-form `metadata` and lets the *provider* decide what to record (store
/// verbatim, run LLM extraction, or nothing) — the host makes no
/// verbatim-vs-extract / provenance / TTL decision. `user_id`/`agent_id`/
/// `thread_id` ride the invocation's [`ResourceScope`], not this request.
///
/// `turn_run_id` is the IronClaw per-turn run id, carried as **provenance** for
/// this exchange. It is NOT mem0's session/`run_id`: mem0's session id maps to our
/// `scope.thread_id` (the conversation) — which a provider derives from the
/// invocation scope — so one mem0 "run"/session spans many of our turns. The
/// native provider uses `turn_run_id` to name a per-run transcript file so that
/// re-recording the same run overwrites idempotently instead of duplicating.
/// `turn_run_id` and `metadata` are opaque provider pass-through.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryServiceRecordRequest {
    pub messages: Vec<MemoryInteractionMessage>,
    /// IronClaw per-turn run id (provenance), `None` when unavailable. Opaque
    /// provider pass-through — NOT the mem0 session id (that is `scope.thread_id`).
    pub turn_run_id: Option<String>,
    /// Free-form provenance metadata, opaque provider pass-through (e.g.
    /// `{ "turn_run_id", "correlation_id" }`). A provider self-generates
    /// timestamps; the host does not add them.
    pub metadata: Value,
}

/// Outcome of a [`MemoryService::record_interaction`] call.
///
/// `recorded` is `false` when the provider does not implement interaction
/// recording (the trait default) or degraded to a no-op because the request
/// lacked the scope it needs to record under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceRecordResponse {
    pub recorded: bool,
}

/// The host-initiated memory LIFECYCLE contract — the only stable part of
/// the memory system.
///
/// These four hooks fire at fixed points of the agent loop and are called
/// only when the bound provider's manifest declares the matching
/// `[memory].lifecycle` token. Everything model-facing is a manifest-declared
/// TOOL served through the ordinary first-party capability handler seam —
/// providers declare whatever tools they want and back them with their own
/// handler; nothing in the memory contract enumerates tool ids.
///
/// Every default fails closed (`unavailable`), except `record_interaction`,
/// whose no-op default reports `recorded: false`.
///
/// Lane retrieval returns RAW snippets: the host — never a provider — owns
/// scope filtering, sanitization, the untrusted-memory envelope, and the
/// model-visible byte budgets (`ironclaw_host_runtime::memory_context`).
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Read the run owner's profile document (loop start). The provider owns
    /// the scope/path resolution and returns raw bytes; the host parses +
    /// size-caps them.
    async fn profile_read(
        &self,
        invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        let _ = invocation;
        Err(MemoryServiceError::unavailable())
    }

    /// Long-term lane (retrieve-before-run): the user's general / durable
    /// memory. The host calls this only when the bound provider's manifest
    /// declares the `read_long_term` lifecycle hook.
    ///
    /// Returns RAW, ranked, in-scope candidates. The host — never the
    /// provider — applies the cross-scope drop filter, sanitization, the
    /// untrusted-memory envelope, and every model-visible byte budget (see
    /// [`MemoryServiceContextSnippet`]), and degrades a retrieval failure to
    /// an empty lane. A provider must exclude per-thread short-term scratch
    /// from this lane — regardless of whether the invocation scope carries an
    /// active thread — so the two lanes stay disjoint when the host
    /// concatenates them.
    async fn read_long_term(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    /// Short-term lane (retrieve-before-run): the active thread's (this
    /// conversation's) scratch memory, scoped by the trusted
    /// `invocation.scope.thread_id`. The host calls this only when the bound
    /// provider's manifest declares the `read_short_term` lifecycle hook.
    ///
    /// Same raw-return contract as
    /// [`read_long_term`](MemoryService::read_long_term): the host owns all
    /// prompt safety. A provider must restrict this lane to the active
    /// thread's content only.
    async fn read_short_term(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    /// Record a completed interaction exchange (the after-turn `add` seam).
    ///
    /// The host passes the raw interaction DATA — the ordered turn transcript
    /// messages, the per-turn `turn_run_id` (provenance, NOT the mem0 session id —
    /// that is `scope.thread_id`), and free-form `metadata` — and lets the
    /// *provider* decide what to do with it (store verbatim, run LLM extraction, or
    /// nothing). `turn_run_id` and `metadata` are opaque provider pass-through.
    /// `user_id`/`agent_id`/`thread_id` ride `invocation.scope`. Name-aligned with
    /// the reserved `memory.interaction.record.v1` op; this is a host-driven trait
    /// method, not a model-facing capability.
    ///
    /// Default: the provider does not record interactions — an infallible no-op
    /// returning `recorded: false`. A provider opts in by overriding. Unlike the
    /// other defaults (which fail closed as `unavailable`), the default here is
    /// `Ok` so the host's after-turn seam completes cleanly against any provider.
    async fn record_interaction(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceRecordRequest,
    ) -> Result<MemoryServiceRecordResponse, MemoryServiceError> {
        let _ = (invocation, request);
        tracing::debug!("memory provider does not implement record_interaction; skipping");
        Ok(MemoryServiceRecordResponse { recorded: false })
    }
}

// ---------------------------------------------------------------------------
// The shared memory tool vocabulary
// ---------------------------------------------------------------------------
// The five tools both bundled providers happen to declare, under the reserved
// stable namespace, plus the wire-output helpers their handlers share so the
// model-visible shapes cannot drift between backends. These are CONVENTIONS —
// not a required surface: a provider may declare any subset, or entirely
// different tools of its own, served by its own capability handler.

pub const MEMORY_SEARCH_CAPABILITY_ID: &str = "ironclaw.memory.search";
pub const MEMORY_WRITE_CAPABILITY_ID: &str = "ironclaw.memory.write";
pub const MEMORY_READ_CAPABILITY_ID: &str = "ironclaw.memory.read";
pub const MEMORY_TREE_CAPABILITY_ID: &str = "ironclaw.memory.tree";
pub const PROFILE_SET_CAPABILITY_ID: &str = "ironclaw.memory.profile_set";

/// Search-scope marker surfaced on every search output so the model knows the
/// search covered internal persistent memory only.
pub const MEMORY_SEARCH_SCOPE: &str = "reborn_internal_persistent_memory";

/// Maximum raw UTF-8 content bytes retained for one conventional search tool
/// result before JSON encoding.
///
/// Conventional search tool results are previews, not full documents. The
/// shared output boundary keeps every provider's model-visible raw result
/// content within the same budget without changing the raw responses providers
/// return to other consumers.
const MAX_SEARCH_RESULT_CONTENT_BYTES: usize = 8 * 1024;

/// Preceding context bytes kept around each exact-literal query occurrence in
/// a bounded conventional search tool result.
const SEARCH_EXCERPT_PRE_BYTES: usize = 128;
/// Following context bytes kept around each exact-literal query occurrence in
/// a bounded conventional search tool result. Together with the preceding
/// window this carries the matching sentence or record in typical memory
/// documents without allowing one match to consume the entire result budget.
const SEARCH_EXCERPT_POST_BYTES: usize = 256;
/// Separator between consecutive excerpts. The ellipsis is Unicode (three
/// UTF-8 bytes) and signals that bytes were elided between the excerpts.
const SEARCH_EXCERPT_DELIMITER: &str = "\n…\n";

/// Wire output for a search tool response. Shared by every provider declaring
/// the conventional search tool so the model-visible shape and per-result raw
/// content bound cannot drift between backends.
pub fn search_response_output(response: MemoryServiceSearchResponse) -> Value {
    let MemoryServiceSearchResponse { query, results } = response;
    let results = results
        .into_iter()
        .map(|result| {
            json!({
                "content": bound_search_result_content(result.content, &query),
                "score": result.score,
                "path": result.path,
                "is_hybrid_match": result.is_hybrid_match,
            })
        })
        .collect::<Vec<_>>();
    let result_count = results.len();
    json!({
        "query": query,
        "results": results,
        "result_count": result_count,
        "search_scope": MEMORY_SEARCH_SCOPE,
        "external_services_searched": false,
    })
}

/// Bound one conventional search tool result's content to
/// [`MAX_SEARCH_RESULT_CONTENT_BYTES`] UTF-8 bytes.
///
/// Short content is returned unchanged — zero allocation, byte-for-byte
/// identical. Oversized content is reduced deterministically:
///
/// - When the exact literal query occurs in the content, the preview contains
///   excerpts around successive non-overlapping occurrences. Each excerpt
///   keeps [`SEARCH_EXCERPT_PRE_BYTES`] preceding and
///   [`SEARCH_EXCERPT_POST_BYTES`] following bytes, and excerpts are joined by
///   an ellipsis delimiter. A window fully covered by the previous excerpt is
///   skipped, and accumulation stops at the cap. Repeated matches therefore
///   retain useful context from across a large document without returning the
///   whole body.
/// - When the exact literal query does not occur (a provider may stem or
///   otherwise normalize matches the output helper cannot reproduce), the
///   preview falls back to a plain bounded head. Matching is byte-exact on the
///   raw query — no case folding, stemming, or token heuristics — so a provider
///   match the literal query cannot reproduce is honestly shown as a head,
///   never as positioned excerpts.
///
/// Truncation never splits a multi-byte character, and the result is
/// deterministic.
fn bound_search_result_content(content: String, query: &str) -> String {
    if content.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES {
        return content;
    }
    if query.is_empty() {
        // The empty query matches everywhere; excerpting would degenerate to
        // the whole content. Keep the plain head.
        return bounded_search_head(content, MAX_SEARCH_RESULT_CONTENT_BYTES);
    }
    bounded_search_excerpts(&content, query)
        .unwrap_or_else(|| bounded_search_head(content, MAX_SEARCH_RESULT_CONTENT_BYTES))
}

/// Join bounded excerpts around successive non-overlapping exact-literal
/// occurrences of `query` in `content`, stopping at
/// [`MAX_SEARCH_RESULT_CONTENT_BYTES`]. Each excerpt
/// keeps up to [`SEARCH_EXCERPT_PRE_BYTES`] before the match and
/// [`SEARCH_EXCERPT_POST_BYTES`] after it, reducing that context when the exact
/// query would otherwise exceed the remaining budget. Endpoints are rounded to
/// UTF-8 character boundaries. Overlapping or touching windows are joined
/// directly; separated windows use [`SEARCH_EXCERPT_DELIMITER`]. The scan is a
/// single left-to-right pass (no occurrence list is materialized), and it stops
/// as soon as the cap is reached. Returns `None` when no exact occurrence can
/// fit, so the caller falls back to the bounded head.
fn bounded_search_excerpts(content: &str, query: &str) -> Option<String> {
    let mut out = String::new();
    let mut search_from = 0usize;
    let mut previous_end = 0usize;
    while let Some(relative) = content[search_from..].find(query) {
        let position = search_from + relative;
        let query_end = position + query.len();
        let mut desired_start = position.saturating_sub(SEARCH_EXCERPT_PRE_BYTES);
        while desired_start < position && !content.is_char_boundary(desired_start) {
            desired_start += 1;
        }
        let mut desired_end = (query_end + SEARCH_EXCERPT_POST_BYTES).min(content.len());
        while !content.is_char_boundary(desired_end) {
            desired_end -= 1;
        }
        if desired_start < previous_end && desired_end <= previous_end {
            // Fully covered by the previous excerpt; the query context
            // is already present.
            search_from = query_end;
            continue;
        }

        let contiguous = !out.is_empty() && desired_start <= previous_end;
        let delimiter_len = if out.is_empty() || contiguous {
            0
        } else {
            SEARCH_EXCERPT_DELIMITER.len()
        };
        let Some(available) = MAX_SEARCH_RESULT_CONTENT_BYTES
            .checked_sub(out.len())
            .and_then(|remaining| remaining.checked_sub(delimiter_len))
        else {
            break;
        };
        let mut start = if contiguous {
            previous_end
        } else {
            let Some(max_pre) = available.checked_sub(query.len()) else {
                break;
            };
            desired_start.max(position.saturating_sub(max_pre))
        };
        while start < position && !content.is_char_boundary(start) {
            start += 1;
        }
        if query_end.saturating_sub(start) > available {
            break;
        }

        let mut end = desired_end.min(start + available);
        while end > query_end && !content.is_char_boundary(end) {
            end -= 1;
        }
        if delimiter_len > 0 {
            out.push_str(SEARCH_EXCERPT_DELIMITER);
        }
        out.push_str(&content[start..end]);
        previous_end = end;
        search_from = query_end;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Cut `content` to a UTF-8-safe head of at most `bound` bytes, truncating in
/// place so oversized content is not copied. The cut never splits a multi-byte
/// character.
fn bounded_search_head(content: String, bound: usize) -> String {
    let mut content = content;
    let mut end = bound.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content
}

/// Wire output for a write tool response. Exhaustive over
/// [`MemoryWriteStatus`]; the `"status"` field serializes to the stable
/// snake_case wire strings (`cleared`/`patched`/`written`).
pub fn write_response_output(response: MemoryServiceWriteResponse) -> Value {
    match response.status {
        MemoryWriteStatus::Cleared => json!({
            "status": response.status,
            "path": response.path,
            "message": response.message.unwrap_or_default(),
        }),
        MemoryWriteStatus::Conflict => json!({
            "status": response.status,
            "path": response.path,
            "message": response.message.unwrap_or_default(),
        }),
        MemoryWriteStatus::Patched => json!({
            "status": response.status,
            "path": response.path,
            "replacements": response.replacements.unwrap_or(0),
            "content_length": response.content_length,
        }),
        MemoryWriteStatus::Written => json!({
            "status": response.status,
            "path": response.path,
            "append": response.append,
            "content_length": response.content_length,
        }),
    }
}

/// Wire output for a read tool response.
pub fn read_response_output(response: MemoryServiceReadResponse) -> Value {
    json!({
        "path": response.path,
        "content": response.content,
        "content_hash": response.content_hash,
        "word_count": response.word_count,
    })
}

/// Wire output for a tree tool response.
pub fn tree_response_output(response: MemoryServiceTreeResponse) -> Value {
    Value::Array(response.entries)
}

/// Wire output for a profile_set tool response.
pub fn profile_set_response_output(response: MemoryServiceProfileSetResponse) -> Value {
    json!({ "status": response.status })
}

fn search_query(input: &Value) -> Result<&str, MemoryServiceError> {
    for key in ["query", "q", "text", "pattern"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    Err(MemoryServiceError::input())
}

fn required_str<'a>(input: &'a Value, key: &'static str) -> Result<&'a str, MemoryServiceError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(MemoryServiceError::input)
}

fn optional_u64(input: &Value, key: &'static str) -> Option<u64> {
    input.get(key).and_then(Value::as_u64)
}

/// Reject a write `target` that would escape the scoped memory mount or fail to
/// name a document. Mirrors the fail-closed `not` pattern the model-facing
/// `document-write` input schema advertises: blank, absolute path (leading `/`),
/// any `..` traversal, or a backslash separator. Reserved names (`daily_log`,
/// `memory`, `heartbeat`, `bootstrap`) and ordinary relative document paths
/// (`notes/x.md`) pass unchanged.
fn reject_out_of_scope_target(target: &str) -> Result<(), MemoryServiceError> {
    if target.trim().is_empty()
        || target.starts_with('/')
        || target.contains("..")
        || target.contains('\\')
    {
        return Err(MemoryServiceError::input());
    }
    Ok(())
}

fn validated_profile_fields(input: &Value) -> Result<Map<String, Value>, MemoryServiceError> {
    let obj = input.as_object().ok_or_else(MemoryServiceError::input)?;
    let mut out = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "timezone" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?;
                value
                    .trim()
                    .parse::<Tz>()
                    .map_err(|_| MemoryServiceError::input())?;
                out.insert("timezone".into(), json!(value.trim()));
            }
            "locale" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?;
                validate_locale(value)?;
                out.insert("locale".into(), json!(value));
            }
            "location" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?.trim();
                if value.is_empty() || value.chars().count() > 200 || value.len() > 800 {
                    return Err(MemoryServiceError::input());
                }
                out.insert("location".into(), json!(value));
            }
            _ => return Err(MemoryServiceError::input()),
        }
    }
    if out.is_empty() {
        return Err(MemoryServiceError::input());
    }
    Ok(out)
}

fn validate_locale(value: &str) -> Result<(), MemoryServiceError> {
    if value.is_empty()
        || value.chars().count() > MAX_LOCALE_LEN
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        || value.split('-').any(str::is_empty)
    {
        return Err(MemoryServiceError::input());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::resource::ResourceScope;

    const QUERY: &str = "needle";
    const MIB: usize = 1 << 20;
    const HEAD_MATCH: &str = "needle-alpha-record";
    const MATCH_AFTER_CAP: &str = "needle-beta-record";
    const MATCH_MIDDLE: &str = "needle-gamma-record";
    const MATCH_TAIL: &str = "needle-delta-record";

    /// A 1 MiB ASCII body with `QUERY` starting at `query_at` (or absent).
    fn mib_body(query_at: Option<usize>) -> String {
        let mut body = vec![b'a'; MIB];
        if let Some(at) = query_at {
            body[at..at + QUERY.len()].copy_from_slice(QUERY.as_bytes());
        }
        String::from_utf8(body).expect("ASCII body is valid UTF-8")
    }

    /// A 1 MiB ASCII body with each `(position, text)` overlay written over
    /// the `a` padding.
    fn mib_body_with(overlays: &[(usize, &str)]) -> String {
        let mut body = vec![b'a'; MIB];
        for &(at, text) in overlays {
            body[at..at + text.len()].copy_from_slice(text.as_bytes());
        }
        String::from_utf8(body).expect("ASCII body is valid UTF-8")
    }

    /// A provider that overrides NOTHING — every `MemoryService` method (including
    /// `record_interaction`) is inherited from the trait default.
    struct NonRecordingProvider;
    impl MemoryService for NonRecordingProvider {}

    /// The two retrieval lanes fail closed by default: a provider that does
    /// not implement a lane reports `unavailable` (and, once bound, the host
    /// only calls lanes the provider's manifest declares).
    #[tokio::test]
    async fn lane_defaults_fail_closed_as_unavailable() {
        let provider = NonRecordingProvider;
        let request = || MemoryServiceContextRequest {
            query: "anything".to_string(),
            max_snippets: 5,
            context_profile_id: MemoryContextProfileId::new("default").expect("profile id"),
        };
        let invocation = || MemoryInvocation {
            scope: ResourceScope::system(),
            correlation_id: CorrelationId::new(),
        };

        let long = provider
            .read_long_term(invocation(), request())
            .await
            .expect_err("default read_long_term must fail closed");
        assert_eq!(long.kind(), MemoryServiceErrorKind::Unavailable);

        let short = provider
            .read_short_term(invocation(), request())
            .await
            .expect_err("default read_short_term must fail closed");
        assert_eq!(short.kind(), MemoryServiceErrorKind::Unavailable);
    }

    /// The default `record_interaction` is a host-driven no-op: it must NOT error
    /// (unlike the other default methods, which fail closed as `unavailable`) and
    /// must report `recorded: false` so a provider that does not opt in still lets
    /// the host's after-turn recording seam complete cleanly.
    #[tokio::test]
    async fn record_interaction_default_is_noop_returning_not_recorded() {
        let provider = NonRecordingProvider;
        let invocation = MemoryInvocation {
            scope: ResourceScope::system(),
            correlation_id: CorrelationId::new(),
        };
        let request = MemoryServiceRecordRequest {
            messages: vec![
                MemoryInteractionMessage {
                    role: MemoryInteractionRole::User,
                    content: "hello".to_string(),
                    name: Some("user-1".to_string()),
                },
                MemoryInteractionMessage {
                    role: MemoryInteractionRole::Assistant,
                    content: "hi there".to_string(),
                    name: Some("agent-1".to_string()),
                },
            ],
            turn_run_id: Some("run-1".to_string()),
            metadata: json!({}),
        };

        let response = provider
            .record_interaction(invocation, request)
            .await
            .expect("default record_interaction must be an infallible no-op");

        assert!(
            !response.recorded,
            "a provider that does not override record_interaction must report recorded=false"
        );
    }

    #[test]
    fn write_request_rejects_out_of_scope_targets() {
        // A traversal-shaped target must be rejected at the contract layer, ahead
        // of provider dispatch — the model-facing schema is not host-enforced, and
        // a swapped provider (e.g. mem0) would otherwise use the target verbatim.
        for target in [
            "",
            "   ",
            "/abs",
            "../escape",
            "notes/../secrets",
            "notes\\evil",
        ] {
            let input = json!({ "target": target, "content": "x" });
            let result = MemoryServiceWriteRequest::from_tool_input(&input);
            assert!(
                result.is_err_and(|error| error.kind() == MemoryServiceErrorKind::Input),
                "target {target:?} must be rejected as out-of-scope"
            );
        }
    }

    #[test]
    fn write_request_accepts_reserved_names_and_relative_paths() {
        // Reserved names and ordinary relative document paths are unaffected.
        for target in [
            "daily_log",
            "memory",
            "heartbeat",
            "bootstrap",
            "notes/sub.md",
        ] {
            let input = json!({ "target": target, "content": "x" });
            assert!(
                MemoryServiceWriteRequest::from_tool_input(&input).is_ok(),
                "target {target:?} must be accepted"
            );
        }
    }

    #[test]
    fn write_request_default_daily_log_target_is_accepted() {
        // The defaulted target (no `target` field) must also pass the guard.
        let input = json!({ "content": "x" });
        let request =
            MemoryServiceWriteRequest::from_tool_input(&input).expect("default target is in-scope");
        assert_eq!(request.target, "daily_log");
    }

    #[test]
    fn write_request_parses_expected_content_hash() {
        let request = MemoryServiceWriteRequest::from_tool_input(&json!({
            "target": "memory",
            "content": "curated",
            "append": false,
            "expected_content_hash": "abc123"
        }))
        .expect("conditional write input parses");

        assert_eq!(request.expected_content_hash.as_deref(), Some("abc123"));
    }

    #[test]
    fn write_request_rejects_malformed_expected_content_hash() {
        let error = MemoryServiceWriteRequest::from_tool_input(&json!({
            "target": "memory",
            "content": "curated",
            "append": false,
            "expected_content_hash": 1
        }))
        .expect_err("a malformed expectation must not become an unconditional write");

        assert_eq!(error.kind(), MemoryServiceErrorKind::Input);
    }

    #[test]
    fn write_request_allows_null_or_omitted_expected_content_hash() {
        for input in [
            json!({"target": "memory", "content": "x"}),
            json!({
                "target": "memory",
                "content": "x",
                "expected_content_hash": null
            }),
        ] {
            let request = MemoryServiceWriteRequest::from_tool_input(&input)
                .expect("null or omitted expectation remains optional");
            assert_eq!(request.expected_content_hash, None);
        }
    }

    #[test]
    fn conflict_write_output_is_model_visible() {
        let output = write_response_output(MemoryServiceWriteResponse {
            status: MemoryWriteStatus::Conflict,
            path: "MEMORY.md".to_string(),
            append: false,
            content_length: 0,
            replacements: None,
            message: Some("Document changed since it was read.".to_string()),
        });

        assert_eq!(output["status"], "conflict");
        assert!(
            output["message"]
                .as_str()
                .is_some_and(|message| message.contains("changed"))
        );
    }

    #[test]
    fn search_output_bounds_oversized_content_around_exact_query() {
        const QUERY: &str = "needle";
        const RESULT_BOUND: usize = 8 * 1024;
        let position = RESULT_BOUND + 512;
        let mut oversized = "a".repeat(position + QUERY.len() + RESULT_BOUND);
        oversized.replace_range(position..position + QUERY.len(), QUERY);

        let output = search_response_output(MemoryServiceSearchResponse {
            query: QUERY.to_string(),
            results: vec![MemoryServiceSearchResult {
                content: oversized,
                score: 1.0,
                path: "oversized.md".to_string(),
                is_hybrid_match: false,
            }],
        });
        let content = output["results"][0]["content"]
            .as_str()
            .expect("search result content is a string");

        assert!(content.len() <= RESULT_BOUND);
        assert!(content.contains(QUERY));
    }

    #[test]
    fn small_snippet_is_returned_unchanged() {
        let snippet = "a tiny memory note".to_string();
        assert_eq!(bound_search_result_content(snippet.clone(), QUERY), snippet);
    }

    #[test]
    fn empty_snippet_is_returned_unchanged() {
        assert_eq!(
            bound_search_result_content(String::new(), QUERY),
            String::new()
        );
    }

    #[test]
    fn snippet_exactly_at_bound_is_returned_unchanged() {
        let snippet = "a".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES);
        let bounded = bound_search_result_content(snippet.clone(), QUERY);
        assert_eq!(bounded, snippet);
    }

    #[test]
    fn snippet_one_byte_over_bound_is_cut_to_exact_cap() {
        // No exact literal query occurrence, so the fallback head lands
        // exactly on the cap: an ASCII body one byte over the bound
        // truncates to precisely MAX_SEARCH_RESULT_CONTENT_BYTES bytes.
        let body = "a".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES + 1);
        let bounded = bound_search_result_content(body, QUERY);
        assert_eq!(bounded, "a".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES));
        assert_eq!(bounded.len(), MAX_SEARCH_RESULT_CONTENT_BYTES);
    }

    #[test]
    fn oversized_body_with_exact_literal_query_retains_head_match() {
        // The query occurs literally near the start, so the excerpt window
        // starts at the snippet's head and the match survives verbatim.
        let body = mib_body(Some(10));
        let bounded = bound_search_result_content(body.clone(), QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(bounded.starts_with(&format!("{}{}", "a".repeat(10), QUERY)));
        assert!(bounded.contains(QUERY));
        assert_eq!(bound_search_result_content(body.clone(), QUERY), bounded);
    }

    #[test]
    fn no_exact_occurrence_falls_back_to_bounded_head() {
        // The backend may stem or otherwise normalize a match this provider
        // cannot reproduce literally. The preview is then an honest bounded
        // head — never positioned excerpts — and a query occurrence beyond
        // the cap stays invisible instead of being falsely claimed as a match.
        let body = mib_body(Some(MIB / 2));
        let bounded = bound_search_result_content(body, "nonexistent");
        assert_eq!(bounded.len(), MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(!bounded.contains(QUERY));
    }

    #[test]
    fn exact_literal_occurrence_beyond_head_is_retained() {
        // With an exact literal query match the preview positions a window
        // around it, so an occurrence in the body's middle — beyond a plain
        // head cut — is retained.
        let body = mib_body(Some(MIB / 2));
        let bounded = bound_search_result_content(body.clone(), QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(bounded.contains(QUERY));
        assert_eq!(bound_search_result_content(body, QUERY), bounded);
    }

    #[test]
    fn exact_query_near_cap_keeps_the_full_match_beyond_head() {
        let query = format!("q{}z", "x".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES - 66));
        let position = MAX_SEARCH_RESULT_CONTENT_BYTES + 128;
        let mut body = "a".repeat(position + query.len() + 512);
        body.replace_range(position..position + query.len(), &query);

        let bounded = bound_search_result_content(body, &query);

        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(bounded.contains(&query));
    }

    #[test]
    fn overlapping_excerpt_windows_preserve_contiguous_source() {
        let first = 4_000;
        let second = 4_100;
        let mut body = "a".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES + 1_000);
        body.replace_range(first..first + QUERY.len(), QUERY);
        body.replace_range(second..second + QUERY.len(), QUERY);
        let expected = body
            [first - SEARCH_EXCERPT_PRE_BYTES..second + QUERY.len() + SEARCH_EXCERPT_POST_BYTES]
            .to_string();

        let bounded = bound_search_result_content(body, QUERY);

        assert_eq!(bounded, expected);
        assert!(!bounded.contains(SEARCH_EXCERPT_DELIMITER));
    }

    #[test]
    fn matching_is_exact_literal_not_case_folded() {
        // An uppercase variant is NOT an exact literal match of the lowercase
        // query: no case folding, so the preview falls back to the head and
        // does not position windows it cannot substantiate.
        let body = mib_body(Some(MIB / 2)).replace(QUERY, "NEEDLE");
        let bounded = bound_search_result_content(body, QUERY);
        assert_eq!(bounded.len(), MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(!bounded.contains(QUERY));
    }

    #[test]
    fn oversized_1mib_multiple_matches_retain_each_query_context() {
        // Matches near the head, just beyond the result cap, in the middle,
        // and near the tail must all remain visible when their combined
        // excerpts fit. Returning only the head would hide three valid
        // matches from the caller.
        let body = mib_body_with(&[
            (100, HEAD_MATCH),
            (MAX_SEARCH_RESULT_CONTENT_BYTES + 64, MATCH_AFTER_CAP),
            (MIB / 2, MATCH_MIDDLE),
            (MIB - 256, MATCH_TAIL),
        ]);
        let bounded = bound_search_result_content(body.clone(), QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(
            std::str::from_utf8(bounded.as_bytes()).is_ok(),
            "excerpts must stay valid UTF-8"
        );
        for token in [HEAD_MATCH, MATCH_AFTER_CAP, MATCH_MIDDLE, MATCH_TAIL] {
            assert!(bounded.contains(token), "missing {token} in {bounded:?}");
        }
        assert_eq!(
            bound_search_result_content(body, QUERY),
            bounded,
            "excerpting must be deterministic"
        );
    }

    #[test]
    fn many_occurrences_stop_at_cap_with_complete_prefix() {
        // ~200 occurrences in 1 MiB: far more than the cap can hold, so the
        // excerpt list stops at the cap with the leading excerpts intact —
        // never a full dump, never a dangling delimiter.
        let mut overlays = Vec::new();
        for i in 0..200 {
            overlays.push((i * (MIB / 200), QUERY));
        }
        let body = mib_body_with(&overlays);
        let bounded = bound_search_result_content(body.clone(), QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(bounded.contains(QUERY));
        assert!(
            bounded.matches(SEARCH_EXCERPT_DELIMITER).count() >= 2,
            "several excerpts must have been joined before the cap"
        );
        assert!(
            !bounded.ends_with(SEARCH_EXCERPT_DELIMITER),
            "no trailing delimiter"
        );
        assert_eq!(bound_search_result_content(body, QUERY), bounded);
    }

    #[test]
    fn multibyte_excerpt_windows_never_split_chars() {
        // Three-byte chars around the occurrence: the pre-window start
        // (position - 128, and 128 % 3 == 2) and the post-window end both
        // round to whole characters, so the excerpt stays valid UTF-8 and
        // the matching record survives.
        let mut body = String::new();
        body.push_str(&"€".repeat(4000));
        body.push_str(HEAD_MATCH);
        body.push_str(&"€".repeat(4000));
        let bounded = bound_search_result_content(body, QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
        assert!(bounded.contains(HEAD_MATCH));
    }

    #[test]
    fn multibyte_excerpt_desired_end_rounds_down_to_a_char_boundary() {
        let body = format!("{}{}{}", "a".repeat(8_500), QUERY, "€".repeat(300));

        let bounded = bound_search_result_content(body, QUERY);

        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert!(bounded.contains(QUERY));
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
        assert!(bounded.ends_with('€'));
    }

    #[test]
    fn match_already_covered_by_the_previous_tail_excerpt_is_not_duplicated() {
        let first = 8_900;
        let second = 8_950;
        let mut body = "a".repeat(9_000);
        body.replace_range(first..first + QUERY.len(), QUERY);
        body.replace_range(second..second + QUERY.len(), QUERY);
        let expected = body[first - SEARCH_EXCERPT_PRE_BYTES..].to_string();

        let bounded = bound_search_result_content(body, QUERY);

        assert_eq!(bounded, expected);
        assert_eq!(bounded.matches(QUERY).count(), 2);
        assert!(!bounded.contains(SEARCH_EXCERPT_DELIMITER));
    }

    #[test]
    fn contiguous_long_match_that_cannot_fit_stops_at_the_complete_prefix() {
        let query = "q".repeat(4_100);
        let body = query.repeat(2);

        let bounded = bound_search_result_content(body, &query);

        assert_eq!(bounded.len(), query.len() + SEARCH_EXCERPT_POST_BYTES);
        assert!(bounded.starts_with(&query));
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
    }

    #[test]
    fn cap_clipped_multibyte_excerpt_rounds_start_and_end_to_char_boundaries() {
        let query = "q".repeat(3_903);
        let mut body = query.clone();
        body.push_str(&"a".repeat(1_000));
        body.push_str(&"€".repeat(400));
        body.push_str(&query);
        body.push_str(&"€".repeat(400));

        let bounded = bound_search_result_content(body, &query);

        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert_eq!(bounded.matches(&query).count(), 2);
        assert!(bounded.contains(SEARCH_EXCERPT_DELIMITER));
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
        assert!(bounded.ends_with('q'));
    }

    #[test]
    fn empty_query_keeps_bounded_head() {
        // The empty query matches everywhere; excerpting would degenerate to
        // the whole snippet, so the preview stays a plain head — the query
        // near the start survives only because it is inside the head.
        let body = mib_body(Some(10));
        let bounded = bound_search_result_content(body.clone(), "");
        assert_eq!(bounded.len(), MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert_eq!(bounded, &body[..MAX_SEARCH_RESULT_CONTENT_BYTES]);
    }

    #[test]
    fn query_longer_than_cap_falls_back_to_head() {
        // A single excerpt cannot fit the query plus its context, so the
        // excerpt builder yields nothing and the preview stays an honest
        // head rather than a truncated query fragment.
        let body = mib_body(Some(0));
        let query = "a".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES + 1);
        let bounded = bound_search_result_content(body.clone(), &query);
        assert_eq!(bounded.len(), MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert_eq!(bounded, &body[..MAX_SEARCH_RESULT_CONTENT_BYTES]);
    }

    #[test]
    fn multibyte_head_cut_never_splits_chars() {
        // Three-byte chars: the 8192-byte cut lands mid-character (8192 % 3
        // == 2) and must round down to a whole number of chars. No exact
        // literal occurrence, so this exercises the head fallback.
        let body = "€".repeat(2731);
        assert!(body.len() > MAX_SEARCH_RESULT_CONTENT_BYTES);
        let bounded = bound_search_result_content(body.clone(), QUERY);
        assert!(bounded.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES);
        assert_eq!(bounded, "€".repeat(MAX_SEARCH_RESULT_CONTENT_BYTES / 3));
    }

    #[test]
    fn truncated_preview_is_deterministic() {
        let body = mib_body(Some(10));
        assert_eq!(
            bound_search_result_content(body.clone(), QUERY),
            bound_search_result_content(body, QUERY)
        );
    }
}
