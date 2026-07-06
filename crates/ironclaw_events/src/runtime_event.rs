use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, INPUT_ENCODE_HUMAN_SUMMARY, InvocationId, ProcessId, ResourceScope,
    RuntimeKind, Timestamp,
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
    CapabilityActivityRequested,
    CapabilityActivitySucceeded,
    CapabilityActivityFailed,
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
    HookDispatched,
    HookDecisionEmitted,
    HookFailed,
}

/// Redacted runtime event payload.
///
/// All optional fields are absent unless meaningful for the event kind.
/// `error_kind` and `error_summary` are constrained by
/// [`sanitize_error_kind`] and [`sanitize_error_summary`] on every wire
/// crossing:
///
/// - the typed failure constructors and [`RuntimeEvent::with_error_summary`]
///   apply sanitization at construction time;
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
    /// Parent run invocation id when this event represents nested activity.
    pub parent_invocation_id: Option<InvocationId>,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    /// Sanitized, host-authored failure summary for display-only projections.
    pub error_summary: Option<String>,
    /// Hex-encoded blake3 hook identity. Present only on hook events.
    pub hook_id: Option<String>,
    /// Closed-vocabulary hook point label (e.g. `before_capability`). Present
    /// on [`RuntimeEventKind::HookDispatched`].
    pub hook_point: Option<String>,
    /// Closed-vocabulary trust class label (e.g. `builtin`, `installed`).
    /// Present on [`RuntimeEventKind::HookDispatched`].
    pub hook_trust_class: Option<String>,
    /// Closed-vocabulary hook decision kind (`allow`, `deny`, `pause_approval`,
    /// `pause_auth`, `pass`, `patch`). Present on
    /// [`RuntimeEventKind::HookDecisionEmitted`].
    pub hook_decision: Option<String>,
    /// Closed-vocabulary hook failure category (e.g. `timeout`, `panic`).
    /// Present on [`RuntimeEventKind::HookFailed`].
    pub hook_failure_category: Option<String>,
    /// Closed-vocabulary failure disposition (`fail_closed`, `fail_isolated`).
    /// Present on [`RuntimeEventKind::HookFailed`].
    pub hook_failure_disposition: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct RuntimeEventWire {
    event_id: RuntimeEventId,
    timestamp: Timestamp,
    kind: RuntimeEventKind,
    scope: ResourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_invocation_id: Option<InvocationId>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_trust_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_failure_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_failure_disposition: Option<String>,
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
            parent_invocation_id: self.parent_invocation_id,
            capability_id: self.capability_id.clone(),
            provider: self.provider.clone(),
            runtime: self.runtime,
            process_id: self.process_id,
            output_bytes: self.output_bytes,
            error_kind: self.error_kind.clone().map(sanitize_error_kind),
            error_summary: self
                .error_summary
                .as_deref()
                .and_then(sanitize_error_summary_str),
            hook_id: self.hook_id.clone().map(sanitize_hook_id),
            hook_point: self.hook_point.clone().map(sanitize_hook_label),
            hook_trust_class: self.hook_trust_class.clone().map(sanitize_hook_label),
            hook_decision: self.hook_decision.clone().map(sanitize_hook_label),
            hook_failure_category: self.hook_failure_category.clone().map(sanitize_hook_label),
            hook_failure_disposition: self
                .hook_failure_disposition
                .clone()
                .map(sanitize_hook_label),
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
        Ok(wire.into_event())
    }
}

