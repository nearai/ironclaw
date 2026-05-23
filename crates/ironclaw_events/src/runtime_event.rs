use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ProcessId, ResourceScope, RuntimeKind, Timestamp,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Runtime event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeEventId(Uuid);

impl RuntimeEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for RuntimeEventId {
    fn default() -> Self {
        Self::new()
    }
}

/// Event kinds emitted by the composition/runtime path.
///
/// Approval-specific event kinds are deliberately absent. Approval resolution
/// is a control-plane concern and is recorded as
/// [`AuditEnvelope`] with `AuditStage::ApprovalResolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ModelStarted,
    ModelCompleted,
    ModelFailed,
    AssistantReplyFinalized,
    LoopCompleted,
    LoopCancelled,
    LoopFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
}

/// Redacted runtime event payload.
///
/// All optional fields are absent unless meaningful for the event kind.
/// `error_kind` is constrained by [`sanitize_error_kind`] on every wire
/// crossing:
///
/// - the typed `dispatch_failed` / `model_failed` / `loop_failed` /
///   `process_failed` constructors apply sanitization at construction time;
/// - the custom [`Deserialize`] impl re-runs the sanitizer on any inbound
///   JSONL/wire payload;
/// - the custom [`Serialize`] impl re-runs the sanitizer before emitting the
///   wire payload, so an in-process caller that builds the struct directly
///   (`RuntimeEvent { error_kind: Some(raw), .. }`) still cannot smuggle raw
///   error text, paths, or token-shaped secrets through any
///   `serde_json::to_*` / durable-log `append` path.
///
/// The struct's fields remain `pub` for ergonomic in-memory inspection, but
/// the redaction invariant is enforced wherever the value crosses an I/O
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEvent {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: RuntimeEventKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct RuntimeEventWire {
    event_id: RuntimeEventId,
    timestamp: Timestamp,
    kind: RuntimeEventKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<RuntimeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_id: Option<ProcessId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
}

impl Serialize for RuntimeEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Re-run the redaction guard on the way out. This is the symmetric
        // partner to the Deserialize hook below; together they enforce that
        // `error_kind` is sanitized on every wire crossing regardless of
        // which constructor or direct field assignment produced the value.
        let wire = RuntimeEventWire {
            event_id: self.event_id,
            timestamp: self.timestamp,
            kind: self.kind,
            scope: self.scope.clone(),
            capability_id: self.capability_id.clone(),
            provider: self.provider.clone(),
            runtime: self.runtime,
            process_id: self.process_id,
            output_bytes: self.output_bytes,
            error_kind: self.error_kind.clone().map(sanitize_error_kind),
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RuntimeEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RuntimeEventWire::deserialize(deserializer)?;
        Ok(Self {
            event_id: wire.event_id,
            timestamp: wire.timestamp,
            kind: wire.kind,
            scope: wire.scope,
            capability_id: wire.capability_id,
            provider: wire.provider,
            runtime: wire.runtime,
            process_id: wire.process_id,
            output_bytes: wire.output_bytes,
            error_kind: wire.error_kind.map(sanitize_error_kind),
        })
    }
}

impl RuntimeEvent {
    pub fn dispatch_requested(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchRequested,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn runtime_selected(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::RuntimeSelected,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn dispatch_succeeded(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchSucceeded,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: Some(output_bytes),
            error_kind: None,
        })
    }

    pub fn dispatch_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchFailed,
            scope,
            capability_id,
            provider,
            runtime,
            process_id: None,
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn model_started(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new_metadata_only(RuntimeEventKind::ModelStarted, scope, capability_id)
    }

    pub fn model_completed(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new_metadata_only(RuntimeEventKind::ModelCompleted, scope, capability_id)
    }

    pub fn model_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ModelFailed,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn assistant_reply_finalized(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new_metadata_only(
            RuntimeEventKind::AssistantReplyFinalized,
            scope,
            capability_id,
        )
    }

    pub fn loop_completed(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new_metadata_only(RuntimeEventKind::LoopCompleted, scope, capability_id)
    }

    pub fn loop_cancelled(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new_metadata_only(RuntimeEventKind::LoopCancelled, scope, capability_id)
    }

    pub fn loop_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::LoopFailed,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    fn new_metadata_only(
        kind: RuntimeEventKind,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_started(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessStarted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_completed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessCompleted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessFailed,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn process_killed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessKilled,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    fn new(payload: RuntimeEventPayload) -> Self {
        Self {
            event_id: RuntimeEventId::new(),
            timestamp: Utc::now(),
            kind: payload.kind,
            scope: payload.scope,
            capability_id: payload.capability_id,
            provider: payload.provider,
            runtime: payload.runtime,
            process_id: payload.process_id,
            output_bytes: payload.output_bytes,
            error_kind: payload.error_kind,
        }
    }
}

struct RuntimeEventPayload {
    kind: RuntimeEventKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    provider: Option<ExtensionId>,
    runtime: Option<RuntimeKind>,
    process_id: Option<ProcessId>,
    output_bytes: Option<u64>,
    error_kind: Option<String>,
}

/// Stable token written to `RuntimeEvent.error_kind` whenever a caller-supplied
/// value fails redaction.
pub const UNCLASSIFIED_ERROR_KIND: &str = "Unclassified";

const MAX_ERROR_KIND_LEN: usize = 64;
const MAX_ERROR_KIND_SEGMENT_LEN: usize = 24;

/// Collapse any error_kind value that does not match the stable classification
/// shape into the single `Unclassified` token. This is the redaction guard
/// that keeps raw error messages, paths, and stringified secrets out of
/// durable runtime events.
///
/// Accepts only `lower_snake_case` identifiers with optional `.` or `:`
/// separators (e.g. `missing_runtime_backend`, `wasm.host_http_denied`,
/// `dispatch:timeout`). Rejects anything that resembles a path, free-form
/// error text, JWT, base64 token, or API key:
///
/// - empty string;
/// - longer than 64 bytes overall, or any dot/colon-separated segment longer
///   than 24 bytes (defeats long random tokens);
/// - characters outside `[a-z0-9_]` for body content, or `[._:]` separators;
/// - leading character that is not a lowercase ASCII letter (defeats
///   numeric-prefixed tokens, leading underscores, leading separators).
pub fn sanitize_error_kind(error_kind: impl Into<String>) -> String {
    let value = error_kind.into();
    if is_safe_error_kind(&value) {
        value
    } else {
        UNCLASSIFIED_ERROR_KIND.to_string()
    }
}

fn is_safe_error_kind(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ERROR_KIND_LEN {
        return false;
    }
    let first = value.as_bytes()[0];
    if !first.is_ascii_lowercase() {
        return false;
    }
    if value
        .bytes()
        .any(|byte| !is_error_kind_char(byte) && !matches!(byte, b'.' | b':'))
    {
        return false;
    }
    for segment in value.split(['.', ':']) {
        if segment.is_empty() || segment.len() > MAX_ERROR_KIND_SEGMENT_LEN {
            return false;
        }
        let segment_first = segment.as_bytes()[0];
        if !segment_first.is_ascii_lowercase() {
            return false;
        }
    }
    true
}

fn is_error_kind_char(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
}
