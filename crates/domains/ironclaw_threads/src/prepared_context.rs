//! The prepared-context accept door's contract types and shared policy helpers.
//!
//! `accept_prepared_context` is the ONE shared accept door for every
//! non-channel caller (subagent spawn, OpenAI-compat, suggestions, future
//! unbound features — docs/internal/design/2026-08-12-unbound-turns.md
//! §4.2): it mints an unbound thread, seeds the caller's complete
//! point-in-time context as ordinary transcript rows, journals the per-run
//! declarations beside them, and is idempotent by `idempotency_key` — a
//! crash-retry returns the SAME prepared context instead of minting an
//! orphan. The helpers here are shared by both backends (the in-memory and
//! filesystem services) so the mint/seed/replay policy cannot drift.
//!
//! Replay discipline: the thread id and every seeded message id are pure
//! deterministic functions of `(scope, idempotency_key)`, so a partially
//! crashed accept re-runs to the same rows (existing rows are skipped) and
//! the journaled [`PreparedContextRecord`] — written last — is the commit
//! marker a replay returns from.

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_host_api::prepared_context::{OutputContract, PreparedTurnDeclarations};
use ironclaw_host_api::turn::AcceptedMessageRef;
use ironclaw_llm::agent_message::{
    AGENT_MESSAGE_TEXT_PART_MAX_BYTES, AgentMessage, AgentMessageRole, ContentPart,
    validate_agent_messages,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::tool_result_reference::{
    ProviderToolCallReferenceEnvelope, ToolResultReferenceEnvelope, ToolResultSafeSummary,
};
use crate::{
    AttachmentRef, MessageKind, MessageStatus, SessionThreadError, ThreadMessageId,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_host_api::ids::ProviderToolName;
use ironclaw_llm::agent_message::{ToolCallContent, ToolResultOutcome};

/// Provider identity stamped on SEEDED tool-history envelopes. The model
/// gateway's replay-identity gate carves this exact value out so a seeded
/// tool round replays as a faithful tool_use/tool_result exchange; every
/// other mismatched identity still degrades to the summary-style user
/// message. Seeded envelopes are host-normalized: `signature` is forced
/// `None` so caller-authored history can never smuggle provider replay
/// artifacts.
pub const PREPARED_SEED_PROVIDER_ID: &str = "prepared-context-seed";

/// Preview budget for a seeded tool result's model observation: one durable
/// record-read window, mirroring the loop's own first-look preview size.
const PREPARED_SEED_PREVIEW_MAX_BYTES: usize = crate::contract::TOOL_RESULT_RECORD_READ_MAX_BYTES;

/// Provider-metadata text budget for seeded `response_reasoning`
/// (mirrors `ironclaw_safety::PROVIDER_METADATA_TEXT_MAX_BYTES`).
const PREPARED_SEED_REASONING_MAX_BYTES: usize = ironclaw_safety::PROVIDER_METADATA_TEXT_MAX_BYTES;

/// Current schema version for [`PreparedContextRecord`] rows.
pub const PREPARED_CONTEXT_RECORD_SCHEMA_VERSION: u32 = 1;

/// Serialized-size cap for a declared `response_format` JSON Schema output
/// contract. Chosen at the same scale as [`AGENT_MESSAGE_TEXT_PART_MAX_BYTES`]
/// (the transcript's own per-part text budget): the schema is journaled with
/// the rest of the declarations and, unlike ordinary transcript text, is
/// compiled into a `jsonschema` validator once per RUN
/// (`ironclaw_loop_host::structured_result::validator_for`) — an unbounded
/// caller-supplied schema is request-triggered CPU/memory amplification, so
/// it gets the same budget as the other untrusted-content parts rather than
/// riding the raw 14 MiB chat-body cap.
pub const PREPARED_OUTPUT_SCHEMA_MAX_BYTES: usize = AGENT_MESSAGE_TEXT_PART_MAX_BYTES;

/// Nesting-depth cap for a declared `response_format` JSON Schema output
/// contract, mirroring [`ironclaw_safety`]'s existing tool-argument depth
/// bound (`PROVIDER_ARGUMENTS_MAX_DEPTH` / the validator's `MAX_DEPTH`).
/// Measured iteratively over an explicit stack — never recursively — because
/// the schema is untrusted request content and a recursive walk would let a
/// pathologically deep (but small) schema exhaust the stack while merely
/// being measured.
pub const PREPARED_OUTPUT_SCHEMA_MAX_DEPTH: usize = 32;

/// The prepared-context accept request.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedContextRequest {
    /// Scope of the minted thread. Callers name a real owner: product
    /// surfaces pass the authenticated caller (`UnboundTurnService` threads
    /// its actor), and subagent spawn mirrors the parent thread's owner so
    /// its evidence checks read the child edge back under the parent's
    /// real-owner scope. The owner shards prepared threads per-user and is
    /// NOT what hides them — invisibility to every owner-scoped
    /// conversation listing comes from the unconditional `prepared_context`
    /// metadata stamp. `owner_user_id: None` remains accepted for legacy
    /// engine-level callers but lands the thread in the tenant system slot;
    /// do not introduce new `None` producers.
    pub scope: ThreadScope,
    /// Acting identity recorded on the seeded user rows
    /// (run-acts-as-invoker).
    pub actor_id: String,
    /// The caller's task prompt; seeded as a `System` row when non-empty.
    /// The resolved profile may prepend host protocol assets at
    /// materialization — callers own the task prompt, not the host frame.
    pub system_prompt: String,
    /// Complete point-in-time input, seeded as the thread's rows. Must be
    /// non-empty: the last message is the accepted pin the submit carries.
    pub messages: Vec<AgentMessage>,
    /// Journaled beside the content; read at admission to derive the
    /// unbound profile and at host build to enforce the output contract.
    pub declarations: PreparedTurnDeclarations,
    /// Replay key: retrying returns the same prepared context.
    pub idempotency_key: String,
    /// The thread to mint, chosen by the caller and always server-generated
    /// upstream (subagent spawn passes `subagent-{child_run_id}`; OpenAI-compat
    /// passes the public completion id). Idempotent replay converges on this
    /// id via the journaled record; the seeded ROW ids stay deterministic
    /// functions of it so crashed retries converge row-by-row.
    pub thread_id: ThreadId,
    /// Optional human-facing title and metadata for the minted thread (the
    /// subagent path stores its crash-reconstruction metadata here).
    pub title: Option<String>,
    pub metadata_json: Option<String>,
}

/// The prepared-context pin the workflow then submits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPreparedContext {
    pub thread_id: ThreadId,
    /// Pin for `submit_turn`: `msg:{last seeded message id}`.
    pub accepted_message_ref: AcceptedMessageRef,
    pub idempotent_replay: bool,
}