#[derive(Deserialize)]
struct TrustedRuntimeEventWire {
    event_id: RuntimeEventId,
    timestamp: Timestamp,
    kind: RuntimeEventKind,
    scope: ResourceScope,
    #[serde(default)]
    parent_invocation_id: Option<InvocationId>,
    capability_id: CapabilityId,
    #[serde(default)]
    provider: Option<ExtensionId>,
    #[serde(default)]
    runtime: Option<TrustedRuntimeKindWire>,
    #[serde(default)]
    process_id: Option<ProcessId>,
    #[serde(default)]
    output_bytes: Option<u64>,
    #[serde(default)]
    error_kind: Option<String>,
    #[serde(default)]
    error_summary: Option<String>,
    #[serde(default)]
    hook_id: Option<String>,
    #[serde(default)]
    hook_point: Option<String>,
    #[serde(default)]
    hook_trust_class: Option<String>,
    #[serde(default)]
    hook_decision: Option<String>,
    #[serde(default)]
    hook_failure_category: Option<String>,
    #[serde(default)]
    hook_failure_disposition: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TrustedRuntimeKindWire {
    Wasm,
    Mcp,
    Script,
    FirstParty,
    System,
}

impl From<TrustedRuntimeKindWire> for RuntimeKind {
    fn from(value: TrustedRuntimeKindWire) -> Self {
        match value {
            TrustedRuntimeKindWire::Wasm => Self::Wasm,
            TrustedRuntimeKindWire::Mcp => Self::Mcp,
            TrustedRuntimeKindWire::Script => Self::Script,
            TrustedRuntimeKindWire::FirstParty => Self::FirstParty,
            TrustedRuntimeKindWire::System => Self::System,
        }
    }
}

impl RuntimeEventWire {
    fn into_event(self) -> RuntimeEvent {
        let error_summary = self
            .error_summary
            .as_deref()
            .and_then(sanitize_error_summary_str);
        RuntimeEvent {
            event_id: self.event_id,
            timestamp: self.timestamp,
            kind: self.kind,
            scope: self.scope,
            parent_invocation_id: self.parent_invocation_id,
            capability_id: self.capability_id,
            provider: self.provider,
            runtime: self.runtime,
            process_id: self.process_id,
            output_bytes: self.output_bytes,
            error_kind: self.error_kind.map(sanitize_error_kind),
            error_summary,
            hook_id: self.hook_id.map(sanitize_hook_id),
            hook_point: self.hook_point.map(sanitize_hook_label),
            hook_trust_class: self.hook_trust_class.map(sanitize_hook_label),
            hook_decision: self.hook_decision.map(sanitize_hook_label),
            hook_failure_category: self.hook_failure_category.map(sanitize_hook_label),
            hook_failure_disposition: self.hook_failure_disposition.map(sanitize_hook_label),
        }
    }
}

impl TrustedRuntimeEventWire {
    fn into_event(self) -> RuntimeEvent {
        let error_summary = self
            .error_summary
            .as_deref()
            .and_then(sanitize_error_summary_str);
        RuntimeEvent {
            event_id: self.event_id,
            timestamp: self.timestamp,
            kind: self.kind,
            scope: self.scope,
            parent_invocation_id: self.parent_invocation_id,
            capability_id: self.capability_id,
            provider: self.provider,
            runtime: self.runtime.map(Into::into),
            process_id: self.process_id,
            output_bytes: self.output_bytes,
            error_kind: self.error_kind.map(sanitize_error_kind),
            error_summary,
            hook_id: self.hook_id.map(sanitize_hook_id),
            hook_point: self.hook_point.map(sanitize_hook_label),
            hook_trust_class: self.hook_trust_class.map(sanitize_hook_label),
            hook_decision: self.hook_decision.map(sanitize_hook_label),
            hook_failure_category: self.hook_failure_category.map(sanitize_hook_label),
            hook_failure_disposition: self.hook_failure_disposition.map(sanitize_hook_label),
        }
    }
}

pub fn deserialize_trusted_runtime_event<'de, D>(deserializer: D) -> Result<RuntimeEvent, D::Error>
where
    D: serde::Deserializer<'de>,
{
    TrustedRuntimeEventWire::deserialize(deserializer).map(TrustedRuntimeEventWire::into_event)
}

pub fn runtime_event_from_trusted_json_slice(
    value: &[u8],
) -> Result<RuntimeEvent, serde_json::Error> {
    serde_json::from_slice::<TrustedRuntimeEventWire>(value)
        .map(TrustedRuntimeEventWire::into_event)
}

pub fn runtime_event_from_trusted_json_str(value: &str) -> Result<RuntimeEvent, serde_json::Error> {
    serde_json::from_str::<TrustedRuntimeEventWire>(value).map(TrustedRuntimeEventWire::into_event)
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
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
            hook_id: None,
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
        })
    }

    fn new(payload: RuntimeEventPayload) -> Self {
        Self {
            event_id: RuntimeEventId::new(),
            timestamp: Utc::now(),
            kind: payload.kind,
            scope: payload.scope,
            parent_invocation_id: None,
            capability_id: payload.capability_id,
            provider: payload.provider,
            runtime: payload.runtime,
            process_id: payload.process_id,
            output_bytes: payload.output_bytes,
            error_kind: payload.error_kind,
            error_summary: None,
            hook_id: payload.hook_id,
            hook_point: payload.hook_point,
            hook_trust_class: payload.hook_trust_class,
            hook_decision: payload.hook_decision,
            hook_failure_category: payload.hook_failure_category,
            hook_failure_disposition: payload.hook_failure_disposition,
        }
    }

    pub fn capability_activity_requested(
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Self {
        Self {
            kind: RuntimeEventKind::CapabilityActivityRequested,
            ..Self::dispatch_requested(scope, capability_id)
        }
    }

    pub fn capability_activity_succeeded(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    ) -> Self {
        Self {
            kind: RuntimeEventKind::CapabilityActivitySucceeded,
            ..Self::dispatch_succeeded(scope, capability_id, provider, runtime, output_bytes)
        }
    }

    pub fn capability_activity_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error_kind: impl Into<String>,
    ) -> Self {
        Self {
            kind: RuntimeEventKind::CapabilityActivityFailed,
            ..Self::dispatch_failed(scope, capability_id, provider, runtime, error_kind)
        }
    }

    pub fn with_error_summary(mut self, summary: impl AsRef<str>) -> Self {
        self.error_summary = sanitize_error_summary(summary);
        self
    }

    /// Construct a [`RuntimeEventKind::HookDispatched`] event.
    ///
    /// `hook_id` is the hex form of the hook's blake3-derived identity. `point`
    /// and `trust_class` are closed-vocabulary labels produced by the hooks
    /// crate's `telemetry` module; values outside the safe label shape are
    /// collapsed to `Unclassified` on every wire crossing.
    pub fn hook_dispatched(
        scope: ResourceScope,
        capability_id: CapabilityId,
        hook_id: impl Into<String>,
        point: impl Into<String>,
        trust_class: impl Into<String>,
        owning_extension: Option<ExtensionId>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::HookDispatched,
            scope,
            capability_id,
            provider: owning_extension,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
            hook_id: Some(sanitize_hook_id(hook_id)),
            hook_point: Some(sanitize_hook_label(point)),
            hook_trust_class: Some(sanitize_hook_label(trust_class)),
            hook_decision: None,
            hook_failure_category: None,
            hook_failure_disposition: None,
        })
    }

    /// Construct a [`RuntimeEventKind::HookDecisionEmitted`] event.
    ///
    /// `decision` must be the closed-vocabulary kind name from
    /// `HookDecisionSummary::kind_name` (`allow`, `deny`, `pause_approval`,
    /// `pause_auth`, `pass`, `patch`).
    pub fn hook_decision_emitted(
        scope: ResourceScope,
        capability_id: CapabilityId,
        hook_id: impl Into<String>,
        decision: impl Into<String>,
        owning_extension: Option<ExtensionId>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::HookDecisionEmitted,
            scope,
            capability_id,
            provider: owning_extension,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
            hook_id: Some(sanitize_hook_id(hook_id)),
            hook_point: None,
            hook_trust_class: None,
            hook_decision: Some(sanitize_hook_label(decision)),
            hook_failure_category: None,
            hook_failure_disposition: None,
        })
    }

    /// Construct a [`RuntimeEventKind::HookFailed`] event.
    pub fn hook_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        hook_id: impl Into<String>,
        category: impl Into<String>,
        disposition: impl Into<String>,
        owning_extension: Option<ExtensionId>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::HookFailed,
            scope,
            capability_id,
            provider: owning_extension,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
            hook_id: Some(sanitize_hook_id(hook_id)),
            hook_point: None,
            hook_trust_class: None,
            hook_decision: None,
            hook_failure_category: Some(sanitize_hook_label(category)),
            hook_failure_disposition: Some(sanitize_hook_label(disposition)),
        })
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
    hook_id: Option<String>,
    hook_point: Option<String>,
    hook_trust_class: Option<String>,
    hook_decision: Option<String>,
    hook_failure_category: Option<String>,
    hook_failure_disposition: Option<String>,
}

