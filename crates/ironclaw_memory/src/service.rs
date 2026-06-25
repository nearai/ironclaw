//! IronClaw memory service contract for Reborn.
//!
//! This module owns the provider-neutral, host-facing IronClaw memory
//! operation shapes and the [`MemoryService`] trait. The default native
//! adapter and its storage behavior live in the `ironclaw_memory_native`
//! provider crate.

use std::fmt;

use async_trait::async_trait;
use chrono_tz::Tz;
use ironclaw_host_api::{CorrelationId, ResourceScope};
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
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub replace_all: bool,
    pub metadata: Option<DocumentMetadata>,
    pub timezone: Option<String>,
}

impl MemoryServiceWriteRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        // Lenient parsing matching the pre-lift host `parse_write_command`:
        // a present-but-wrong-typed `target` is rejected, but every other
        // present-but-wrong-typed optional field coerces to its default rather
        // than failing (exact original behavior). `new_string`/`timezone` are
        // only consulted by the native write path when relevant (patch /
        // daily_log), preserving origin semantics.
        let target = match input.get("target") {
            Some(Value::String(target)) => target.to_string(),
            Some(_) => return Err(MemoryServiceError::input()),
            None => "daily_log".to_string(),
        };
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
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

/// Serializes to exactly `"cleared"` / `"written"` / `"patched"` via serde
/// snake_case, preserving the historical wire format that previously lived in
/// a `String` status field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteStatus {
    Cleared,
    Written,
    Patched,
}

impl MemoryWriteStatus {
    /// The stable wire string for this status (matches serde snake_case output).
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            MemoryWriteStatus::Cleared => "cleared",
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

#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn search(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceSearchRequest,
    ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn write(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceWriteRequest,
    ) -> Result<MemoryServiceWriteResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn read(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceReadRequest,
    ) -> Result<MemoryServiceReadResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn tree(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceTreeRequest,
    ) -> Result<MemoryServiceTreeResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn profile_set(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceProfileSetRequest,
    ) -> Result<MemoryServiceProfileSetResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    /// Read the run owner's profile document. Provider-neutral counterpart of
    /// [`profile_set`](MemoryService::profile_set): the provider owns the
    /// scope/path resolution (single home shared with the write path) and returns
    /// raw bytes; the host parses + size-caps them.
    async fn profile_read(
        &self,
        invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        let _ = invocation;
        Err(MemoryServiceError::unavailable())
    }

    async fn retrieve_context(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }
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