/// Durable journal record for a prepared context, stored beside the
/// thread. Written LAST during the accept, so its presence is the commit
/// marker replays return from; admission and the loop host read the
/// declarations back through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedContextRecord {
    pub schema_version: u32,
    pub idempotency_key: String,
    pub actor_id: String,
    /// Wire form of the accepted pin (`msg:{message_id}`).
    pub accepted_message_ref: String,
    #[serde(default)]
    pub declarations: PreparedTurnDeclarations,
    pub seeded_message_count: u64,
    pub created_at: DateTime<Utc>,
}

/// Deterministic message id for seeded row `index` of an unbound thread, so
/// a crashed retry re-writes the same rows instead of duplicating them
/// (precedent: the capability display preview's derived message ids).
pub(crate) fn prepared_seed_message_id(thread_id: &ThreadId, index: usize) -> ThreadMessageId {
    let mut hasher = Sha256::new();
    hasher.update(b"prepared-context-seed:v1\0");
    hasher.update(thread_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(index.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    ThreadMessageId::from_uuid(Uuid::from_bytes(bytes))
}

pub(crate) fn accepted_prepared_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, SessionThreadError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|error| {
        SessionThreadError::Backend(format!("accepted unbound message ref invalid: {error}"))
    })
}

/// Metadata key stamped onto every thread the accept door mints, so
/// owner-scoped listings can exclude prepared-context (unbound/subagent)
/// threads without a schema change.
pub const PREPARED_CONTEXT_METADATA_MARKER_KEY: &str = "prepared_context";

/// Stamp the caller's metadata (if any) with the prepared-context marker.
/// Non-object metadata is rejected — the marker must be able to coexist with
/// whatever the caller stored (the subagent path keeps its
/// crash-reconstruction fields here).
pub(crate) fn stamped_metadata_json(
    metadata_json: Option<&str>,
) -> Result<String, SessionThreadError> {
    let mut object = match metadata_json {
        None => serde_json::Map::new(),
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return Err(invalid("metadata_json must be a JSON object")),
        },
    };
    object.insert(
        PREPARED_CONTEXT_METADATA_MARKER_KEY.to_string(),
        serde_json::Value::Bool(true),
    );
    Ok(serde_json::Value::Object(object).to_string())
}

/// Listing-side predicate: prepared-context threads are working state, not
/// conversations, and stay out of owner-scoped listings. Matches the stamp
/// on every door-minted thread plus the pre-marker subagent metadata shape
/// (`"kind":"subagent"`) so pre-existing child threads hide too.
pub fn record_is_prepared_context_hidden(record: &crate::SessionThreadRecord) -> bool {
    record
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| {
            value.get(PREPARED_CONTEXT_METADATA_MARKER_KEY) == Some(&serde_json::Value::Bool(true))
                || value.get("kind").and_then(serde_json::Value::as_str) == Some("subagent")
        })
}

fn invalid(reason: impl Into<String>) -> SessionThreadError {
    SessionThreadError::InvalidPreparedContext {
        reason: reason.into(),
    }
}

/// Validate the request fail-closed before any state is minted.
pub(crate) fn validate_prepared_context_request(
    request: &PreparedContextRequest,
) -> Result<(), SessionThreadError> {
    if request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 256
        || request.idempotency_key.chars().any(char::is_control)
    {
        return Err(invalid(
            "idempotency_key must be 1..=256 bytes with no control characters",
        ));
    }
    if request.actor_id.is_empty() {
        return Err(invalid("actor_id must not be empty"));
    }
    validate_prepared_seed_content(&request.system_prompt, &request.messages)?;
    validate_output_contract(&request.declarations.output)
}

/// Validate a declared output contract's JSON Schema against the door's size
/// and nesting bounds, fail-closed before any state is minted. This is the
/// ONE authoritative check: `ironclaw_openai_compat::prepared_turn::parse_response_format`
/// calls [`validate_output_schema`] directly, in-process, before its own
/// idempotency reservation — the same in-process-call pattern
/// `validate_prepared_seed_content` already uses — so there is no mirrored
/// bound to drift.
pub fn validate_output_contract(output: &OutputContract) -> Result<(), SessionThreadError> {
    match output {
        OutputContract::AssistantMessage => Ok(()),
        OutputContract::JsonSchema { schema } => validate_output_schema(schema),
    }
}