/// Stable token written to `RuntimeEvent.error_kind` whenever a caller-supplied
/// value fails redaction.
pub const UNCLASSIFIED_ERROR_KIND: &str = "Unclassified";

const MAX_ERROR_KIND_LEN: usize = 64;
const MAX_ERROR_KIND_SEGMENT_LEN: usize = 24;
const MAX_ERROR_SUMMARY_BYTES: usize = 512;
const REDACTED_ERROR_SUMMARY: &str = "the tool failure details were redacted";
const WORKSPACE_FILE_ERROR_SUMMARY: &str = "can't access your workspace file";

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

pub fn sanitize_error_summary(summary: impl AsRef<str>) -> Option<String> {
    sanitize_error_summary_str(summary.as_ref())
}

fn sanitize_error_summary_str(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == INPUT_ENCODE_HUMAN_SUMMARY {
        return Some(INPUT_ENCODE_HUMAN_SUMMARY.to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_workspace_file_error_summary(trimmed, &lower) {
        return Some(WORKSPACE_FILE_ERROR_SUMMARY.to_string());
    }
    if !is_safe_error_summary(trimmed, &lower) {
        return Some(REDACTED_ERROR_SUMMARY.to_string());
    }
    Some(truncate_error_summary(trimmed))
}

fn is_workspace_file_error_summary(value: &str, lower: &str) -> bool {
    if value == WORKSPACE_FILE_ERROR_SUMMARY {
        return false;
    }
    if contains_internal_workspace_filename(lower) {
        return true;
    }
    let mentions_workspace_file = mentions_workspace_file_context(lower);
    if mentions_workspace_file && contains_filename_like_token(lower) {
        return true;
    }
    mentions_workspace_file
        && (lower.contains("failed")
            || lower.contains("not found")
            || lower.contains("denied")
            || lower.contains("missing")
            || lower.contains("can't access")
            || lower.contains("cannot access"))
}

fn mentions_workspace_file_context(lower: &str) -> bool {
    [
        "workspace file",
        "workspace path",
        "filesystem",
        "file not found",
        "file failed",
        "read_file",
        "write_file",
        "list_dir",
        "path workspace",
    ]
    .iter()
    .any(|phrase| contains_bounded_phrase(lower, phrase))
}

fn contains_bounded_phrase(value: &str, phrase: &str) -> bool {
    value.match_indices(phrase).any(|(start, matched)| {
        let end = start + matched.len();
        is_phrase_boundary(char_before(value, start)) && is_phrase_boundary(char_at(value, end))
    })
}

fn char_before(value: &str, byte_index: usize) -> Option<char> {
    value.get(..byte_index)?.chars().next_back()
}

fn char_at(value: &str, byte_index: usize) -> Option<char> {
    value.get(byte_index..)?.chars().next()
}

fn is_phrase_boundary(character: Option<char>) -> bool {
    match character {
        Some(character) => !character.is_ascii_alphanumeric() && character != '_',
        None => true,
    }
}

fn contains_internal_workspace_filename(lower: &str) -> bool {
    [
        ".system",
        "agents.md",
        "bootstrap.md",
        "heartbeat.md",
        "identity.md",
        "memory.md",
        "soul.md",
        "tools.md",
        "user.md",
    ]
    .iter()
    .any(|forbidden| lower.contains(forbidden))
}

fn contains_filename_like_token(lower: &str) -> bool {
    lower
        .split(|character: char| character.is_whitespace())
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '(' | ')' | ',' | ':' | ';' | '[' | ']' | '{' | '}'
                )
            })
        })
        .any(is_filename_like_token)
}

fn is_filename_like_token(token: &str) -> bool {
    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty() || extension.is_empty() || extension.len() > 8 {
        return false;
    }
    if !is_common_filename_extension(extension) {
        return false;
    }
    stem.chars()
        .any(|character| character.is_ascii_alphanumeric())
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn is_common_filename_extension(extension: &str) -> bool {
    matches!(
        extension,
        "bash"
            | "css"
            | "csv"
            | "env"
            | "html"
            | "js"
            | "json"
            | "jsonl"
            | "lock"
            | "log"
            | "md"
            | "py"
            | "rs"
            | "sh"
            | "toml"
            | "ts"
            | "txt"
            | "yaml"
            | "yml"
    )
}

fn is_safe_error_summary(value: &str, lower: &str) -> bool {
    if value.chars().any(|character| {
        character == '\0'
            || character.is_control()
            || !character.is_ascii()
            || matches!(
                character,
                '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
            )
    }) {
        return false;
    }
    for forbidden in [
        "access token",
        "api key",
        "api_key",
        "apikey",
        "authorization:",
        "bearer ",
        "password",
        "passwd",
        "provider error",
        "raw runtime",
        "secret",
        "stack trace",
        "traceback",
    ] {
        if lower.contains(forbidden) {
            return false;
        }
    }
    !contains_secret_like_token(lower)
}

fn contains_secret_like_token(lower: &str) -> bool {
    lower
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .any(is_secret_like_token)
}

fn is_secret_like_token(token: &str) -> bool {
    [
        "sk-",
        "sk-ant-",
        "ghp_",
        "github_pat_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "glpat-",
        "gcp-",
        "ya29.",
        "aiza",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix))
        || (token.len() >= 16 && (token.starts_with("akia") || token.starts_with("asia")))
}