/// Bounds-check a declared JSON Schema value: serialized size and nesting
/// depth. See [`PREPARED_OUTPUT_SCHEMA_MAX_BYTES`] and
/// [`PREPARED_OUTPUT_SCHEMA_MAX_DEPTH`] for why these bounds exist.
pub fn validate_output_schema(schema: &serde_json::Value) -> Result<(), SessionThreadError> {
    let serialized = serde_json::to_string(schema)
        .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    if serialized.len() > PREPARED_OUTPUT_SCHEMA_MAX_BYTES {
        return Err(invalid(format!(
            "response_format json_schema exceeds {PREPARED_OUTPUT_SCHEMA_MAX_BYTES} bytes"
        )));
    }
    if json_value_max_depth(schema) > PREPARED_OUTPUT_SCHEMA_MAX_DEPTH {
        return Err(invalid(format!(
            "response_format json_schema exceeds {PREPARED_OUTPUT_SCHEMA_MAX_DEPTH} levels of nesting"
        )));
    }
    Ok(())
}

/// Iterative (explicit-stack) nesting-depth measurement over untrusted JSON.
/// No recursion: an adversarial deeply-nested-but-small schema must not be
/// able to blow the call stack while its depth is merely being measured.
fn json_value_max_depth(value: &serde_json::Value) -> usize {
    let mut max_depth = 0usize;
    let mut stack: Vec<(&serde_json::Value, usize)> = vec![(value, 1)];
    while let Some((current, depth)) = stack.pop() {
        max_depth = max_depth.max(depth);
        match current {
            serde_json::Value::Array(items) => {
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            serde_json::Value::Object(map) => {
                stack.extend(map.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    max_depth
}

/// The content-deterministic half of the accept door's validation: message
/// shape, byte budgets, tool pairing, and provider-grammar seedability.
/// Product surfaces call THIS (the one authoritative validator) before
/// reserving idempotency state, so a body the door would refuse never burns
/// a caller's key — and there is no mirrored copy to drift.
pub fn validate_prepared_seed_content(
    system_prompt: &str,
    messages: &[AgentMessage],
) -> Result<(), SessionThreadError> {
    if messages.is_empty() {
        return Err(invalid(
            "messages must not be empty; the last message is the accepted pin",
        ));
    }
    if system_prompt.len() > AGENT_MESSAGE_TEXT_PART_MAX_BYTES {
        return Err(invalid(format!(
            "system_prompt exceeds {AGENT_MESSAGE_TEXT_PART_MAX_BYTES} bytes"
        )));
    }
    validate_agent_messages(messages)
        .map_err(|error| invalid(format!("invalid message list: {error}")))?;
    // Seeded tool history is journaled as provider replay metadata, so every
    // caller-supplied identity fragment must satisfy the provider grammars
    // BEFORE any state is minted (validate-before-mint discipline: the
    // filesystem backend ensures the thread between validation and seeding).
    for message in messages {
        let has_tool_call = message
            .content
            .iter()
            .any(|part| matches!(part, ContentPart::ToolCall(_)));
        for part in &message.content {
            match part {
                ContentPart::ToolCall(call) => {
                    ironclaw_safety::validate_provider_token(&call.call_id, "tool call id", 512)
                        .map_err(|error| {
                            invalid(format!("tool call id is not seedable: {error}"))
                        })?;
                    seeded_provider_tool_name(&call.capability)?;
                }
                // Reasoning has exactly one storage slot in the transcript:
                // the provider envelope of a tool-call turn. A final answer's
                // reasoning would be silently dropped — reject instead.
                ContentPart::Reasoning { .. } if !has_tool_call => {
                    return Err(invalid(
                        "reasoning parts are seedable only on assistant messages \
                         that also carry a tool call",
                    ));
                }
                ContentPart::Reasoning { .. }
                | ContentPart::ToolResult(_)
                | ContentPart::Text { .. }
                | ContentPart::Image { .. }
                | ContentPart::File { .. } => {}
            }
        }
    }
    Ok(())
}

/// Provider-facing tool name derived from the capability id, using the same
/// `.` -> `__` mapping the live tool surface renders.
fn seeded_provider_tool_name(
    capability: &ironclaw_host_api::ids::CapabilityId,
) -> Result<ProviderToolName, SessionThreadError> {
    ProviderToolName::for_capability(capability).map_err(|error| {
        invalid(format!(
            "tool call capability {capability} has no provider-safe tool name: {error}"
        ))
    })
}

fn seeded_text_and_attachments(parts: &[ContentPart]) -> (String, Vec<AttachmentRef>) {
    let mut sections: Vec<&str> = Vec::new();
    let mut attachments = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text { text } => sections.push(text),
            ContentPart::Image { attachment } | ContentPart::File { attachment } => {
                attachments.push(attachment.clone());
            }
            // Rejected by validation above; unreachable by construction.
            ContentPart::ToolCall(_)
            | ContentPart::ToolResult(_)
            | ContentPart::Reasoning { .. } => {}
        }
    }
    (sections.join("\n\n"), attachments)
}

/// One accepted seeding: the transcript rows plus the durable tool-result
/// records (full outcome bytes keyed by their seeded result refs) the
/// backend must persist beside them so `builtin.result_read` paging
/// resolves seeded references exactly like live ones.
pub(crate) struct PreparedSeed {
    pub(crate) rows: Vec<ThreadMessageRecord>,
    pub(crate) tool_result_records: Vec<(String, Vec<u8>)>,
}

/// Deterministic result ref for seeded tool-history row `index`, so a
/// crashed accept retry converges on the same durable record key.
fn seeded_result_ref(thread_id: &ThreadId, index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"prepared-context-tool-result:v1\0");
    hasher.update(thread_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(index.to_le_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("result:seed.{hex}")
}

fn truncated_at_char_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Full outcome bytes + a bounded textual preview for one seeded tool result.
fn seeded_outcome_bytes_and_preview(
    outcome: &ToolResultOutcome,
) -> Result<(Vec<u8>, String), SessionThreadError> {
    let text = match outcome {
        ToolResultOutcome::Text { text } => text.clone(),
        ToolResultOutcome::Json { value } => value.to_string(),
        ToolResultOutcome::Artifacts { attachments } => attachments
            .iter()
            .map(|attachment| {
                format!(
                    "[attachment id={id} ({mime})]",
                    id = attachment.id,
                    mime = attachment.mime_type
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let preview = truncated_at_char_boundary(&text, PREPARED_SEED_PREVIEW_MAX_BYTES).to_string();
    Ok((text.into_bytes(), preview))
}

/// A held assistant tool call awaiting its later tool-result message.
struct PendingSeededCall {
    call: ToolCallContent,
    turn_ordinal: usize,
    reasoning: Option<String>,
}

/// Build one seeded tool-result row's envelopes. The provider identity is
/// the host-owned [`PREPARED_SEED_PROVIDER_ID`] sentinel and `signature` is
/// forced `None`: seeded history replays as a faithful tool round through
/// the gateway's sentinel carve-out, but can never impersonate a real
/// provider route's replay artifacts.
fn seeded_tool_result_row_parts(
    pending: PendingSeededCall,
    result: &ironclaw_llm::agent_message::ToolResultContent,
    result_ref: String,
) -> Result<(String, ProviderToolCallReferenceEnvelope, Vec<u8>), SessionThreadError> {
    let (full_bytes, preview) = seeded_outcome_bytes_and_preview(&result.outcome)?;
    let summary_text = format!("seeded tool result ({} bytes)", full_bytes.len());
    // silent-ok: a summary that fails strict safe-summary validation degrades
    // to the fixed redaction-marker label; the real tool output still reaches
    // the model via the result reference / observation.
    let safe_summary = ToolResultSafeSummary::new(summary_text.clone())
        .unwrap_or_else(|_| ToolResultSafeSummary::redacted_tool_result_summary());
    let observation = serde_json::json!({
        "schema_version": 1,
        "status": if result.is_error { "error" } else { "success" },
        "summary": summary_text,
        "detail": {
            "kind": "result_reference",
            "result_ref": result_ref,
            "byte_len": full_bytes.len(),
            "preview": preview,
        },
        "trust": "untrusted_tool_output",
    });
    let envelope = ToolResultReferenceEnvelope::new_best_effort_model_observation(
        result_ref,
        safe_summary,
        Some(observation),
    )
    .map_err(|error| invalid(format!("seeded tool result envelope invalid: {error}")))?;
    let provider_call = ProviderToolCallReferenceEnvelope {
        provider_id: PREPARED_SEED_PROVIDER_ID.to_string(),
        provider_model_id: "seeded".to_string(),
        provider_turn_id: format!("seed_turn.{}", pending.turn_ordinal),
        provider_call_id: result.call_id.clone(),
        provider_tool_name: seeded_provider_tool_name(&pending.call.capability)?,
        capability_id: pending.call.capability.clone(),
        arguments: pending.call.arguments.clone(),
        response_reasoning: pending.reasoning.map(|text| {
            truncated_at_char_boundary(&text, PREPARED_SEED_REASONING_MAX_BYTES).to_string()
        }),
        reasoning: None,
        signature: None,
    };
    provider_call
        .validate()
        .map_err(|error| invalid(format!("seeded provider call metadata invalid: {error}")))?;
    let content = serde_json::to_string(&envelope)
        .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    Ok((content, provider_call, full_bytes))
}

/// Build the seeded transcript rows and durable tool-result records
/// (validation must already have passed). Sequences are left at 0 for the
/// backend to assign in order; ids and timestamps are final.
///
/// Assistant tool-call turns follow the live transcript's storage shape:
/// there is NO persisted assistant tool-call row — the whole round is
/// synthesized at replay from the provider envelope stored on the
/// tool-result row. An assistant message that carries only tool calls
/// therefore seeds no assistant row at all.
pub(crate) fn prepared_seed(
    request: &PreparedContextRequest,
    thread_id: &ThreadId,
    now: DateTime<Utc>,
) -> Result<PreparedSeed, SessionThreadError> {
    let mut rows = Vec::with_capacity(request.messages.len() + 1);
    let mut tool_result_records = Vec::new();
    let mut pending_calls: std::collections::HashMap<String, PendingSeededCall> =
        std::collections::HashMap::new();
    let mut next_turn_ordinal = 0usize;
    let mut index = 0usize;

    fn blank_row(
        thread_id: &ThreadId,
        index: usize,
        now: DateTime<Utc>,
        kind: MessageKind,
        status: MessageStatus,
    ) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: prepared_seed_message_id(thread_id, index),
            thread_id: thread_id.clone(),
            sequence: 0,
            kind,
            status,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: None,
            attachments: Vec::new(),
            redaction_ref: None,
        }
    }

    if !request.system_prompt.is_empty() {
        let mut row = blank_row(
            thread_id,
            index,
            now,
            MessageKind::System,
            MessageStatus::Finalized,
        );
        row.content = Some(request.system_prompt.clone());
        rows.push(row);
        index += 1;
    }
    for message in &request.messages {
        match message.role {
            AgentMessageRole::User => {
                let (content, attachments) = seeded_text_and_attachments(&message.content);
                crate::contract::validate_attachment_refs(&attachments)?;
                let mut row = blank_row(
                    thread_id,
                    index,
                    now,
                    MessageKind::User,
                    MessageStatus::Accepted,
                );
                row.actor_id = Some(request.actor_id.clone());
                row.content = Some(content);
                row.attachments = attachments;
                rows.push(row);
                index += 1;
            }
            AgentMessageRole::Assistant => {
                let (content, attachments) = seeded_text_and_attachments(&message.content);
                crate::contract::validate_attachment_refs(&attachments)?;
                let tool_calls: Vec<&ToolCallContent> = message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::ToolCall(call) => Some(call),
                        _ => None,
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    let turn_ordinal = next_turn_ordinal;
                    next_turn_ordinal += 1;
                    let mut reasoning = message.content.iter().find_map(|part| match part {
                        ContentPart::Reasoning { reasoning } => reasoning.display_text(),
                        _ => None,
                    });
                    for call in &tool_calls {
                        pending_calls.insert(
                            call.call_id.clone(),
                            PendingSeededCall {
                                call: (*call).clone(),
                                turn_ordinal,
                                reasoning: reasoning.take(),
                            },
                        );
                    }
                }
                if !content.is_empty() || !attachments.is_empty() {
                    let mut row = blank_row(
                        thread_id,
                        index,
                        now,
                        MessageKind::Assistant,
                        MessageStatus::Finalized,
                    );
                    row.content = Some(content);
                    row.attachments = attachments;
                    rows.push(row);
                    index += 1;
                }
            }
            AgentMessageRole::Tool => {
                let result = message
                    .content
                    .iter()
                    .find_map(|part| match part {
                        ContentPart::ToolResult(result) => Some(result),
                        _ => None,
                    })
                    .ok_or_else(|| invalid("tool message carries no tool_result part"))?;
                let pending = pending_calls.remove(&result.call_id).ok_or_else(|| {
                    invalid(format!(
                        "tool result {:?} pairs with no seeded tool call",
                        result.call_id
                    ))
                })?;
                let result_ref = seeded_result_ref(thread_id, index);
                let (content, provider_call, full_bytes) =
                    seeded_tool_result_row_parts(pending, result, result_ref.clone())?;
                let mut row = blank_row(
                    thread_id,
                    index,
                    now,
                    MessageKind::ToolResultReference,
                    MessageStatus::Finalized,
                );
                row.tool_result_ref = Some(result_ref.clone());
                row.tool_result_provider_call = Some(provider_call);
                row.content = Some(content);
                rows.push(row);
                index += 1;
                tool_result_records.push((result_ref, full_bytes));
            }
        }
    }
    Ok(PreparedSeed {
        rows,
        tool_result_records,
    })
}

/// Validate a stored record against a replaying request. Returns the replay
/// response or the typed mismatch.
pub(crate) fn replay_prepared_context(
    record: &PreparedContextRecord,
    request: &PreparedContextRequest,
    thread_id: &ThreadId,
) -> Result<AcceptedPreparedContext, SessionThreadError> {
    if record.idempotency_key != request.idempotency_key {
        return Err(SessionThreadError::PreparedContextKeyMismatch {
            thread_id: thread_id.clone(),
        });
    }
    if record.actor_id != request.actor_id {
        return Err(SessionThreadError::IdempotentReplayActorMismatch {
            stored_actor_id: record.actor_id.clone(),
            requested_actor_id: request.actor_id.clone(),
        });
    }
    let accepted_message_ref = AcceptedMessageRef::new(record.accepted_message_ref.clone())
        .map_err(|error| {
            SessionThreadError::Backend(format!(
                "stored prepared context carries an invalid accepted ref: {error}"
            ))
        })?;
    Ok(AcceptedPreparedContext {
        thread_id: thread_id.clone(),
        accepted_message_ref,
        idempotent_replay: true,
    })
}

/// Map a turn scope onto the thread scope its prepared-context record lives
/// under. `None` when the scope cannot name a prepared thread at all: threads
/// physically require an agent axis, and an actor-fallback owner cannot be
/// resolved without an actor.
pub(crate) fn prepared_thread_scope_for_turn_scope(
    scope: &ironclaw_host_api::turn::TurnScope,
) -> Option<ThreadScope> {
    use ironclaw_host_api::turn::TurnThreadOwner;
    let agent_id = scope.agent_id.clone()?;
    let owner_user_id = match &scope.thread_owner {
        TurnThreadOwner::Ownerless => None,
        TurnThreadOwner::ExplicitUser { owner_user_id } => Some(owner_user_id.clone()),
        TurnThreadOwner::ActorFallback => return None,
    };
    Some(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

/// Host-build-side read of an ADMITTED run's journaled declarations, keyed by
/// the run's own thread. Unlike the admission probe below, no ref match is
/// required: the coordinator already pinned the submission to the prepared
/// context, so the thread IS the identity here.
pub async fn read_declarations_for_run_scope(
    thread_service: &dyn crate::SessionThreadService,
    scope: &ironclaw_host_api::turn::TurnScope,
) -> Result<Option<PreparedTurnDeclarations>, SessionThreadError> {
    let Some(thread_scope) = prepared_thread_scope_for_turn_scope(scope) else {
        return Ok(None);
    };
    match thread_service
        .read_prepared_context(&thread_scope, &scope.thread_id)
        .await
    {
        Ok(record) => Ok(record.map(|record| record.declarations)),
        Err(SessionThreadError::UnknownThread { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Admission-side probe over the threads-tier prepared-context record: the
/// one production implementation of
/// [`ironclaw_host_api::prepared_context::PreparedContextSource`], wired
/// into the turn coordinator by composition. `Ok(None)` means "not a
/// prepared context" (admission rejects ref-less submissions on
/// that answer); storage faults surface as `Unavailable`, never as `None`.
pub struct ThreadServicePreparedContextSource {
    thread_service: std::sync::Arc<dyn crate::SessionThreadService>,
}

impl ThreadServicePreparedContextSource {
    pub fn new(thread_service: std::sync::Arc<dyn crate::SessionThreadService>) -> Self {
        Self { thread_service }
    }
}

#[async_trait::async_trait]
impl ironclaw_host_api::prepared_context::PreparedContextSource
    for ThreadServicePreparedContextSource
{
    async fn read_declarations(
        &self,
        scope: &ironclaw_host_api::turn::TurnScope,
        actor: &ironclaw_host_api::turn::TurnActor,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<
        Option<PreparedTurnDeclarations>,
        ironclaw_host_api::prepared_context::PreparedContextReadError,
    > {
        use ironclaw_host_api::prepared_context::PreparedContextReadError;
        use ironclaw_host_api::turn::TurnThreadOwner;

        // Threads physically require an agent axis; a turn scope without one
        // cannot name an unbound-prepared thread.
        let Some(agent_id) = scope.agent_id.clone() else {
            return Ok(None);
        };
        let owner_user_id = match &scope.thread_owner {
            TurnThreadOwner::Ownerless => None,
            TurnThreadOwner::ExplicitUser { owner_user_id } => Some(owner_user_id.clone()),
            TurnThreadOwner::ActorFallback => Some(actor.user_id.clone()),
        };
        let thread_scope = ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id,
            project_id: scope.project_id.clone(),
            owner_user_id,
            mission_id: None,
        };
        match self
            .thread_service
            .read_prepared_context(&thread_scope, &scope.thread_id)
            .await
        {
            Ok(Some(record)) => {
                // The submitted ref must pin the prepared context exactly; a
                // mismatched ref is not an unbound submission.
                if record.accepted_message_ref != accepted_message_ref.as_str() {
                    return Ok(None);
                }
                Ok(Some(record.declarations))
            }
            Ok(None) => Ok(None),
            Err(SessionThreadError::UnknownThread { .. }) => Ok(None),
            Err(error) => Err(PreparedContextReadError::Unavailable {
                reason: error.kind_name().to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{AgentId, TenantId};

    fn scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-unbound-helpers").expect("tenant"),
            agent_id: AgentId::new("agent-unbound-helpers").expect("agent"),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        }
    }

    fn request() -> PreparedContextRequest {
        PreparedContextRequest {
            scope: scope(),
            actor_id: "user-unbound".to_string(),
            system_prompt: "You are a background task.".to_string(),
            messages: vec![AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text("do the thing")],
            }],
            declarations: PreparedTurnDeclarations::default(),
            idempotency_key: "unbound-key-1".to_string(),
            thread_id: ThreadId::new("unbound-test-thread-1").expect("thread id"),
            title: None,
            metadata_json: None,
        }
    }

    #[test]
    fn seed_message_ids_are_deterministic_per_thread_and_index() {
        let thread = ThreadId::new("unbound-test-determinism").expect("thread id");
        let other = ThreadId::new("unbound-test-determinism-2").expect("thread id");
        assert_eq!(
            prepared_seed_message_id(&thread, 0),
            prepared_seed_message_id(&thread, 0)
        );
        assert_ne!(
            prepared_seed_message_id(&thread, 0),
            prepared_seed_message_id(&thread, 1)
        );
        assert_ne!(
            prepared_seed_message_id(&thread, 0),
            prepared_seed_message_id(&other, 0)
        );
    }

    #[test]
    fn seed_rows_map_roles_onto_transcript_kinds_in_order() {
        let mut request = request();
        request.messages.push(AgentMessage {
            role: AgentMessageRole::Assistant,
            content: vec![ContentPart::text("earlier answer")],
        });
        let thread_id = request.thread_id.clone();
        let rows = prepared_seed(&request, &thread_id, Utc::now())
            .expect("rows")
            .rows;

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, MessageKind::System);
        assert_eq!(rows[0].status, MessageStatus::Finalized);
        assert_eq!(
            rows[0].content.as_deref(),
            Some("You are a background task.")
        );
        assert_eq!(rows[1].kind, MessageKind::User);
        assert_eq!(rows[1].status, MessageStatus::Accepted);
        assert_eq!(rows[1].actor_id.as_deref(), Some("user-unbound"));
        assert_eq!(rows[2].kind, MessageKind::Assistant);
        assert_eq!(rows[2].status, MessageStatus::Finalized);
        assert!(rows.iter().all(|row| row.source_binding_id.is_none()));
        assert!(rows.iter().all(|row| row.reply_target_binding_id.is_none()));
    }

    #[test]
    fn validation_rejects_empty_and_tool_bearing_requests() {
        let mut empty = request();
        empty.messages.clear();
        assert!(matches!(
            validate_prepared_context_request(&empty),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));

        let mut blank_key = request();
        blank_key.idempotency_key.clear();
        assert!(matches!(
            validate_prepared_context_request(&blank_key),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));

        // Reasoning without a sibling tool call has no storage slot: reject.
        let mut orphan_reasoning = request();
        orphan_reasoning.messages.push(AgentMessage {
            role: AgentMessageRole::Assistant,
            content: vec![ContentPart::Reasoning {
                reasoning: ironclaw_llm::ReasoningDetails::from_text("thought")
                    .expect("non-empty reasoning"),
            }],
        });
        assert!(matches!(
            validate_prepared_context_request(&orphan_reasoning),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));

        // A tool call id outside the provider token grammar cannot be
        // journaled as replay metadata: reject before any mint.
        let mut bad_call_id = request();
        bad_call_id.messages = tool_round_messages("call id with spaces", "web.search");
        assert!(matches!(
            validate_prepared_context_request(&bad_call_id),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));
    }

    fn tool_round_messages(call_id: &str, capability: &str) -> Vec<AgentMessage> {
        vec![
            AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text("look this up")],
            },
            AgentMessage {
                role: AgentMessageRole::Assistant,
                content: vec![
                    ContentPart::Reasoning {
                        reasoning: ironclaw_llm::ReasoningDetails::from_text("I should search")
                            .expect("non-empty reasoning"),
                    },
                    ContentPart::ToolCall(ironclaw_llm::agent_message::ToolCallContent {
                        call_id: call_id.into(),
                        capability: ironclaw_host_api::ids::CapabilityId::new(capability)
                            .expect("capability"),
                        arguments: serde_json::json!({"query": "release status"}),
                    }),
                ],
            },
            AgentMessage {
                role: AgentMessageRole::Tool,
                content: vec![ContentPart::ToolResult(
                    ironclaw_llm::agent_message::ToolResultContent {
                        call_id: call_id.into(),
                        outcome: ironclaw_llm::agent_message::ToolResultOutcome::Text {
                            text: "the release went great".into(),
                        },
                        is_error: false,
                    },
                )],
            },
            AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text("now classify it")],
            },
        ]
    }

    #[test]
    fn tool_history_seeds_replayable_tool_result_rows() {
        let mut request = request();
        request.messages = tool_round_messages("call_abc123", "web.search");
        let thread_id = request.thread_id.clone();
        validate_prepared_context_request(&request).expect("tool history validates");
        let seed = prepared_seed(&request, &thread_id, Utc::now()).expect("seed");

        // system + user + tool-result + trailing user; the pure tool-call
        // assistant message seeds NO assistant row (live storage shape).
        assert_eq!(seed.rows.len(), 4);
        let tool_row = &seed.rows[2];
        assert_eq!(tool_row.kind, MessageKind::ToolResultReference);
        assert_eq!(tool_row.status, MessageStatus::Finalized);
        let result_ref = tool_row.tool_result_ref.as_deref().expect("result ref");
        assert!(result_ref.starts_with("result:seed."));
        let provider_call = tool_row
            .tool_result_provider_call
            .as_ref()
            .expect("provider call envelope");
        assert_eq!(provider_call.provider_id, PREPARED_SEED_PROVIDER_ID);
        assert_eq!(provider_call.provider_call_id, "call_abc123");
        assert_eq!(provider_call.provider_tool_name.as_str(), "web__search");
        assert_eq!(provider_call.capability_id.as_str(), "web.search");
        assert_eq!(
            provider_call.response_reasoning.as_deref(),
            Some("I should search")
        );
        assert_eq!(
            provider_call.signature, None,
            "signature is host-forced None"
        );
        provider_call.validate().expect("envelope validates");

        // The row content is a strict-parsable result envelope and the full
        // outcome bytes land in the durable record set under the same ref.
        let envelope = ToolResultReferenceEnvelope::from_json_str(
            tool_row.content.as_deref().expect("content"),
        )
        .expect("strict envelope parse");
        assert_eq!(envelope.result_ref, result_ref);
        assert_eq!(seed.tool_result_records.len(), 1);
        assert_eq!(seed.tool_result_records[0].0, result_ref);
        assert_eq!(
            seed.tool_result_records[0].1,
            b"the release went great".to_vec()
        );

        // Determinism: a retry converges on identical refs and rows.
        let again = prepared_seed(&request, &thread_id, Utc::now()).expect("seed again");
        assert_eq!(
            again.rows[2].tool_result_ref, tool_row.tool_result_ref,
            "seeded result refs are deterministic"
        );
    }

    #[test]
    fn seeded_external_tool_provider_name_matches_the_live_lane() {
        // The live external-tool lane (`ironclaw_loop_host::external_tool_capability`)
        // advertises and matches the bare client-declared tool name for a
        // capability id under the `external_tool.` namespace (e.g.
        // `external_tool.lookup` -> tool name `lookup`), never the blanket
        // `.` -> `__` encoding every other capability id gets. Seeded replay
        // history must derive the SAME name or it names a tool no declared
        // external tool matches (mirrors
        // `external_tool_surface_maps_provider_name_to_capability_id` in
        // `ironclaw_loop_host`).
        let mut request = request();
        request.messages = tool_round_messages("call_ext_1", "external_tool.lookup");
        let thread_id = request.thread_id.clone();
        validate_prepared_context_request(&request).expect("tool history validates");
        let seed = prepared_seed(&request, &thread_id, Utc::now()).expect("seed");

        let tool_row = &seed.rows[2];
        let provider_call = tool_row
            .tool_result_provider_call
            .as_ref()
            .expect("provider call envelope");
        assert_eq!(provider_call.capability_id.as_str(), "external_tool.lookup");
        assert_eq!(
            provider_call.provider_tool_name.as_str(),
            "lookup",
            "seeded provider tool name must match the live external-tool lane's \
             bare client-declared name, not a double-underscore encoding"
        );
    }

    /// A deeply-nested-but-tiny schema is JSON's classic CPU/stack-amplification
    /// shape: `[[[...]]]` many levels deep serializes to only a few hundred
    /// bytes, so the byte cap alone would not catch it — the nesting-depth
    /// cap is the check that does. Each wrap adds exactly one level, so
    /// `nested_value(levels)` has `json_value_max_depth == levels + 1`
    /// (the leaf itself sits at depth 1).
    fn nested_value(levels: usize) -> serde_json::Value {
        let mut value = serde_json::json!("leaf");
        for _ in 0..levels {
            value = serde_json::json!([value]);
        }
        value
    }

    #[test]
    fn oversized_schema_is_rejected_by_the_door() {
        let big_enum: Vec<String> = (0..(PREPARED_OUTPUT_SCHEMA_MAX_BYTES / 8))
            .map(|i| format!("v{i:06}"))
            .collect();
        let schema = serde_json::json!({"type": "string", "enum": big_enum});
        assert!(
            serde_json::to_string(&schema).expect("serialize").len()
                > PREPARED_OUTPUT_SCHEMA_MAX_BYTES
        );
        assert!(matches!(
            validate_output_schema(&schema),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));
    }

    #[test]
    fn over_deep_schema_is_rejected_by_the_door() {
        // depth == PREPARED_OUTPUT_SCHEMA_MAX_DEPTH + 1, one past the cap.
        let schema = nested_value(PREPARED_OUTPUT_SCHEMA_MAX_DEPTH);
        assert_eq!(
            json_value_max_depth(&schema),
            PREPARED_OUTPUT_SCHEMA_MAX_DEPTH + 1
        );
        assert!(matches!(
            validate_output_schema(&schema),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));
    }

    #[test]
    fn boundary_size_and_depth_schemas_are_accepted() {
        // Exactly at the depth cap.
        let schema = nested_value(PREPARED_OUTPUT_SCHEMA_MAX_DEPTH - 1);
        assert_eq!(
            json_value_max_depth(&schema),
            PREPARED_OUTPUT_SCHEMA_MAX_DEPTH
        );
        validate_output_schema(&schema).expect("boundary depth accepted");

        // A comfortably small, shallow schema.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"answer": {"type": "string"}},
            "required": ["answer"],
        });
        validate_output_schema(&schema).expect("small schema accepted");

        // Wired through the full request-level door too.
        let mut req = request();
        req.declarations.output = OutputContract::JsonSchema { schema };
        validate_prepared_context_request(&req).expect("request-level door accepts it");
    }

    #[test]
    fn assistant_message_output_contract_skips_schema_bounds() {
        let mut req = request();
        req.declarations.output = OutputContract::AssistantMessage;
        validate_prepared_context_request(&req).expect("assistant message needs no schema");
    }

    #[test]
    fn oversized_declared_schema_is_rejected_at_the_request_door() {
        let big_enum: Vec<String> = (0..(PREPARED_OUTPUT_SCHEMA_MAX_BYTES / 8))
            .map(|i| format!("v{i:06}"))
            .collect();
        let mut req = request();
        req.declarations.output = OutputContract::JsonSchema {
            schema: serde_json::json!({"type": "string", "enum": big_enum}),
        };
        assert!(matches!(
            validate_prepared_context_request(&req),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));
    }

    #[test]
    fn replay_checks_key_and_actor_fail_closed() {
        let request = request();
        let thread_id = request.thread_id.clone();
        let record = PreparedContextRecord {
            schema_version: PREPARED_CONTEXT_RECORD_SCHEMA_VERSION,
            idempotency_key: request.idempotency_key.clone(),
            actor_id: request.actor_id.clone(),
            accepted_message_ref: format!("msg:{}", prepared_seed_message_id(&thread_id, 1)),
            declarations: request.declarations.clone(),
            seeded_message_count: 2,
            created_at: Utc::now(),
        };

        let replay = replay_prepared_context(&record, &request, &thread_id).expect("replay");
        assert!(replay.idempotent_replay);
        assert_eq!(replay.thread_id, thread_id);

        let mut wrong_key = request.clone();
        wrong_key.idempotency_key = "different-key".to_string();
        assert!(matches!(
            replay_prepared_context(&record, &wrong_key, &thread_id),
            Err(SessionThreadError::PreparedContextKeyMismatch { .. })
        ));

        let mut wrong_actor = request;
        wrong_actor.actor_id = "user-other".to_string();
        assert!(matches!(
            replay_prepared_context(&record, &wrong_actor, &thread_id),
            Err(SessionThreadError::IdempotentReplayActorMismatch { .. })
        ));
    }
}