fn truncate_error_summary(value: &str) -> String {
    const ELLIPSIS: &str = "...";
    if value.len() <= MAX_ERROR_SUMMARY_BYTES {
        return value.to_string();
    }
    let mut end = MAX_ERROR_SUMMARY_BYTES - ELLIPSIS.len();
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], ELLIPSIS)
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

/// Stable token written to hook string fields whenever a caller-supplied
/// value fails the closed-vocabulary shape guard. Distinct from
/// [`UNCLASSIFIED_ERROR_KIND`] only by virtue of being applied to hook
/// telemetry rather than runtime error classification.
pub const UNCLASSIFIED_HOOK_LABEL: &str = "unclassified";

const MAX_HOOK_LABEL_LEN: usize = 48;
const HOOK_ID_LEN: usize = 64;

/// Collapse any hook label (point, trust class, decision kind, failure
/// category, failure disposition) that does not match the stable
/// `lower_snake_case` shape into the single `unclassified` token. This is the
/// redaction guard that keeps free-form text out of durable hook events.
///
/// Accepts only lowercase ASCII letters, digits, and `_`. First character must
/// be a lowercase ASCII letter. Maximum 48 bytes.
pub fn sanitize_hook_label(label: impl Into<String>) -> String {
    let value = label.into();
    if is_safe_hook_label(&value) {
        value
    } else {
        UNCLASSIFIED_HOOK_LABEL.to_string()
    }
}

fn is_safe_hook_label(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_HOOK_LABEL_LEN {
        return false;
    }
    let first = value.as_bytes()[0];
    if !first.is_ascii_lowercase() {
        return false;
    }
    value.bytes().all(is_error_kind_char)
}

/// Collapse any hook identity string that does not match the stable
/// blake3-hex shape (exactly 64 lowercase hex characters) into the
/// [`UNCLASSIFIED_HOOK_LABEL`] token. The hex form is produced by
/// `ironclaw_hooks::HookId::to_hex`; values of any other shape are rejected so
/// that durable hook events cannot smuggle arbitrary strings through the
/// `hook_id` slot.
pub fn sanitize_hook_id(hook_id: impl Into<String>) -> String {
    let value = hook_id.into();
    if is_safe_hook_id(&value) {
        value
    } else {
        UNCLASSIFIED_HOOK_LABEL.to_string()
    }
}

fn is_safe_hook_id(value: &str) -> bool {
    value.len() == HOOK_ID_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AgentId, INPUT_ENCODE_HUMAN_SUMMARY, InvocationId, ProjectId, TenantId, UserId,
    };

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-hook").unwrap(),
            user_id: UserId::new("user-hook").unwrap(),
            agent_id: Some(AgentId::new("agent-hook").unwrap()),
            project_id: Some(ProjectId::new("project-hook").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn capability() -> CapabilityId {
        CapabilityId::new("hook.dispatch").unwrap()
    }

    fn hook_id_hex() -> String {
        // 64-char lowercase hex matching the blake3 hook id shape produced by
        // `ironclaw_hooks::HookId::to_hex`.
        "0123456789abcdef".repeat(4)
    }

    #[test]
    fn runtime_event_error_summary_round_trips_with_redaction() {
        let event = RuntimeEvent::capability_activity_failed(
            scope(),
            capability(),
            None,
            None,
            "operation_failed",
        )
        .with_error_summary(
            "read_file failed for path workspace ironclaw_issues.json: file not found",
        );
        let wire = serde_json::to_string(&event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some(WORKSPACE_FILE_ERROR_SUMMARY)
        );

        let tool_input_event = RuntimeEvent::capability_activity_failed(
            scope(),
            capability(),
            None,
            None,
            "invalid_input",
        )
        .with_error_summary(INPUT_ENCODE_HUMAN_SUMMARY);
        let wire = serde_json::to_string(&tool_input_event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some(INPUT_ENCODE_HUMAN_SUMMARY)
        );

        let non_filesystem_identifier_event = RuntimeEvent::capability_activity_failed(
            scope(),
            CapabilityId::new("builtin.json").unwrap(),
            None,
            None,
            "invalid_input",
        )
        .with_error_summary("builtin.json returned invalid input");
        let wire = serde_json::to_string(&non_filesystem_identifier_event)
            .expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some("builtin.json returned invalid input")
        );

        let non_filesystem_suffix_event = RuntimeEvent::capability_activity_failed(
            scope(),
            CapabilityId::new("builtin.json").unwrap(),
            None,
            None,
            "invalid_input",
        )
        .with_error_summary("profile not found for builtin.json");
        let wire =
            serde_json::to_string(&non_filesystem_suffix_event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some("profile not found for builtin.json")
        );

        let internal_filename_event = RuntimeEvent::capability_activity_failed(
            scope(),
            capability(),
            None,
            None,
            "operation_failed",
        )
        .with_error_summary("failed to read AGENTS.md");
        let wire =
            serde_json::to_string(&internal_filename_event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some(WORKSPACE_FILE_ERROR_SUMMARY)
        );

        let path_event = RuntimeEvent::capability_activity_failed(
            scope(),
            capability(),
            None,
            None,
            "operation_failed",
        )
        .with_error_summary("read_file failed for /tmp/api_key.txt: secret leaked");
        let wire = serde_json::to_string(&path_event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some(WORKSPACE_FILE_ERROR_SUMMARY)
        );

        let unsafe_event = RuntimeEvent::capability_activity_failed(
            scope(),
            capability(),
            None,
            None,
            "operation_failed",
        )
        .with_error_summary("provider error: bearer token leaked");
        let wire = serde_json::to_string(&unsafe_event).expect("serialize runtime event");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize runtime event");
        assert_eq!(
            decoded.error_summary.as_deref(),
            Some(REDACTED_ERROR_SUMMARY)
        );

        for secret_like_summary in [
            "provider returned AKIAIOSFODNN7EXAMPLE",
            "provider returned ASIAIOSFODNN7EXAMPLE",
            "provider returned gcp-live-secret",
            "provider returned sk-ant-live-secret",
            "provider returned ghp_live_secret",
            "provider returned github_pat_live_secret",
            "provider returned ya29.live-secret",
        ] {
            let event = RuntimeEvent::capability_activity_failed(
                scope(),
                capability(),
                None,
                None,
                "operation_failed",
            )
            .with_error_summary(secret_like_summary);
            let wire = serde_json::to_string(&event).expect("serialize runtime event");
            let decoded: RuntimeEvent =
                serde_json::from_str(&wire).expect("deserialize runtime event");
            assert_eq!(
                decoded.error_summary.as_deref(),
                Some(REDACTED_ERROR_SUMMARY),
                "summary must be redacted: {secret_like_summary}"
            );
        }
    }

    #[test]
    fn truncate_error_summary_preserves_utf8_boundaries() {
        let summary = "a".repeat(MAX_ERROR_SUMMARY_BYTES - 4) + "ééé";
        let truncated = truncate_error_summary(&summary);

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.len() <= MAX_ERROR_SUMMARY_BYTES);
    }

    #[test]
    fn hook_dispatched_round_trips_through_serde() {
        let event = RuntimeEvent::hook_dispatched(
            scope(),
            capability(),
            hook_id_hex(),
            "before_capability",
            "builtin",
            None,
        );
        let wire = serde_json::to_string(&event).expect("serialize hook dispatched");
        let decoded: RuntimeEvent =
            serde_json::from_str(&wire).expect("deserialize hook dispatched");
        assert_eq!(decoded, event);
        assert_eq!(decoded.kind, RuntimeEventKind::HookDispatched);
        assert_eq!(decoded.hook_id.as_deref(), Some(hook_id_hex().as_str()));
        assert_eq!(decoded.hook_point.as_deref(), Some("before_capability"));
        assert_eq!(decoded.hook_trust_class.as_deref(), Some("builtin"));
        assert!(decoded.hook_decision.is_none());
        assert!(decoded.hook_failure_category.is_none());
        assert!(decoded.hook_failure_disposition.is_none());
    }

    #[test]
    fn hook_decision_emitted_round_trips_through_serde() {
        let event = RuntimeEvent::hook_decision_emitted(
            scope(),
            capability(),
            hook_id_hex(),
            "pause_approval",
            None,
        );
        let wire = serde_json::to_string(&event).expect("serialize hook decision");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize hook decision");
        assert_eq!(decoded, event);
        assert_eq!(decoded.kind, RuntimeEventKind::HookDecisionEmitted);
        assert_eq!(decoded.hook_decision.as_deref(), Some("pause_approval"));
        assert_eq!(decoded.hook_id.as_deref(), Some(hook_id_hex().as_str()));
    }

    #[test]
    fn hook_failed_round_trips_through_serde() {
        let event = RuntimeEvent::hook_failed(
            scope(),
            capability(),
            hook_id_hex(),
            "timeout",
            "fail_closed",
            None,
        );
        let wire = serde_json::to_string(&event).expect("serialize hook failed");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize hook failed");
        assert_eq!(decoded, event);
        assert_eq!(decoded.kind, RuntimeEventKind::HookFailed);
        assert_eq!(decoded.hook_failure_category.as_deref(), Some("timeout"));
        assert_eq!(
            decoded.hook_failure_disposition.as_deref(),
            Some("fail_closed")
        );
    }

    /// PR #3640 finding D10: the round-trip tests above pass `None` for the
    /// `owning_extension` argument so they never exercise the `provider`
    /// projection on hook-meta events. This test pins the property that
    /// when an `owning_extension` is supplied, it appears on `event.provider`
    /// (and survives serde) for each of the three hook-meta event kinds —
    /// the lookup that scope filtering depends on.
    #[test]
    fn hook_meta_events_round_trip_owning_extension_as_provider() {
        let owner = ExtensionId::new("ext.polymarket").expect("valid extension id");

        let dispatched = RuntimeEvent::hook_dispatched(
            scope(),
            capability(),
            hook_id_hex(),
            "before_capability",
            "installed",
            Some(owner.clone()),
        );
        assert_eq!(dispatched.provider.as_ref(), Some(&owner));
        let decoded: RuntimeEvent =
            serde_json::from_str(&serde_json::to_string(&dispatched).expect("ser")).expect("de");
        assert_eq!(decoded.provider, Some(owner.clone()));

        let decision = RuntimeEvent::hook_decision_emitted(
            scope(),
            capability(),
            hook_id_hex(),
            "deny",
            Some(owner.clone()),
        );
        assert_eq!(decision.provider.as_ref(), Some(&owner));
        let decoded: RuntimeEvent =
            serde_json::from_str(&serde_json::to_string(&decision).expect("ser")).expect("de");
        assert_eq!(decoded.provider, Some(owner.clone()));

        let failed = RuntimeEvent::hook_failed(
            scope(),
            capability(),
            hook_id_hex(),
            "timeout",
            "fail_isolated",
            Some(owner.clone()),
        );
        assert_eq!(failed.provider.as_ref(), Some(&owner));
        let decoded: RuntimeEvent =
            serde_json::from_str(&serde_json::to_string(&failed).expect("ser")).expect("de");
        assert_eq!(decoded.provider, Some(owner));
    }

    #[test]
    fn runtime_event_deserializes_host_written_privileged_runtime_kind() {
        let mut event = RuntimeEvent::dispatch_succeeded(
            scope(),
            capability(),
            ExtensionId::new("builtin").expect("valid extension id"),
            RuntimeKind::FirstParty,
            0,
        );
        event.runtime = Some(RuntimeKind::System);

        for runtime in ["first_party", "system"] {
            let mut wire =
                serde_json::to_value(&event).expect("runtime event should serialize to json");
            wire["runtime"] = serde_json::Value::String(runtime.to_string());

            assert!(
                serde_json::from_value::<RuntimeEvent>(wire.clone()).is_err(),
                "untrusted runtime event serde must not accept privileged runtime kind"
            );
            let decoded =
                runtime_event_from_trusted_json_str(&serde_json::to_string(&wire).unwrap())
                    .expect("trusted runtime event should deserialize");
            assert_eq!(
                decoded.runtime,
                Some(match runtime {
                    "first_party" => RuntimeKind::FirstParty,
                    "system" => RuntimeKind::System,
                    _ => unreachable!("test table only contains privileged runtime kinds"),
                })
            );
        }
    }

    #[test]
    fn trusted_runtime_event_rejects_unknown_runtime_kind_from_json() {
        let event = RuntimeEvent::dispatch_succeeded(
            scope(),
            capability(),
            ExtensionId::new("builtin").expect("valid extension id"),
            RuntimeKind::Script,
            0,
        );
        let mut wire =
            serde_json::to_value(&event).expect("runtime event should serialize to json");
        wire["runtime"] = serde_json::Value::String("admin".to_string());

        assert!(
            runtime_event_from_trusted_json_str(&serde_json::to_string(&wire).unwrap()).is_err()
        );
    }

    #[test]
    fn hook_label_outside_safe_shape_collapses_to_unclassified() {
        let event = RuntimeEvent::hook_dispatched(
            scope(),
            capability(),
            // not 64 hex chars
            "not-a-hook-id",
            // not lower_snake_case
            "Before Capability",
            "trusted",
            None,
        );
        let wire = serde_json::to_string(&event).expect("serialize");
        let decoded: RuntimeEvent = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(decoded.hook_id.as_deref(), Some(UNCLASSIFIED_HOOK_LABEL));
        assert_eq!(decoded.hook_point.as_deref(), Some(UNCLASSIFIED_HOOK_LABEL));
        assert_eq!(decoded.hook_trust_class.as_deref(), Some("trusted"));
        assert!(
            !wire.contains("not-a-hook-id"),
            "raw unsafe hook id leaked into wire payload: {wire}"
        );
        assert!(
            !wire.contains("Before Capability"),
            "raw unsafe hook point label leaked into wire payload: {wire}"
        );
    }
}
