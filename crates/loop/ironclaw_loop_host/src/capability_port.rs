use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalStoreError, GateRecordStorePort};
use ironclaw_capabilities::{ReplayPayload, ReplayPayloadStoreError, ReplayPayloadStorePort};
use ironclaw_host_api::{
    approval::sha256_digest_token,
    capability::{CapabilitySet, EffectKind},
    capability_surface::CapabilitySurfacePolicy,
    dispatch::{
        CapabilityDisplayOutputPreview, DispatchFailureDetail, DispatchInputIssue,
        DispatchInputIssueCode, RuntimeDispatchErrorKind,
    },
    gate_record::GateRecord,
    ids::{
        ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId, GateRef, InvocationId,
        ProviderToolName,
    },
    invocation::InvocationOrigin,
    mount::MountView,
    resolution::{Resolution, ResolutionBatch},
    resource::{ResourceEstimate, ResourceScope},
    result_meta::{FailureKind, ModelDiagnostic},
    runtime::RuntimeKind,
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{
    CapabilityFailureDisposition, HostRuntime, HostRuntimeError, IdempotencyKey,
    RuntimeBlockedReason, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityFailureDetail,
    CapabilityInputIssue, CapabilityInputRef, CapabilityResumeToken, ContentDigest,
    LoopCapabilityPort, LoopHostMilestone, LoopHostMilestoneKind, LoopHostMilestoneSink,
    LoopProcessRef, LoopRequest, LoopRequestBatch, LoopRunContext, LoopSafeSummary,
    ModelVisibleToolObservation, ProviderToolCall, ProviderToolCallCapabilityIds,
    ProviderToolCallReplay, ProviderToolDefinition, RegisterProviderToolCallRequest,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
    resolution::{self, GatedResolution},
};
use ironclaw_turns::{CapabilityActivityId, LoopGateRef, LoopResultRef};
use serde_json::Value;
use tokio::sync::Notify;

mod provider_input;
mod provider_validation;
mod surface_snapshot;

use self::provider_input::{
    normalize_provider_arguments, prepare_provider_arguments,
    prepare_provider_arguments_with_detail, schema_contains_external_ref,
};
use self::provider_validation::{
    PROVIDER_TOOL_NAME_MAX_BYTES, validate_provider_arguments, validate_provider_tool_call,
};
use self::surface_snapshot::{
    RuntimeSurfaceCapabilitySnapshot, SurfaceCapabilitySnapshot, SurfaceSnapshot,
    SyntheticSurfaceCapabilitySnapshot,
};

// arch-exempt: large_file, host capability adapter + Slice C result-wiring seam, plan #3988
// (decomposition tracker). Synthetic surface snapshot logic already lives in
// `capability_port/surface_snapshot.rs`; the Slice C seam (§5.3) adds the gate-record
// persistence wrapper and its focused tests here to keep the existing adapter boundary.
const PROVIDER_TOOL_NAME_DIGEST_BYTES: usize = 32;
const PROVIDER_TOOL_CALL_INPUT_REF_PREFIX: &str = "input:provider-tool-";

/// Observes a capability invocation's resolved input (arguments) as the host
/// loop executes it, for trajectory capture by downstream consumers (benchmark
/// harnesses, debuggers, UI). `call_id` is the capability input ref.
///
/// The capability port emits input events. Result-writer implementations may
/// emit the matching result event through [`Self::on_capability_result`].
///
/// Best-effort and side-effect-free. The callback fires inline on the
/// per-capability hot path, so an implementation **must never block** (do I/O,
/// contend on a lock): hand the event to a non-blocking queue and return. A
/// callback that panics is caught at the call site and the event is dropped —
/// it cannot unwind or fail the run — but it must not rely on that.
pub trait CapabilityTrajectoryObserver: std::fmt::Debug + Send + Sync {
    /// A model tool call resolved to a capability invocation: `capability_id` is
    /// the resolved capability (e.g. `builtin.shell`), `arguments` the tool-call
    /// input JSON resolved from the input ref. This fires before schema
    /// normalization/coercion, so `arguments` is the raw model-emitted input
    /// (what the trajectory should record), not the post-validation execution
    /// payload.
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    );

    /// A capability completed and staged `output` for the model. The default
    /// keeps existing input-only observers source-compatible.
    fn on_capability_result(
        &self,
        _call_id: &str,
        _capability_id: &str,
        _output: &serde_json::Value,
    ) {
    }
}

#[async_trait]
pub trait LoopCapabilityInputResolver: Send + Sync {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError>;

    async fn register_provider_tool_call_input(
        &self,
        _run_context: &LoopRunContext,
        _tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool-call input registration is not supported",
        ))
    }

    /// Record the display-preview input for a provider tool call under
    /// `input_ref`, keyed for display by the resolved dotted `capability_id`
    /// (e.g. `nearai.web_search`) — NOT the provider tool name
    /// (`nearai__web_search`), which is a lossy, digest-suffixed encoding that
    /// both renders badly and defeats the per-tool summary/subtitle matchers.
    ///
    /// `ProviderToolCallInputResolver` decorates this trait and owns the
    /// canonical (digest-based) `input_ref`; it stages the arguments itself and
    /// does NOT delegate `register_provider_tool_call_input` to the inner
    /// resolver, so it forwards this hook to `inner` instead. The caller
    /// (`register_provider_tool_call`) drives it after registration because that
    /// is where the resolved `capability_id` and the canonical `input_ref` are
    /// both in hand. Default no-op: only resolvers that own a display-preview
    /// store implement it.
    fn record_provider_tool_call_display_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
        _capability_id: &CapabilityId,
        _tool_call: &ProviderToolCall,
    ) {
    }
}

struct ProviderToolCallInputResolver {
    inner: Arc<dyn LoopCapabilityInputResolver>,
    provider_inputs: Mutex<HashMap<String, serde_json::Value>>,
}

impl ProviderToolCallInputResolver {
    fn new(inner: Arc<dyn LoopCapabilityInputResolver>) -> Self {
        Self {
            inner,
            provider_inputs: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for ProviderToolCallInputResolver {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        if let Some(input) = self
            .provider_inputs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "provider tool-call input store is unavailable",
                )
            })?
            .get(input_ref.as_str())
            .cloned()
        {
            return Ok(input);
        }
        self.inner
            .resolve_capability_input(run_context, input_ref)
            .await
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref = provider_tool_call_input_ref(run_context, tool_call)?;
        let mut provider_inputs = self.provider_inputs.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "provider tool-call input store is unavailable",
            )
        })?;
        if let Some(existing) = provider_inputs.get(input_ref.as_str()) {
            if existing != &tool_call.arguments {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "provider tool-call input ref collision",
                ));
            }
        } else {
            provider_inputs.insert(input_ref.as_str().to_string(), tool_call.arguments.clone());
        }
        Ok(input_ref)
    }

    fn record_provider_tool_call_display_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
        capability_id: &CapabilityId,
        tool_call: &ProviderToolCall,
    ) {
        // This decorator bypasses the inner `register_provider_tool_call_input`,
        // so forward the display-recording side effect to `inner` (the resolver
        // that owns the display-preview store).
        self.inner.record_provider_tool_call_display_input(
            run_context,
            input_ref,
            capability_id,
            tool_call,
        );
    }
}

#[async_trait]
pub trait LoopCapabilityResultWriter: Send + Sync {
    /// Write the result of a completed capability invocation.
    ///
    /// Returns metadata for the staged output: the result ref, serialized byte
    /// length for per-capability byte accounting, and an optional normalized
    /// content digest for future output-aware progress detection.
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError>;

    async fn update_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &LoopResultRef,
        _output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability result updates are not supported by this writer",
        ))
    }

    async fn delete_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    /// Note that the invocation `invocation_id` has started executing with the
    /// input staged under `input_ref`. Links the two so the still-running
    /// activity frame can surface the input (inline argument + parameters)
    /// before the result lands — the input was recorded under `input_ref` at
    /// registration, but the activity projection only knows the `invocation_id`.
    /// Default no-op: only writers that own a display-preview store implement it.
    fn record_running_invocation(
        &self,
        _run_context: &LoopRunContext,
        _invocation_id: InvocationId,
        _input_ref: &CapabilityInputRef,
    ) {
    }

    /// Stage a display preview for a FAILED capability invocation so the UI can
    /// render the specific failure detail (e.g. invalid-input field issues)
    /// instead of only the bare error kind. `summary` is a bounded,
    /// host-authored string (see `capability_failure_display_summary`).
    /// Default no-op: only writers that own a display-preview store implement
    /// it. Async so implementers can durably persist the failure preview the
    /// same way `write_capability_result` persists success previews.
    async fn stage_capability_failure_preview(
        &self,
        _run_context: &LoopRunContext,
        _invocation_id: InvocationId,
        _capability_id: &CapabilityId,
        _summary: &str,
    ) {
    }
}

/// Maximum number of input issues rendered into a failure display preview.
const CAPABILITY_FAILURE_PREVIEW_MAX_ISSUES: usize = 5;
/// Byte budget for the rendered failure summary. Stays well under the display
/// preview's own `CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES` (2 KiB) cap.
const CAPABILITY_FAILURE_PREVIEW_MAX_BYTES: usize = 1024;

/// Generic placeholder summaries assigned when a failure carries no
/// host-authored message. Surfacing these adds nothing over the bare error
/// kind, so they are filtered out (`runtime_failure_to_loop` /
/// `runtime_model_visible_failure_to_loop`).
const GENERIC_CAPABILITY_FAILURE_SUMMARIES: [&str; 2] = [
    "capability invocation failed",
    "capability authorization denied",
];

/// Render a bounded, host-authored display summary for a failed capability so
/// the per-tool UI preview shows the actual reason instead of the bare error
/// kind.
///
/// Preference order:
/// 1. Structured `InvalidInput` field issues, when present — these carry the
///    most actionable per-field detail. Only schema-derived fields (`path`,
///    `code`, `expected`) are interpolated; `received` echoes raw tool input
///    and is deliberately omitted from any display surface.
/// 2. Otherwise the failure's host-authored `safe_summary` (e.g. a builtin's
///    `"invalid JSON: ..."` message), unless it is one of the generic
///    placeholders that say nothing the kind doesn't.
///
/// Returns `None` when neither is available, so the projection keeps its
/// existing `tool failed: <kind>` fallback.
fn failure_display_summary(safe_summary: &str, detail: &CapabilityFailureDetail) -> Option<String> {
    if let CapabilityFailureDetail::InvalidInput { issues } = detail
        && !issues.is_empty()
    {
        let rendered = issues
            .iter()
            .take(CAPABILITY_FAILURE_PREVIEW_MAX_ISSUES)
            .filter_map(render_capability_input_issue)
            .collect::<Vec<_>>()
            .join("; ");
        if !rendered.is_empty() {
            let mut summary = format!("Invalid input: {rendered}");
            if issues.len() > CAPABILITY_FAILURE_PREVIEW_MAX_ISSUES {
                let extra = issues.len() - CAPABILITY_FAILURE_PREVIEW_MAX_ISSUES;
                summary.push_str(&format!(" (+{extra} more)"));
            }
            return Some(
                ironclaw_host_api::dispatch::truncate_capability_display_text(
                    &summary,
                    CAPABILITY_FAILURE_PREVIEW_MAX_BYTES,
                )
                .text,
            );
        }
    }

    let summary = safe_summary.trim();
    if summary.is_empty() || GENERIC_CAPABILITY_FAILURE_SUMMARIES.contains(&summary) {
        return None;
    }
    Some(
        ironclaw_host_api::dispatch::truncate_capability_display_text(
            summary,
            CAPABILITY_FAILURE_PREVIEW_MAX_BYTES,
        )
        .text,
    )
}

const CAPABILITY_INPUT_ISSUE_FIELD_MAX_BYTES: usize = 160;

fn render_capability_input_issue(issue: &CapabilityInputIssue) -> Option<String> {
    let code = match issue.code {
        DispatchInputIssueCode::MissingRequired => "missing required field",
        DispatchInputIssueCode::UnexpectedField => "unexpected field",
        DispatchInputIssueCode::TypeMismatch => "type mismatch",
        DispatchInputIssueCode::InvalidValue => "invalid value",
    };
    let path = capability_input_issue_display_text(&issue.path)?;
    match issue
        .expected
        .as_deref()
        .and_then(capability_input_issue_display_text)
    {
        Some(expected) if !expected.is_empty() => {
            Some(format!("{path} — {code} (expected {expected})"))
        }
        _ => Some(format!("{path} — {code}")),
    }
}

fn capability_input_issue_display_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().any(|character| {
            character == '\0'
                || character.is_control()
                || !character.is_ascii()
                || matches!(
                    character,
                    '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
                )
        })
        || contains_capability_input_issue_sensitive_marker(trimmed)
    {
        return None;
    }
    Some(
        ironclaw_host_api::dispatch::truncate_capability_display_text(
            trimmed,
            CAPABILITY_INPUT_ISSUE_FIELD_MAX_BYTES,
        )
        .text,
    )
}

fn contains_capability_input_issue_sensitive_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let normalized = lower
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    for forbidden in [
        "accesstoken",
        "apikey",
        "authtoken",
        "authorization",
        "bearer",
        "password",
        "passwd",
        "secret",
        "toolinput",
    ] {
        if normalized.contains(forbidden) {
            return true;
        }
    }
    for forbidden in [
        "access token",
        "access_token",
        "api key",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "passwd",
        "secret",
        "tool input",
        "tool_input",
    ] {
        if lower.contains(forbidden) {
            return true;
        }
    }
    lower
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .any(|token| {
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
        })
}

/// Whether a capability result write must be durably persisted, or the
/// content is already fully delivered to the model inline and only needs
/// best-effort in-memory staging for the current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DurablePersistence {
    /// Durably persist the result content. The default, and correct choice
    /// for any capability result the model has not already seen in full.
    #[default]
    Persist,
    /// Skip durable persistence. Reserved for outputs that are already
    /// fully model-visible inline (e.g. a `result_read` continuation chunk,
    /// whose bytes are returned directly in the tool observation) — writing
    /// them durably again would mint a redundant record per chunk with no
    /// reader that needs it. Best-effort in-memory staging still happens,
    /// so an immediate re-read from cache can still succeed; a later durable
    /// read against this ref must fail gracefully as unavailable.
    InlineOnly,
}

pub struct CapabilityResultWrite<'a> {
    pub run_context: &'a LoopRunContext,
    pub input_ref: &'a CapabilityInputRef,
    pub invocation_id: InvocationId,
    pub capability_id: &'a CapabilityId,
    pub output: serde_json::Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub durable_persistence: DurablePersistence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWriteResult {
    pub result_ref: LoopResultRef,
    pub byte_len: u64,
    pub output_digest: Option<ContentDigest>,
    pub model_observation: Option<ModelVisibleToolObservation>,
}

impl CapabilityWriteResult {
    pub fn without_output_digest(result_ref: LoopResultRef, byte_len: u64) -> Self {
        Self {
            result_ref,
            byte_len,
            output_digest: None,
            model_observation: None,
        }
    }

    pub fn from_output(
        result_ref: LoopResultRef,
        byte_len: u64,
        output: &serde_json::Value,
    ) -> Self {
        // The output digest is a best-effort progress hint (consumed by output-aware
        // no-progress detection in a later change). A failure to compute it must NEVER
        // fail an otherwise-successful capability write — degrade to `None` instead.
        let output_digest = match ContentDigest::from_json_value(output) {
            Ok(digest) => Some(digest),
            Err(error) => {
                tracing::debug!(
                    %error,
                    "capability result output digest could not be built; recording result without it"
                );
                None
            }
        };
        Self {
            model_observation: None,
            result_ref,
            byte_len,
            output_digest,
        }
    }
}

#[async_trait]
pub trait LoopCapabilityPortFactory: Send + Sync {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError>;

    /// Build a port from an already-resolved host-owned model-surface policy.
    ///
    /// The default preserves compatibility for factories whose inner surface
    /// has no host-runtime visibility request. Production host-backed
    /// factories override this method so disclosure, outer filtering, and the
    /// host's visible snapshot all consume the same resolved value.
    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        _surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.create_capability_port(run_context).await
    }
}

pub trait LoopCapabilityPortDecorator: Send + Sync {
    fn decorate(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Arc<dyn LoopCapabilityPort>;
}

pub struct DecoratingLoopCapabilityPortFactory {
    inner: Arc<dyn LoopCapabilityPortFactory>,
    decorators: Vec<Arc<dyn LoopCapabilityPortDecorator>>,
}

impl DecoratingLoopCapabilityPortFactory {
    pub fn new(inner: Arc<dyn LoopCapabilityPortFactory>) -> Self {
        Self {
            inner,
            decorators: Vec::new(),
        }
    }

    pub fn with_decorator(mut self, decorator: Arc<dyn LoopCapabilityPortDecorator>) -> Self {
        self.decorators.push(decorator);
        self
    }
}

#[async_trait]
impl LoopCapabilityPortFactory for DecoratingLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut port = self.inner.create_capability_port(run_context).await?;
        for decorator in &self.decorators {
            port = decorator.decorate(run_context, port);
        }
        Ok(port)
    }

    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut port = self
            .inner
            .create_capability_port_with_surface_policy(run_context, surface_policy)
            .await?;
        for decorator in &self.decorators {
            port = decorator.decorate(run_context, port);
        }
        Ok(port)
    }
}

#[derive(Clone)]
pub struct HostRuntimeLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    execution_mounts: MountView,
    capability_execution_mounts: HashMap<CapabilityId, MountView>,
    trajectory_observer: Option<Arc<dyn CapabilityTrajectoryObserver>>,
    gate_record_store: Arc<dyn GateRecordStorePort>,
    replay_payload_store: Arc<dyn ReplayPayloadStorePort>,
}

impl HostRuntimeLoopCapabilityPortFactory {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            runtime,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink,
            execution_mounts: MountView::default(),
            capability_execution_mounts: HashMap::new(),
            trajectory_observer: None,
            // Transitional no-op default until composition wires the durable
            // store via `with_gate_record_store` (the record has no reader until
            // the resume-read follow-up, so skipping the write is behavior-
            // preserving). See `NoopGateRecordStore`.
            gate_record_store: Arc::new(NoopGateRecordStore),
            // Transitional fail-closed default until composition wires the durable
            // replay-payload store via `with_replay_payload_store`. See
            // `NoopReplayPayloadStore`: an unwired factory persists nothing, so a
            // gate/auth resume that must reconstitute its replay input fails closed
            // (sanitized terminal failure) rather than dispatching empty input.
            replay_payload_store: Arc::new(NoopReplayPayloadStore),
        }
    }

    /// Wire the durable [`GateRecordStorePort`] every port built by this factory
    /// persists pending-gate records into (§5.2.9). Production composition always
    /// calls this; the fail-closed default only guards an unwired factory.
    pub fn with_gate_record_store(mut self, store: Arc<dyn GateRecordStorePort>) -> Self {
        self.gate_record_store = store;
        self
    }

    /// Wire the durable host-private [`ReplayPayloadStorePort`] every port built by
    /// this factory persists gate/auth replay payloads into and reconstitutes
    /// them from on resume (arch-simplification §5.3 Stage 2a-i). Production
    /// composition always calls this; the fail-closed default only guards an
    /// unwired factory.
    pub fn with_replay_payload_store(mut self, store: Arc<dyn ReplayPayloadStorePort>) -> Self {
        self.replay_payload_store = store;
        self
    }

    /// Attach a [`CapabilityTrajectoryObserver`] that every port built by this
    /// factory forwards capability inputs to. No-op when unset.
    pub fn with_trajectory_observer(
        mut self,
        observer: Option<Arc<dyn CapabilityTrajectoryObserver>>,
    ) -> Self {
        self.trajectory_observer = observer;
        self
    }

    pub fn with_execution_mounts(mut self, mounts: MountView) -> Self {
        self.execution_mounts = mounts;
        self
    }

    pub fn with_capability_execution_mount(
        mut self,
        capability_id: CapabilityId,
        mounts: MountView,
    ) -> Self {
        self.capability_execution_mounts
            .insert(capability_id, mounts);
        self
    }

    pub fn for_run_context(&self, run_context: LoopRunContext) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(self.port_for_run_context(run_context))
    }

    fn port_for_run_context(&self, run_context: LoopRunContext) -> HostRuntimeLoopCapabilityPort {
        HostRuntimeLoopCapabilityPort::new(
            Arc::clone(&self.runtime),
            run_context,
            self.visible_request.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_gate_record_store(Arc::clone(&self.gate_record_store))
        .with_replay_payload_store(Arc::clone(&self.replay_payload_store))
        .with_execution_mounts(self.execution_mounts.clone())
        .with_capability_execution_mounts(self.capability_execution_mounts.clone())
        .with_trajectory_observer(self.trajectory_observer.clone())
    }
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(self.for_run_context(run_context.clone()))
    }

    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut factory = self.clone();
        factory.visible_request.policy = surface_policy.as_ref().clone();
        factory.create_capability_port(run_context).await
    }
}

struct PreparedProviderToolCall {
    surface_version: ironclaw_loop_contracts::CapabilitySurfaceVersion,
    capability_id: CapabilityId,
    provider_turn_id: String,
    normalized_arguments: serde_json::Value,
    effective_capability_ids: Vec<CapabilityId>,
}

const MAX_IN_MEMORY_DISPATCH_RECORDS: usize = 128;

#[derive(Clone)]
enum DispatchRecord {
    InFlight {
        notify: Arc<Notify>,
    },
    RuntimeCompleted {
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    TerminalMilestonePending {
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
    },
    LoopCompleted {
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
    },
}

struct RuntimeOutcomeCompletion<'a> {
    input_ref: &'a CapabilityInputRef,
    invocation_id: InvocationId,
    correlation_id: CorrelationId,
    requested_capability_id: &'a CapabilityId,
    provider: ExtensionId,
    runtime: RuntimeKind,
    outcome: RuntimeCapabilityOutcome,
}

struct RuntimeOutcomeConversion<'a> {
    input_ref: &'a CapabilityInputRef,
    invocation_id: InvocationId,
    correlation_id: CorrelationId,
    requested_capability_id: &'a CapabilityId,
    outcome: RuntimeCapabilityOutcome,
}

fn ensure_cached_invocation_matches_activity(
    cached_invocation_id: InvocationId,
    requested_invocation_id: InvocationId,
) -> Result<(), AgentLoopHostError> {
    if cached_invocation_id == requested_invocation_id {
        return Ok(());
    }
    Err(AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "cached capability dispatch activity identity does not match the requested activity",
    ))
}

impl<'a> RuntimeOutcomeCompletion<'a> {
    fn conversion(&self) -> RuntimeOutcomeConversion<'a> {
        RuntimeOutcomeConversion {
            input_ref: self.input_ref,
            invocation_id: self.invocation_id,
            correlation_id: self.correlation_id,
            requested_capability_id: self.requested_capability_id,
            outcome: self.outcome.clone(),
        }
    }
}

#[derive(Default)]
struct DispatchRecordStore {
    records: HashMap<String, DispatchRecord>,
    insertion_order: VecDeque<String>,
}

impl DispatchRecordStore {
    fn reserve(
        &mut self,
        key: &IdempotencyKey,
        requested_invocation_id: InvocationId,
    ) -> Result<DispatchReservation, AgentLoopHostError> {
        let key_value = key.as_str().to_string();
        match self.records.get(key.as_str()).cloned() {
            Some(DispatchRecord::InFlight { notify }) => Ok(DispatchReservation::Wait(notify)),
            Some(DispatchRecord::RuntimeCompleted {
                invocation_id,
                correlation_id,
                requested_capability_id,
                outcome,
            }) => {
                ensure_cached_invocation_matches_activity(invocation_id, requested_invocation_id)?;
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::RuntimeCompleted {
                    invocation_id,
                    correlation_id,
                    requested_capability_id,
                    outcome,
                })
            }
            Some(DispatchRecord::TerminalMilestonePending {
                invocation_id,
                result,
                milestone,
            }) => {
                ensure_cached_invocation_matches_activity(invocation_id, requested_invocation_id)?;
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::TerminalMilestonePending {
                    invocation_id,
                    result,
                    milestone,
                })
            }
            Some(DispatchRecord::LoopCompleted {
                invocation_id,
                result,
            }) => {
                ensure_cached_invocation_matches_activity(invocation_id, requested_invocation_id)?;
                Ok(DispatchReservation::LoopCompleted(result))
            }
            None => {
                self.evict_completed_until_below_limit()?;
                self.insertion_order.push_back(key_value.clone());
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::Reserved)
            }
        }
    }

    fn record(&mut self, key: &IdempotencyKey, record: DispatchRecord) -> Option<Arc<Notify>> {
        let previous = self.records.insert(key.as_str().to_string(), record);
        match previous {
            Some(DispatchRecord::InFlight { notify }) => Some(notify),
            _ => None,
        }
    }

    fn remove(&mut self, key: &IdempotencyKey) -> Option<Arc<Notify>> {
        let removed = self.records.remove(key.as_str());
        self.insertion_order
            .retain(|candidate| candidate != key.as_str());
        match removed {
            Some(DispatchRecord::InFlight { notify }) => Some(notify),
            _ => None,
        }
    }

    fn in_flight_matches(&self, key: &IdempotencyKey, notify: &Arc<Notify>) -> bool {
        matches!(
            self.records.get(key.as_str()),
            Some(DispatchRecord::InFlight { notify: current }) if Arc::ptr_eq(current, notify)
        )
    }

    fn evict_completed_until_below_limit(&mut self) -> Result<(), AgentLoopHostError> {
        let mut scanned = 0;
        let scan_limit = self.insertion_order.len();
        while self.records.len() >= MAX_IN_MEMORY_DISPATCH_RECORDS && scanned < scan_limit {
            let Some(candidate) = self.insertion_order.pop_front() else {
                break;
            };
            scanned += 1;
            match self.records.get(&candidate) {
                None => {}
                Some(DispatchRecord::InFlight { .. }) => self.insertion_order.push_back(candidate),
                Some(DispatchRecord::RuntimeCompleted { .. })
                | Some(DispatchRecord::TerminalMilestonePending { .. })
                | Some(DispatchRecord::LoopCompleted { .. }) => {
                    self.records.remove(&candidate);
                }
            }
        }
        if self.records.len() >= MAX_IN_MEMORY_DISPATCH_RECORDS {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability dispatch record store is full",
            ));
        }
        Ok(())
    }
}

enum DispatchReservation {
    Reserved,
    Wait(Arc<Notify>),
    RuntimeCompleted {
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    TerminalMilestonePending {
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
    },
    LoopCompleted(Result<GatedResolution, AgentLoopHostError>),
}

/// RAII guard for an `InFlight` dispatch reservation: if the holder drops
/// without calling [`Self::commit`], the reservation is cleared and any
/// waiters are notified. Clearing failures are logged but do not panic, since
/// dropping happens on unwind paths where there's nothing useful to propagate.
struct DispatchReservationGuard<'a> {
    port: &'a HostRuntimeLoopCapabilityPort,
    key: IdempotencyKey,
    committed: bool,
}

impl DispatchReservationGuard<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for DispatchReservationGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(error) = self.port.clear_dispatch(&self.key) {
            tracing::warn!(
                cleanup_error = %error,
                "failed to clean up dispatch reservation after early return"
            );
        }
    }
}

/// RAII guard for an `InFlight` gate-resolution reservation: if the owning
/// persist future drops without calling [`Self::commit`] (cancellation, a
/// transient store fault, or any early error), the reservation is cleared and
/// its waiters woken so a same-key replay re-owns and retries — never left
/// waiting on an orphaned in-flight entry. Mirrors [`DispatchReservationGuard`].
struct GateResolutionReservationGuard<'a> {
    port: &'a HostRuntimeLoopCapabilityPort,
    key: IdempotencyKey,
    committed: bool,
}

impl GateResolutionReservationGuard<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for GateResolutionReservationGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(error) = self.port.clear_gate_resolution_reservation(&self.key) {
            tracing::warn!(
                cleanup_error = %error,
                "failed to clean up gate resolution reservation after early return"
            );
        }
    }
}

#[derive(Default)]
struct ProviderToolCallRegistrationStore {
    records: HashMap<String, ProviderToolCallRegistrationRecord>,
}

#[derive(Clone)]
struct ProviderToolCallRegistrationRecord {
    activity_id: CapabilityActivityId,
    capability_id: CapabilityId,
    effective_capability_ids: Option<HashSet<CapabilityId>>,
}

impl ProviderToolCallRegistrationStore {
    /// Register one canonical provider tool call for this run. `input_ref` is
    /// only the lookup key; the activity id remains an independent UI identity
    /// stored with the registration record.
    fn record(
        &mut self,
        input_ref: &CapabilityInputRef,
        capability_id: &CapabilityId,
        activity_id: Option<CapabilityActivityId>,
        effective_capability_ids: Option<HashSet<CapabilityId>>,
    ) -> Result<CapabilityActivityId, AgentLoopHostError> {
        let key = input_ref.as_str().to_string();
        let record =
            self.records
                .entry(key)
                .or_insert_with(|| ProviderToolCallRegistrationRecord {
                    activity_id: activity_id.unwrap_or_default(),
                    capability_id: capability_id.clone(),
                    effective_capability_ids: None,
                });
        if record.capability_id != *capability_id {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool-call capability identity changed",
            ));
        }
        if let Some(activity_id) = activity_id
            && record.activity_id != activity_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool-call activity identity changed",
            ));
        }
        if let Some(next_effective_capability_ids) = effective_capability_ids {
            match &record.effective_capability_ids {
                Some(existing) if existing != &next_effective_capability_ids => {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "provider tool-call effective capability identity changed",
                    ));
                }
                Some(_) => {}
                None => {
                    record.effective_capability_ids = Some(next_effective_capability_ids);
                }
            }
        }
        Ok(record.activity_id)
    }

    fn registration_for(
        &self,
        input_ref: &CapabilityInputRef,
    ) -> Option<ProviderToolCallRegistrationRecord> {
        self.records.get(input_ref.as_str()).cloned()
    }
}

pub struct HostRuntimeLoopCapabilityPort {
    runtime: Arc<dyn HostRuntime>,
    run_context: LoopRunContext,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    execution_mounts: MountView,
    capability_execution_mounts: HashMap<CapabilityId, MountView>,
    snapshots: Mutex<HashMap<String, SurfaceSnapshot>>,
    current_surface_version: Mutex<Option<String>>,
    dispatch_records: Mutex<DispatchRecordStore>,
    provider_tool_call_registrations: Mutex<ProviderToolCallRegistrationStore>,
    trajectory_observer: Option<Arc<dyn CapabilityTrajectoryObserver>>,
    /// Durable store for the model-visible [`GateRecord`] a pending gate renders
    /// from on a later resume turn (§5.2.9). Written at the capability seam when a
    /// gate/suspension outcome is produced; see `persist_gate_record_for_mapped`.
    gate_record_store: Arc<dyn GateRecordStorePort>,
    /// Host-private store for the raw replay payload (tool `input` + `estimate`)
    /// a gate/auth resume re-dispatches from (arch-simplification §5.3 Stage
    /// 2a-i). Written at a FRESH gate raise keyed by `InvocationId`
    /// (`persist_replay_payload_for_fresh_gate`); loaded on resume by the
    /// invocation id recovered from the resume token
    /// (`replay_payload_for_resume`). Never model-visible.
    replay_payload_store: Arc<dyn ReplayPayloadStorePort>,
    /// Per-idempotency-key reservation for a gate outcome's persisted
    /// [`Resolution`]. The mapping mints a fresh random `GateRef` per call for
    /// the approval/resource/dependent/external channels, so a replayed
    /// invocation (same key) must return the FIRST invocation's resolution — the
    /// one whose gate ref the record is under — not a freshly-minted ref no
    /// record exists under (#6287). Exactly one caller (the owner) persists the
    /// record; concurrent duplicates and later replays WAIT on the reservation's
    /// notify for that durable save before receiving the resolution, so a
    /// concurrent replay never receives a blocked resolution whose record is not
    /// yet persisted. A failed save clears the reservation and wakes the waiters
    /// so one of them re-owns and retries.
    persisted_gate_resolutions: Mutex<HashMap<IdempotencyKey, GateResolutionState>>,
}

/// Reservation state for a gate outcome's persisted resolution, keyed by
/// idempotency key. Mirrors the `InFlight`/completed shape of
/// [`DispatchRecord`] so the wait is the same lost-wakeup-safe pattern as
/// [`HostRuntimeLoopCapabilityPort::wait_for_dispatch_completion`].
enum GateResolutionState {
    /// The owning invocation is persisting the record. Waiters block on the
    /// notify until it either publishes `Persisted` or clears the entry.
    InFlight(Arc<Notify>),
    /// The record is durably persisted; the resolution is safe to hand back to
    /// a replayed invocation. Boxed so this variant does not dominate the map
    /// entry's size over the pointer-sized `InFlight`.
    Persisted(Box<Resolution>),
}

/// Lock a poisoned-aware `Mutex` and wrap a poison error as the canonical
/// "<label> is unavailable" host error. Every store in this module is reached
/// via this helper so the error message stays consistent and the call sites
/// shrink to one line.
fn lock_mut<'a, T>(
    mutex: &'a Mutex<T>,
    label: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, AgentLoopHostError> {
    mutex.lock().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("{label} is unavailable"),
        )
    })
}

impl HostRuntimeLoopCapabilityPort {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        run_context: LoopRunContext,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> =
            Arc::new(ProviderToolCallInputResolver::new(input_resolver));
        Self {
            runtime,
            run_context,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink,
            execution_mounts: MountView::default(),
            capability_execution_mounts: HashMap::new(),
            snapshots: Mutex::new(HashMap::new()),
            current_surface_version: Mutex::new(None),
            dispatch_records: Mutex::new(DispatchRecordStore::default()),
            provider_tool_call_registrations: Mutex::new(
                ProviderToolCallRegistrationStore::default(),
            ),
            trajectory_observer: None,
            // Transitional no-op default; composition wires the durable store
            // through the factory's `with_gate_record_store`, which forwards via
            // the port-level builder below. See `NoopGateRecordStore`.
            gate_record_store: Arc::new(NoopGateRecordStore),
            // Transitional fail-closed default; composition wires the durable
            // store through the factory's `with_replay_payload_store`. See
            // `NoopReplayPayloadStore`.
            replay_payload_store: Arc::new(NoopReplayPayloadStore),
            persisted_gate_resolutions: Mutex::new(HashMap::new()),
        }
    }

    /// Wire the durable [`GateRecordStorePort`] this port persists pending-gate
    /// records into (§5.2.9). Defaults to the transitional
    /// [`NoopGateRecordStore`] when unset.
    pub fn with_gate_record_store(mut self, store: Arc<dyn GateRecordStorePort>) -> Self {
        self.gate_record_store = store;
        self
    }

    /// Wire the durable host-private [`ReplayPayloadStorePort`] this port persists
    /// gate/auth replay payloads into and reconstitutes them from on resume
    /// (arch-simplification §5.3 Stage 2a-i). Defaults to the transitional
    /// fail-closed [`NoopReplayPayloadStore`] when unset.
    pub fn with_replay_payload_store(mut self, store: Arc<dyn ReplayPayloadStorePort>) -> Self {
        self.replay_payload_store = store;
        self
    }

    /// Attach a [`CapabilityTrajectoryObserver`] notified of each capability's
    /// resolved input as this port executes it. No-op when unset.
    pub fn with_trajectory_observer(
        mut self,
        observer: Option<Arc<dyn CapabilityTrajectoryObserver>>,
    ) -> Self {
        self.trajectory_observer = observer;
        self
    }

    pub fn with_execution_mounts(mut self, mounts: MountView) -> Self {
        self.execution_mounts = mounts;
        self
    }

    pub fn with_capability_execution_mounts(
        mut self,
        mounts: HashMap<CapabilityId, MountView>,
    ) -> Self {
        self.capability_execution_mounts = mounts;
        self
    }

    fn execution_mounts_for(&self, capability_id: &CapabilityId) -> &MountView {
        self.capability_execution_mounts
            .get(capability_id)
            .unwrap_or(&self.execution_mounts)
    }

    fn snapshot_for(
        &self,
        version: &ironclaw_loop_contracts::CapabilitySurfaceVersion,
    ) -> Result<SurfaceSnapshot, AgentLoopHostError> {
        let snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        snapshots.get(version.as_str()).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is stale or unknown",
            )
        })
    }

    fn current_snapshot(&self) -> Result<Option<(String, SurfaceSnapshot)>, AgentLoopHostError> {
        let snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        let version = lock_mut(
            &self.current_surface_version,
            "capability surface snapshot pointer",
        )?
        .clone();
        let Some(version) = version else {
            return Ok(None);
        };
        let snapshot = snapshots.get(&version).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "current capability surface snapshot is unavailable",
            )
        })?;
        Ok(Some((version, snapshot)))
    }

    fn reserve_dispatch(
        &self,
        key: &IdempotencyKey,
        requested_invocation_id: InvocationId,
    ) -> Result<DispatchReservation, AgentLoopHostError> {
        lock_mut(&self.dispatch_records, "capability dispatch record store")?
            .reserve(key, requested_invocation_id)
    }

    fn dispatch_in_flight_matches(
        &self,
        key: &IdempotencyKey,
        notify: &Arc<Notify>,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(
            lock_mut(&self.dispatch_records, "capability dispatch record store")?
                .in_flight_matches(key, notify),
        )
    }

    fn record_runtime_completed(
        &self,
        key: &IdempotencyKey,
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::RuntimeCompleted {
                invocation_id,
                correlation_id,
                requested_capability_id,
                outcome,
            },
        );
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn record_terminal_milestone_pending(
        &self,
        key: &IdempotencyKey,
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::TerminalMilestonePending {
                invocation_id,
                result,
                milestone,
            },
        );
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn record_loop_completed(
        &self,
        key: &IdempotencyKey,
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::LoopCompleted {
                invocation_id,
                result,
            },
        );
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn clear_dispatch(&self, key: &IdempotencyKey) -> Result<(), AgentLoopHostError> {
        let notify =
            lock_mut(&self.dispatch_records, "capability dispatch record store")?.remove(key);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn record_provider_tool_call_registration(
        &self,
        input_ref: &CapabilityInputRef,
        capability_id: &CapabilityId,
        activity_id: Option<CapabilityActivityId>,
        effective_capability_ids: Option<HashSet<CapabilityId>>,
    ) -> Result<CapabilityActivityId, AgentLoopHostError> {
        lock_mut(
            &self.provider_tool_call_registrations,
            "provider tool-call registration store",
        )?
        .record(
            input_ref,
            capability_id,
            activity_id,
            effective_capability_ids,
        )
    }

    fn provider_tool_call_registration_for(
        &self,
        input_ref: &CapabilityInputRef,
    ) -> Result<Option<ProviderToolCallRegistrationRecord>, AgentLoopHostError> {
        Ok(lock_mut(
            &self.provider_tool_call_registrations,
            "provider tool-call registration store",
        )?
        .registration_for(input_ref))
    }

    fn validate_provider_tool_call_registration_activity(
        &self,
        input_ref: &CapabilityInputRef,
        activity_id: CapabilityActivityId,
    ) -> Result<(), AgentLoopHostError> {
        if let Some(registration) = self.provider_tool_call_registration_for(input_ref)?
            && registration.activity_id != activity_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "registered provider tool-call activity identity does not match the requested activity",
            ));
        }
        Ok(())
    }

    /// Drop guard for an `InFlight` dispatch reservation. Releases the
    /// reservation (and wakes any waiters) unless [`commit`] is called first.
    /// Use after a successful `reserve_dispatch` returns `Reserved` so any
    /// early-return error path between reservation and outcome recording
    /// unwinds the reservation automatically.
    fn dispatch_reservation_guard<'a>(
        &'a self,
        key: &IdempotencyKey,
    ) -> DispatchReservationGuard<'a> {
        DispatchReservationGuard {
            port: self,
            key: key.clone(),
            committed: false,
        }
    }

    fn validate_visible_request_scope(&self) -> Result<(), AgentLoopHostError> {
        let context = &self.visible_request.context;
        context.validate().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability execution context is invalid",
            )
        })?;
        if context.tenant_id != self.run_context.scope.tenant_id
            || context.agent_id != self.run_context.scope.agent_id
            || context.project_id != self.run_context.scope.project_id
            || context.thread_id.as_ref() != Some(&self.run_context.thread_id)
            || context.resource_scope.tenant_id != self.run_context.scope.tenant_id
            || context.resource_scope.agent_id != self.run_context.scope.agent_id
            || context.resource_scope.project_id != self.run_context.scope.project_id
            || context.resource_scope.thread_id.as_ref() != Some(&self.run_context.thread_id)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "capability execution context is not scoped to this loop run",
            ));
        }
        if context.mounts != MountView::default() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unauthorized,
                "capability execution context must not carry caller-supplied mounts",
            ));
        }
        Ok(())
    }

    async fn finish_runtime_outcome(
        &self,
        key: &IdempotencyKey,
        completion: RuntimeOutcomeCompletion<'_>,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        let result = runtime_outcome_to_loop(
            &self.run_context,
            self.result_writer.as_ref(),
            completion.conversion(),
        )
        .await;
        if should_retry_result_write(&completion.outcome, &result) {
            self.record_runtime_completed(
                key,
                completion.invocation_id,
                completion.correlation_id,
                completion.requested_capability_id.clone(),
                completion.outcome,
            )?;
            return result;
        }
        if result.is_err() {
            self.record_loop_completed(key, completion.invocation_id, result.clone())?;
            return result;
        }
        let terminal_milestone = match runtime_terminal_milestone(
            CapabilityActivityId::from_uuid(completion.invocation_id.as_uuid()),
            completion.provider,
            completion.runtime,
            &completion.outcome,
        ) {
            Ok(milestone) => milestone,
            Err(error) => {
                let result = Err(error);
                self.record_loop_completed(key, completion.invocation_id, result.clone())?;
                return result;
            }
        };
        self.complete_terminal_milestone(key, completion.invocation_id, result, terminal_milestone)
            .await
    }

    async fn finish_auth_decline_outcome(
        &self,
        key: &IdempotencyKey,
        conversion: RuntimeOutcomeConversion<'_>,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        let RuntimeOutcomeConversion {
            input_ref,
            invocation_id,
            correlation_id,
            requested_capability_id,
            outcome,
        } = conversion;
        let failure = match &outcome {
            RuntimeCapabilityOutcome::Failed(failure) => failure,
            _ => {
                let result = Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "capability auth decline returned a non-terminal runtime outcome",
                ));
                self.record_loop_completed(key, invocation_id, result.clone())?;
                return result;
            }
        };
        let result = runtime_outcome_to_loop(
            &self.run_context,
            self.result_writer.as_ref(),
            RuntimeOutcomeConversion {
                input_ref,
                invocation_id,
                correlation_id,
                requested_capability_id,
                outcome: outcome.clone(),
            },
        )
        .await;
        if should_retry_result_write(&outcome, &result) {
            self.record_runtime_completed(
                key,
                invocation_id,
                correlation_id,
                requested_capability_id.clone(),
                outcome.clone(),
            )?;
            return result;
        }
        if result.is_err() {
            self.record_loop_completed(key, invocation_id, result.clone())?;
            return result;
        }
        let milestone = LoopHostMilestoneKind::CapabilityFailed {
            activity_id: CapabilityActivityId::from_uuid(invocation_id.as_uuid()),
            capability_id: failure.capability_id.clone(),
            // The current contract may already be gone. Durable invocation
            // identity, rather than stale provider/runtime metadata, is the
            // authority for this terminal transition.
            provider: None,
            runtime: None,
            reason_kind: failure.kind,
            safe_summary: runtime_failure_loop_safe_summary(failure),
        };
        self.complete_terminal_milestone(key, invocation_id, result, Some(milestone))
            .await
    }

    async fn complete_terminal_milestone(
        &self,
        key: &IdempotencyKey,
        invocation_id: InvocationId,
        result: Result<GatedResolution, AgentLoopHostError>,
        terminal_milestone: Option<LoopHostMilestoneKind>,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        if let Some(milestone) = terminal_milestone
            && let Err(error) = self.emit_capability_milestone(milestone.clone()).await
        {
            self.record_terminal_milestone_pending(key, invocation_id, result.clone(), milestone)?;
            return Err(error);
        }
        self.record_loop_completed(key, invocation_id, result.clone())?;
        result
    }

    async fn wait_for_dispatch_completion(
        &self,
        key: &IdempotencyKey,
        notify: Arc<Notify>,
    ) -> Result<(), AgentLoopHostError> {
        let notified = notify.notified();
        tokio::pin!(notified);
        if self.dispatch_in_flight_matches(key, &notify)? {
            notified.await;
        }
        Ok(())
    }

    async fn emit_capability_milestone(
        &self,
        kind: LoopHostMilestoneKind,
    ) -> Result<(), AgentLoopHostError> {
        self.milestone_sink
            .publish_loop_milestone(LoopHostMilestone {
                scope: self.run_context.scope.clone(),
                actor: self.run_context.actor.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                loop_driver_id: self.run_context.loop_driver_id.clone(),
                kind,
            })
            .await
    }

    async fn invoke_synthetic_capability(
        &self,
        request: LoopRequest,
        capability: SyntheticSurfaceCapabilitySnapshot,
        snapshot: SurfaceSnapshot,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        let input = self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await?;
        let registration = self.provider_tool_call_registration_for(&request.input_ref)?;
        let effective_capability_ids = registration
            .and_then(|registration| registration.effective_capability_ids)
            .unwrap_or_default();
        let output = match capability.output(&input, |requested| {
            let capability = snapshot.capability_info(requested)?;
            if !effective_capability_ids.contains(capability.capability_id) {
                return None;
            }
            Some(capability)
        }) {
            Ok(output) => output,
            Err(error) if error.kind == AgentLoopHostErrorKind::InvalidInvocation => {
                // Synthetic capability InvalidInvocation errors are model-side input failures
                // such as bad arguments or an unknown capability_info target. Keep those
                // model-visible so the driver can retry instead of terminalizing the host.
                // INVARIANT: synthetic capabilities must not use InvalidInvocation for
                // internal or host-fatal conditions.
                let detail = diagnostic_detail_from_raw(&error.safe_summary);
                return Ok(GatedResolution::bare(resolution::failed(
                    FailureKind::InputEncode,
                    error.safe_summary,
                    detail,
                )));
            }
            Err(error) => return Err(error),
        };
        let write_result = self
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &self.run_context,
                input_ref: &request.input_ref,
                invocation_id: InvocationId::from_uuid(request.activity_id.as_uuid()),
                capability_id: &request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        Ok(GatedResolution::bare(resolution::completed(
            write_result.result_ref,
            "capability info returned".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            write_result.byte_len,
            write_result.output_digest,
            write_result.model_observation,
        )))
    }

    fn prepare_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<PreparedProviderToolCall, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        validate_provider_tool_call(tool_call)?;
        let provider_turn_id = tool_call.turn_id.clone().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is missing a provider turn id",
            )
        })?;
        let Some((version, snapshot)) = self.current_snapshot()? else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is unavailable",
            ));
        };
        let (capability_id, capability) = snapshot.provider_capability(&tool_call.name)?;
        let prepared =
            capability.prepare_provider_tool_call(capability_id, &snapshot, tool_call)?;
        Ok(PreparedProviderToolCall {
            surface_version: loop_surface_version(&version)?,
            capability_id: prepared.capability_id,
            provider_turn_id,
            normalized_arguments: prepared.normalized_arguments,
            effective_capability_ids: prepared.effective_capability_ids,
        })
    }

    async fn register_provider_tool_call_with_activity(
        &self,
        tool_call: ProviderToolCall,
        activity_id: Option<CapabilityActivityId>,
    ) -> Result<ironclaw_loop_contracts::CapabilityCallCandidate, AgentLoopHostError> {
        let prepared = self.prepare_provider_tool_call(&tool_call)?;
        let mut normalized_tool_call = tool_call.clone();
        normalized_tool_call.arguments = prepared.normalized_arguments;
        let input_ref = self
            .input_resolver
            .register_provider_tool_call_input(&self.run_context, &normalized_tool_call)
            .await?;
        // Record the activity-card display input now that both the canonical
        // `input_ref` and the resolved dotted `capability_id` are in hand, so
        // the card shows `nearai.web_search   <query>` (not the lossy provider
        // tool name `nearai__web_search`) and the per-tool summary matches.
        self.input_resolver.record_provider_tool_call_display_input(
            &self.run_context,
            &input_ref,
            &prepared.capability_id,
            &normalized_tool_call,
        );
        let registered_effective_capability_ids = (prepared.capability_id.as_str()
            == crate::capability_info::CAPABILITY_ID)
            .then(|| prepared.effective_capability_ids.iter().cloned().collect());
        let activity_id = self.record_provider_tool_call_registration(
            &input_ref,
            &prepared.capability_id,
            activity_id,
            registered_effective_capability_ids,
        )?;
        Ok(ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id,
            surface_version: prepared.surface_version,
            capability_id: prepared.capability_id,
            input_ref,
            effective_capability_ids: prepared.effective_capability_ids,
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: tool_call.provider_id,
                provider_model_id: tool_call.provider_model_id,
                provider_turn_id: prepared.provider_turn_id,
                provider_call_id: tool_call.id,
                provider_tool_name: tool_call.name,
                arguments: tool_call.arguments,
                response_reasoning: tool_call.response_reasoning,
                reasoning: tool_call.reasoning,
                signature: tool_call.signature,
            }),
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for HostRuntimeLoopCapabilityPort {
    fn requires_ordered_batch_invocation(&self, _invocations: &[LoopRequest]) -> bool {
        false
    }

    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        let Some((_, snapshot)) = self.current_snapshot()? else {
            return Ok(Vec::new());
        };
        let mut definitions = Vec::new();
        for (capability_id, capability) in &snapshot.capabilities {
            if let Some(definition) = capability.tool_definition(capability_id)? {
                definitions.push(definition);
            }
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        let prepared = self.prepare_provider_tool_call(tool_call)?;
        Ok(ProviderToolCallCapabilityIds {
            provider_capability_id: prepared.capability_id,
            effective_capability_ids: prepared.effective_capability_ids,
        })
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.prepare_provider_tool_call(tool_call).map(|_| ())
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<ironclaw_loop_contracts::CapabilityCallCandidate, AgentLoopHostError> {
        self.register_provider_tool_call_with_activity(request.tool_call, request.activity_id)
            .await
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        let runtime_surface = self
            .runtime
            .visible_capabilities(self.visible_request.clone())
            .await
            .map_err(host_runtime_error)?;
        let version = loop_surface_version(runtime_surface.version.as_str())?;
        let mut snapshot = SurfaceSnapshot::with_synthetic_capabilities()?;
        let mut descriptors = runtime_surface
            .capabilities
            .into_iter()
            .map(|capability| {
                let capability_id = capability.descriptor.id.clone();
                if snapshot.capabilities.contains_key(&capability_id) {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "host runtime capability id is reserved for a synthetic loop capability",
                    ));
                }
                let provider_tool_name =
                    provider_tool_name(&capability.descriptor.id, &snapshot.provider_names);
                snapshot
                    .provider_names
                    .insert(provider_tool_name.clone(), capability_id.clone());
                snapshot.capabilities.insert(
                    capability_id.clone(),
                    SurfaceCapabilitySnapshot::Runtime(Box::new(
                        RuntimeSurfaceCapabilitySnapshot {
                            provider: capability.descriptor.provider.clone(),
                            runtime: capability.descriptor.runtime,
                            estimate: capability.estimated_resources.clone(),
                            safe_description: capability.descriptor.description.clone(),
                            description_trust: capability.description_trust,
                            parameters_schema: capability.descriptor.parameters_schema.clone(),
                            effects: capability.descriptor.effects.clone(),
                            provider_tool_name,
                        },
                    )),
                );
                Ok(CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    description_trust: capability.description_trust,
                    parameters_schema: capability.descriptor.parameters_schema,
                })
            })
            .collect::<Result<Vec<_>, AgentLoopHostError>>()?;
        descriptors.extend(snapshot.synthetic_descriptor_views()?);

        let mut snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        snapshots.clear();
        snapshots.insert(version.as_str().to_string(), snapshot);
        *lock_mut(
            &self.current_surface_version,
            "capability surface snapshot pointer",
        )? = Some(version.as_str().to_string());

        Ok(VisibleCapabilitySurface {
            version,
            descriptors,
            // Empty = "callable == advertised". A disclosure decorator that narrows
            // the advertised set populates this with the wider reachable catalog.
            callable_capability_ids: None,
        })
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        // §5.3 Stage 2b (collapse complete): dispatch produces the host_api
        // `Resolution` directly, paired with the durable `GateRecord` its channel
        // renders from (a `GatedResolution`) — mapped ONCE, by construction, so
        // the returned resolution carries the SAME gate ref the record is
        // persisted under. `persist_gate_record_for_mapped` persists that record
        // and returns the resolution to hand back; on a concurrent duplicate it
        // is the OWNER's resolution (whose gate ref the record is under), returned
        // only AFTER its durable save completes (#6287). The idempotency key is
        // derived INSIDE `persist_gate_record_for_mapped`, after dispatch and only
        // for a
        // gate-bearing outcome — its `resume.input_ref` binding is the
        // STORE-derived one (same derivation the dispatch cache uses), so it stays
        // byte-stable and identical to dispatch's (§5.3 Stage 0). Deriving it there
        // (rather than up front) keeps dispatch's own resume identity/activity
        // validation the FIRST error a malformed resume surfaces — a missing/stale
        // resume payload must not pre-empt an `InvalidInvocation` activity mismatch.
        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        let gated = Box::pin(self.invoke_capability_dispatch(request.clone())).await?;
        Box::pin(self.persist_gate_record_for_mapped(&request, gated)).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let mut resolutions = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            // `invoke_capability` (the trait method above) persists each gate
            // record at the seam, so the batch inherits per-outcome persistence.
            // Chain-boxing: each port delegation is boxed so the stacked
            // decorator chain never compiles into a single oversized poll
            // frame (see reborn_integration_model_recovery stack-overflow).
            let resolution = Box::pin(self.invoke_capability(invocation)).await?;
            // `parks()`, not `is_suspension()` (H1): a re-entrant gate (`Blocked`)
            // stops the batch too — nothing after a gated invocation can proceed
            // until it is resolved, exactly as parked work does.
            let parks = resolution.parks();
            resolutions.push(resolution);
            if request.stop_on_first_suspension && parks {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension,
        })
    }
}

impl HostRuntimeLoopCapabilityPort {
    /// Persist the durable, model-visible [`GateRecord`] a later resume turn
    /// renders from
    /// (§5.2.9), keyed by the freshly-minted [`GateRef`] on the resolution
    /// channel (#6242 mapping / #6243 store). `DenyRecord` is terminal and
    /// same-turn (per #6243) and is intentionally NOT persisted; `Done` and
    /// `Suspended(Process)` carry no gate record and no-op.
    ///
    /// Fail-closed: a store write failure is a genuine host storage fault and
    /// propagates, consistent with `record_loop_completed`/`record_runtime_completed`.
    ///
    /// The idempotency key (the write-once replay guard) is derived HERE, after
    /// dispatch and only once a gate record exists, from the SAME store-derived
    /// input_ref dispatch used (hazard 3, §5.3 Stage 0): on a resume the payload
    /// is reconstituted from the store, not the advisory loop-supplied
    /// `resume.input_ref`, so the key is byte-stable. Deriving it lazily keeps a
    /// missing/stale resume payload from pre-empting dispatch's own resume
    /// identity/activity validation — a malformed resume surfaces
    /// `InvalidInvocation` from dispatch, never a spurious payload-missing error.
    async fn persist_gate_record_for_mapped(
        &self,
        request: &LoopRequest,
        gated: GatedResolution,
    ) -> Result<Resolution, AgentLoopHostError> {
        let Some(record) = gated.gate_record.as_ref() else {
            // Done / Denied / Suspended(Process): nothing durable to persist, no
            // idempotency key needed, and no gate ref that must stay loadable.
            return Ok(gated.resolution);
        };
        let Some(gate_ref) = gate_ref_for_resolution(&gated.resolution) else {
            // A gate record without a gate-ref-bearing channel is a mapping
            // invariant violation, not a recoverable model-visible error.
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "mapped gate record has no gate ref on its resolution channel",
            ));
        };
        // The dispatch that produced this gate outcome already reconstituted (and
        // for a fresh raise, persisted) the resume payload, so this load hits a
        // present record on a resume and returns `None` on a fresh dispatch.
        let resume_payload = self.resume_replay_payload(request).await?;
        let effective_input_ref = resume_payload
            .as_ref()
            .map(|payload| &payload.input_ref)
            .unwrap_or(&request.input_ref);
        let idempotency_key =
            invocation_idempotency_key(&self.run_context, request, effective_input_ref)?;
        // Reserve-or-wait: exactly one caller (the owner) persists the record;
        // repeats and concurrent duplicates WAIT for that durable save before
        // receiving the resolution, so a concurrent replay never receives a
        // blocked resolution whose gate record is not yet persisted (#6287). The
        // mapping mints a fresh random `GateRef` per call, so a waiter must
        // return the owner's resolution — the one whose gate ref the record is
        // under — not its own re-mint. A failed save clears the reservation and
        // wakes the waiters so one re-owns and retries.
        let owner_notify = loop {
            let wait_notify = {
                let mut reserved = lock_mut(
                    &self.persisted_gate_resolutions,
                    "gate resolution replay cache",
                )?;
                match reserved.get(&idempotency_key) {
                    Some(GateResolutionState::Persisted(resolution)) => {
                        return Ok(resolution.as_ref().clone());
                    }
                    Some(GateResolutionState::InFlight(notify)) => Arc::clone(notify),
                    None => {
                        let notify = Arc::new(Notify::new());
                        reserved.insert(
                            idempotency_key.clone(),
                            GateResolutionState::InFlight(Arc::clone(&notify)),
                        );
                        break notify;
                    }
                }
            };
            // Lost-wakeup-safe wait (mirrors `wait_for_dispatch_completion`):
            // register on the notify, then re-check under the lock; only await if
            // the entry is still the SAME in-flight reservation.
            let notified = wait_notify.notified();
            tokio::pin!(notified);
            if self.gate_resolution_in_flight_matches(&idempotency_key, &wait_notify)? {
                notified.await;
            }
        };

        // RAII cleanup: if this future is cancelled (dropped mid-`save`) or
        // returns early before we commit, the guard clears the reservation and
        // wakes its waiters so a same-key replay re-owns and retries — never left
        // waiting on an orphaned in-flight entry (#6287 IronLoop). The success
        // path commits the guard AFTER publishing the durable resolution.
        let reservation_guard = GateResolutionReservationGuard {
            port: self,
            key: idempotency_key.clone(),
            committed: false,
        };
        let scope = self.visible_request.context.resource_scope.clone();
        let save_result = self
            .gate_record_store
            .save(scope, gate_ref, record.clone())
            .await;
        match save_result {
            // Success: publish the resolution (so waiters receive the SAME gate
            // ref the record is under), wake them, and commit the guard so its
            // drop is a no-op.
            Ok(()) => {
                self.publish_gate_resolution(&idempotency_key, &gated.resolution)?;
                owner_notify.notify_waiters();
                reservation_guard.commit();
                Ok(gated.resolution)
            }
            // A deterministic gate-record key (the auth gate's `for_auth_gate`,
            // and the approval gate's `for_approval_request` on the authorize
            // path) means a re-raise of the SAME gate — a deny-then-retry, or a
            // fresh port instance whose in-memory reservation was reset across
            // turns — derives the SAME content-addressed key and an identical
            // record. The write-once store reports `GateRecordAlreadyExists`;
            // that is benign (already persisted, byte-identical), never a fault.
            // "Byte-identical" holds because the auth-gate fingerprint
            // (`stable_auth_gate_id`) covers `setup` as well as provider /
            // requester / scopes (#6299 IronLoop), so two requirements that
            // differ in their setup flow derive DIFFERENT keys and never reach
            // this branch with a stale record.
            // Mirrors `persist_replay_payload_for_fresh_gate`'s tolerance of
            // `ReplayPayloadAlreadyExists`. Publish + commit like the success path.
            Err(ApprovalStoreError::GateRecordAlreadyExists { .. }) => {
                tracing::debug!(
                    %gate_ref,
                    "gate record already persisted for this deterministic key; keeping existing record"
                );
                self.publish_gate_resolution(&idempotency_key, &gated.resolution)?;
                owner_notify.notify_waiters();
                reservation_guard.commit();
                Ok(gated.resolution)
            }
            // Transient fault: do NOT commit. The guard's drop clears the
            // reservation and wakes the waiters so one re-owns and retries the
            // persist instead of hanging on an orphaned in-flight entry.
            Err(error) => Err(gate_record_store_error(error)),
        }
    }

    /// Publish the durable resolution for `key` so a waiting or later same-key
    /// replay receives the SAME gate ref the record was persisted under.
    fn publish_gate_resolution(
        &self,
        key: &IdempotencyKey,
        resolution: &Resolution,
    ) -> Result<(), AgentLoopHostError> {
        lock_mut(
            &self.persisted_gate_resolutions,
            "gate resolution replay cache",
        )?
        .insert(
            key.clone(),
            GateResolutionState::Persisted(Box::new(resolution.clone())),
        );
        Ok(())
    }

    /// Clear an in-flight gate-resolution reservation for `key` and wake its
    /// waiters so one re-owns and retries. Only clears an `InFlight` entry (never
    /// a published resolution), so a committed owner's guard is a no-op here.
    fn clear_gate_resolution_reservation(
        &self,
        key: &IdempotencyKey,
    ) -> Result<(), AgentLoopHostError> {
        let mut reserved = lock_mut(
            &self.persisted_gate_resolutions,
            "gate resolution replay cache",
        )?;
        if let Some(GateResolutionState::InFlight(notify)) = reserved.get(key) {
            let notify = Arc::clone(notify);
            reserved.remove(key);
            drop(reserved);
            notify.notify_waiters();
        }
        Ok(())
    }

    /// True iff `key`'s reservation is still the SAME in-flight entry `notify`
    /// belongs to — the re-check that makes [`Self::persist_gate_record_for_mapped`]'s
    /// wait lost-wakeup-safe (mirrors [`Self::dispatch_in_flight_matches`]).
    fn gate_resolution_in_flight_matches(
        &self,
        key: &IdempotencyKey,
        notify: &Arc<Notify>,
    ) -> Result<bool, AgentLoopHostError> {
        let reserved = lock_mut(
            &self.persisted_gate_resolutions,
            "gate resolution replay cache",
        )?;
        Ok(match reserved.get(key) {
            Some(GateResolutionState::InFlight(existing)) => Arc::ptr_eq(existing, notify),
            _ => false,
        })
    }

    /// Persist the host-private [`ReplayPayload`] a later gate/auth resume
    /// reconstitutes `{input, estimate}` from (arch-simplification §5.3 Stage
    /// 2a-i), keyed by `invocation_id`. Only an approval/auth gate outcome
    /// carries a resume; every other outcome no-ops.
    ///
    /// Called ONLY on a fresh dispatch, so `prior_approval` is always absent here
    /// (a fresh invocation has passed no prior approval gate) and the write cannot
    /// collide with an existing entry for a reused invocation id. The payload is
    /// invocation-stable, so a benign duplicate (`ReplayPayloadAlreadyExists`) is
    /// tolerated rather than ending the run; any other store fault is a genuine
    /// host storage failure and fails closed.
    async fn persist_replay_payload_for_fresh_gate(
        &self,
        invocation_id: InvocationId,
        input_ref: &CapabilityInputRef,
        input: &Value,
        estimate: &ResourceEstimate,
        correlation_id: CorrelationId,
        outcome: &RuntimeCapabilityOutcome,
    ) -> Result<(), AgentLoopHostError> {
        if !matches!(
            outcome,
            RuntimeCapabilityOutcome::ApprovalRequired(_)
                | RuntimeCapabilityOutcome::AuthRequired(_)
        ) {
            return Ok(());
        }
        let payload = ReplayPayload {
            input: input.clone(),
            estimate: estimate.clone(),
            // Fresh dispatch: no prior approval. The approval→auth bridge keeps
            // the prior-approval identity on the loop-facing resume wire in this
            // slice (it moves host-side in §5.3 Stage 2a-ii).
            prior_approval: None,
            input_ref: input_ref.clone(),
            correlation_id,
        };
        let scope = self.visible_request.context.resource_scope.clone();
        match self
            .replay_payload_store
            .save(scope, invocation_id, payload)
            .await
        {
            Ok(()) => Ok(()),
            Err(ReplayPayloadStoreError::ReplayPayloadAlreadyExists { .. }) => {
                // Invocation-stable payload already persisted; the resume-read path
                // will load the identical record. Benign, not a fault.
                tracing::debug!(
                    invocation_id = %invocation_id,
                    "replay payload already persisted for fresh gate raise; keeping existing record"
                );
                Ok(())
            }
            Err(error) => Err(replay_payload_store_error(error)),
        }
    }

    /// Load the host-private replay payload persisted at the fresh gate raise for
    /// `invocation_id` (recovered from the resume token). **Fail closed on a
    /// miss:** a resume whose payload is absent — including a wrong-scope read the
    /// store reports as unknown — is a sanitized terminal failure, never a silent
    /// empty-input dispatch (arch-simplification §5.3 Stage 2a-i).
    /// Reconstitute the host-private replay payload a resume binds to, if this is
    /// a resume. On a gate/auth resume the loop-supplied `input_ref` is ADVISORY:
    /// the payload persisted at the FRESH gate raise is the host-side source of
    /// truth for `input_ref` (and `{input, estimate}`), so the idempotency key
    /// stays byte-stable regardless of what the loop echoes back, and a resume
    /// whose payload is absent fails CLOSED (§5.3 Stage 2a-i / Stage 0).
    ///
    /// Returns `None` for a fresh dispatch and for the mutually-exclusive
    /// both-resume-modes case — `invoke_capability_dispatch`'s `resume_mode`
    /// resolution surfaces the latter as `InvalidInvocation`; this helper does not
    /// pre-empt that with a payload load. Keeping the derivation in one place lets
    /// the `invoke_capability` seam and dispatch compute the SAME key.
    async fn resume_replay_payload(
        &self,
        request: &LoopRequest,
    ) -> Result<Option<ReplayPayload>, AgentLoopHostError> {
        let invocation_id = match (
            request.approval_resume.as_ref(),
            request.auth_resume.as_ref(),
        ) {
            (Some(_), Some(_)) | (Option::None, Option::None) => return Ok(Option::None),
            (Some(resume), Option::None) => invocation_id_from_resume_token(&resume.resume_token)?,
            (Option::None, Some(auth_resume)) => {
                let Some(resume_token) = auth_resume.resume_token.as_ref() else {
                    return Ok(Option::None);
                };
                invocation_id_from_resume_token(resume_token)?
            }
        };
        Ok(Some(self.replay_payload_for_resume(invocation_id).await?))
    }

    async fn replay_payload_for_resume(
        &self,
        invocation_id: InvocationId,
    ) -> Result<ReplayPayload, AgentLoopHostError> {
        let scope = self.visible_request.context.resource_scope.clone();
        let payload = self
            .replay_payload_store
            .load(&scope, invocation_id)
            .await
            .map_err(replay_payload_store_error)?;
        payload.ok_or_else(|| {
            tracing::warn!(
                invocation_id = %invocation_id,
                "capability resume replay payload is missing; failing the run closed"
            );
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability resume replay payload is unavailable",
            )
        })
    }

    async fn invoke_auth_decline_dispatch(
        &self,
        request: LoopRequest,
        invocation_id: InvocationId,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        let idempotency_key = auth_decline_idempotency_key(
            &self.run_context,
            request.activity_id,
            invocation_id,
            &request.capability_id,
        )?;
        loop {
            match self.reserve_dispatch(&idempotency_key, invocation_id)? {
                DispatchReservation::Reserved => break,
                DispatchReservation::Wait(notify) => {
                    self.wait_for_dispatch_completion(&idempotency_key, notify)
                        .await?;
                }
                DispatchReservation::RuntimeCompleted {
                    invocation_id,
                    correlation_id,
                    requested_capability_id,
                    outcome,
                } => {
                    return self
                        .finish_auth_decline_outcome(
                            &idempotency_key,
                            RuntimeOutcomeConversion {
                                input_ref: &request.input_ref,
                                invocation_id,
                                correlation_id,
                                requested_capability_id: &requested_capability_id,
                                outcome,
                            },
                        )
                        .await;
                }
                DispatchReservation::TerminalMilestonePending {
                    invocation_id,
                    result,
                    milestone,
                } => {
                    return self
                        .complete_terminal_milestone(
                            &idempotency_key,
                            invocation_id,
                            result,
                            Some(milestone),
                        )
                        .await;
                }
                DispatchReservation::LoopCompleted(result) => return result,
            }
        }

        let guard = self.dispatch_reservation_guard(&idempotency_key);
        let invocation_context = auth_decline_context_from_visible(
            &self.visible_request.context,
            &self.run_context,
            request.activity_id,
        )?;
        let correlation_id = invocation_context.correlation_id;
        let requested_capability_id = request.capability_id.clone();
        self.result_writer.record_running_invocation(
            &self.run_context,
            invocation_id,
            &request.input_ref,
        );
        let activity_id = CapabilityActivityId::from_uuid(invocation_id.as_uuid());
        self.emit_capability_milestone(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id: requested_capability_id.clone(),
        })
        .await?;

        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        let outcome = match Box::pin(dispatch_runtime_capability_auth_decline(
            self.runtime.as_ref(),
            invocation_context,
            request.capability_id,
        ))
        .await
        {
            Ok(outcome) => outcome,
            Err(error @ HostRuntimeError::Unavailable { .. }) => {
                return Err(host_runtime_error(error));
            }
            Err(error) => {
                let host_error = host_runtime_error(error);
                let milestone = LoopHostMilestoneKind::CapabilityFailed {
                    activity_id,
                    capability_id: requested_capability_id,
                    provider: None,
                    runtime: None,
                    reason_kind: host_error.kind.failure_kind(),
                    safe_summary: None,
                };
                guard.commit();
                return self
                    .complete_terminal_milestone(
                        &idempotency_key,
                        invocation_id,
                        Err(host_error),
                        Some(milestone),
                    )
                    .await;
            }
        };
        guard.commit();
        self.finish_auth_decline_outcome(
            &idempotency_key,
            RuntimeOutcomeConversion {
                input_ref: &request.input_ref,
                invocation_id,
                correlation_id,
                requested_capability_id: &requested_capability_id,
                outcome,
            },
        )
        .await
    }

    async fn invoke_capability_dispatch(
        &self,
        request: LoopRequest,
    ) -> Result<GatedResolution, AgentLoopHostError> {
        let requested_invocation_id = InvocationId::from_uuid(request.activity_id.as_uuid());
        if let Some(auth_resume) = request.auth_resume.as_ref().filter(|resume| {
            matches!(
                resume.disposition,
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            if request.approval_resume.is_some() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "capability invocation has both approval_resume and auth_resume set; \
                     these resume modes are mutually exclusive",
                ));
            }
            if auth_resume.prior_approval.is_some() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "denied capability auth resume must not carry prior approval identity",
                ));
            }
            if let Some(resume_token) = auth_resume.resume_token.as_ref() {
                let token_invocation_id = invocation_id_from_resume_token(resume_token)?;
                ensure_resume_invocation_matches_activity(
                    token_invocation_id,
                    requested_invocation_id,
                    "auth denial",
                )?;
            }
            // Chain-boxing: each port delegation is boxed so the stacked
            // decorator chain never compiles into a single oversized poll
            // frame (see reborn_integration_model_recovery stack-overflow).
            return Box::pin(self.invoke_auth_decline_dispatch(request, requested_invocation_id))
                .await;
        }
        // Normalize resume mode and validate token/activity identity before
        // dispatch reservation. Cached replay branches can return without
        // touching runtime state, so they must pass the same fail-closed checks
        // as fresh dispatch.
        enum ResolvedResumeMode<'a> {
            Approval {
                resume: &'a CapabilityApprovalResume,
                invocation_id: InvocationId,
            },
            Auth {
                resume: &'a CapabilityAuthResume,
                invocation_id: InvocationId,
            },
            None,
        }
        let resume_mode = match (
            request.approval_resume.as_ref(),
            request.auth_resume.as_ref(),
        ) {
            (Some(_), Some(_)) => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "capability invocation has both approval_resume and auth_resume set; \
                     these resume modes are mutually exclusive",
                ));
            }
            (Some(resume), _) => {
                let resume_invocation_id = invocation_id_from_resume_token(&resume.resume_token)?;
                ensure_resume_invocation_matches_activity(
                    resume_invocation_id,
                    requested_invocation_id,
                    "approval",
                )?;
                ResolvedResumeMode::Approval {
                    resume,
                    invocation_id: resume_invocation_id,
                }
            }
            (_, Some(auth_resume)) => {
                let resume_token = auth_resume.resume_token.as_ref().ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "resolved capability auth resume is missing its resume token",
                    )
                })?;
                let resume_invocation_id = invocation_id_from_resume_token(resume_token)?;
                ensure_resume_invocation_matches_activity(
                    resume_invocation_id,
                    requested_invocation_id,
                    "auth",
                )?;
                ResolvedResumeMode::Auth {
                    resume: auth_resume,
                    invocation_id: resume_invocation_id,
                }
            }
            (Option::None, Option::None) => ResolvedResumeMode::None,
        };
        // Host-side resume reconstitution (hazard 3, §5.3 Stage 0): on a resume the
        // effective input_ref used for the idempotency key + validation is derived
        // from the host-private payload persisted at the FRESH gate raise — loaded
        // by the resume's invocation id — NOT from the advisory loop-supplied
        // `resume.input_ref`. `resume_mode` above already validated token identity
        // and the resume→activity match, so that precedence still runs before the
        // registered-activity check below; the payload load fails CLOSED on a miss
        // (§5.3 Stage 2a-i). The same payload is reused for `{input, estimate}`
        // below, so a resume loads it exactly once here.
        let resume_payload = match &resume_mode {
            ResolvedResumeMode::Approval { invocation_id, .. }
            | ResolvedResumeMode::Auth { invocation_id, .. } => {
                // Chain-boxing: each port delegation is boxed so the stacked
                // decorator chain never compiles into a single oversized poll
                // frame (see reborn_integration_model_recovery stack-overflow).
                Some(Box::pin(self.replay_payload_for_resume(*invocation_id)).await?)
            }
            ResolvedResumeMode::None => Option::None,
        };
        // Owned clone so `effective_input_ref` borrows this local, not
        // `resume_payload` — the payload is consumed for `{input, estimate}` below.
        let resume_input_ref = resume_payload
            .as_ref()
            .map(|payload| payload.input_ref.clone());
        let effective_input_ref = resume_input_ref.as_ref().unwrap_or(&request.input_ref);
        // Host-side resume reconstitution of the correlation identity (§5.3 Stage
        // 2a-i, mirroring `input_ref`): the loop-facing `Resolution` no longer
        // carries the original `correlation_id` (it is minted fresh at the loop
        // boundary post-flip), so the authoritative one — the identity the
        // fingerprinted approval lease is scoped to — is reconstituted here from
        // the host-private replay payload persisted at the fresh gate raise. Using
        // the loop's advisory value instead would fail the lease's correlation
        // match ("approval request does not match invocation: correlation_id").
        let resume_correlation_id = resume_payload
            .as_ref()
            .map(|payload| payload.correlation_id);
        self.validate_provider_tool_call_registration_activity(
            effective_input_ref,
            request.activity_id,
        )?;
        let snapshot = self.snapshot_for(&request.surface_version)?;
        let Some(capability) = snapshot.capabilities.get(&request.capability_id).cloned() else {
            return Ok(GatedResolution::bare(
                resolution::denied(
                    capability_denied_reason_kind("outside_visible_surface")?,
                    "capability was not visible on the cited surface".to_string(),
                )
                .resolution,
            ));
        };
        let idempotency_key =
            invocation_idempotency_key(&self.run_context, &request, effective_input_ref)?;
        let requested_invocation_id = InvocationId::from_uuid(request.activity_id.as_uuid());
        loop {
            match self.reserve_dispatch(&idempotency_key, requested_invocation_id)? {
                DispatchReservation::Reserved => break,
                DispatchReservation::Wait(notify) => {
                    self.wait_for_dispatch_completion(&idempotency_key, notify)
                        .await?;
                }
                DispatchReservation::RuntimeCompleted {
                    invocation_id,
                    correlation_id,
                    requested_capability_id,
                    outcome,
                } => {
                    if let SurfaceCapabilitySnapshot::Runtime(capability) = &capability {
                        return self
                            .finish_runtime_outcome(
                                &idempotency_key,
                                RuntimeOutcomeCompletion {
                                    input_ref: effective_input_ref,
                                    invocation_id,
                                    correlation_id,
                                    requested_capability_id: &requested_capability_id,
                                    provider: capability.provider.clone(),
                                    runtime: capability.runtime,
                                    outcome,
                                },
                            )
                            .await;
                    }
                    let result = runtime_outcome_to_loop(
                        &self.run_context,
                        self.result_writer.as_ref(),
                        RuntimeOutcomeConversion {
                            input_ref: effective_input_ref,
                            invocation_id,
                            correlation_id,
                            requested_capability_id: &requested_capability_id,
                            outcome,
                        },
                    )
                    .await;
                    self.record_loop_completed(&idempotency_key, invocation_id, result.clone())?;
                    return result;
                }
                DispatchReservation::TerminalMilestonePending {
                    invocation_id,
                    result,
                    milestone,
                } => {
                    return self
                        .complete_terminal_milestone(
                            &idempotency_key,
                            invocation_id,
                            result,
                            Some(milestone),
                        )
                        .await;
                }
                DispatchReservation::LoopCompleted(result) => return result,
            }
        }

        // Any early `?` between reservation and `finish_runtime_outcome` unwinds
        // the in-flight reservation via the guard's `Drop`. The success path
        // calls `guard.commit()` so the dispatch record is replaced by
        // `finish_runtime_outcome` rather than cleared.
        let guard = self.dispatch_reservation_guard(&idempotency_key);

        let capability = match capability {
            SurfaceCapabilitySnapshot::Runtime(capability) => capability,
            SurfaceCapabilitySnapshot::Synthetic(capability) => {
                let result = self
                    .invoke_synthetic_capability(request, capability, snapshot)
                    .await;
                if result.is_ok() {
                    guard.commit();
                    self.record_loop_completed(
                        &idempotency_key,
                        requested_invocation_id,
                        result.clone(),
                    )?;
                }
                return result;
            }
        };

        let Some(trust_decision) = self
            .visible_request
            .provider_trust
            .get(&capability.provider)
            .cloned()
        else {
            return Ok(GatedResolution::bare(
                resolution::denied(
                    capability_denied_reason_kind("missing_provider_trust")?,
                    "capability provider trust is unavailable".to_string(),
                )
                .resolution,
            ));
        };
        let (input, estimate) = match resume_payload {
            // Host-side resume replay: reconstitute {input, estimate} from the
            // host-private payload loaded up front (keyed by the resume's
            // invocation id). `Some` iff this is a resume; a missing payload
            // already failed CLOSED above — never a silent empty-input dispatch
            // (arch-simplification §5.3 Stage 2a-i).
            Some(payload) => (payload.input, payload.estimate),
            Option::None => {
                let input = self
                    .input_resolver
                    .resolve_capability_input(&self.run_context, effective_input_ref)
                    .await?;
                // Trajectory capture: the resolved input is the model's tool
                // arguments, and this is the one place they are visible (the provider
                // tool-call decorator stages them upstream and bypasses the input
                // resolver hook).
                if let Some(observer) = &self.trajectory_observer {
                    // Best-effort, inline on the capability hot path: a panicking
                    // observer must never unwind the invocation before dispatch.
                    // (Blocking is the observer's own contract.)
                    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        observer.on_capability_input(
                            effective_input_ref.as_str(),
                            request.capability_id.as_str(),
                            &input,
                        );
                    }));
                    if caught.is_err() {
                        tracing::warn!(
                            capability_id = request.capability_id.as_str(),
                            "trajectory observer on_capability_input panicked; dropping event"
                        );
                    }
                }
                let input = match prepare_provider_arguments_with_detail(
                    &input,
                    &capability.parameters_schema,
                    "capability input",
                ) {
                    Ok(input) => input,
                    Err(error)
                        if error.error.kind == AgentLoopHostErrorKind::InvalidInvocation
                            && is_provider_tool_call_input_ref(effective_input_ref) =>
                    {
                        let host_error = *error.error;
                        let detail = error.detail.unwrap_or_else(|| {
                            diagnostic_detail_from_raw(&host_error.safe_summary)
                        });
                        let result = Ok(GatedResolution::bare(resolution::failed(
                            FailureKind::InputEncode,
                            host_error.safe_summary,
                            detail,
                        )));
                        guard.commit();
                        self.record_loop_completed(
                            &idempotency_key,
                            requested_invocation_id,
                            result.clone(),
                        )?;
                        return result;
                    }
                    Err(error) => return Err(*error.error),
                };
                // Runtime-specific request-shape validation belongs to the host
                // runtime. In particular, process-sandbox spawn and resume paths
                // return malformed plans as model-visible `InvalidInput` failures;
                // the mapper below then applies the canonical diagnostic scrubber.
                (input, capability.estimate.clone())
            }
        };
        let mut invocation_context =
            invocation_context_from_visible(VisibleInvocationContextRequest {
                base: &self.visible_request.context,
                run_context: &self.run_context,
                activity_id: request.activity_id,
                capability_id: &request.capability_id,
                capability: &capability,
                trust: trust_decision.effective_trust.class(),
                allowed_effects: &trust_decision.authority_ceiling.allowed_effects,
                execution_mounts: self.execution_mounts_for(&request.capability_id),
            })?;
        match &resume_mode {
            ResolvedResumeMode::Approval {
                resume,
                invocation_id: resume_invocation_id,
            } => {
                invocation_context.invocation_id = *resume_invocation_id;
                // Prefer the host-reconstituted correlation identity (the one the
                // approval lease is scoped to); the loop DTO's is advisory post-flip.
                invocation_context.correlation_id =
                    resume_correlation_id.unwrap_or(resume.correlation_id);
                invocation_context.resource_scope.invocation_id = *resume_invocation_id;
                invocation_context.validate().map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "capability approval resume context is invalid",
                    )
                })?;
            }
            ResolvedResumeMode::Auth {
                resume: auth_resume,
                invocation_id: resume_invocation_id,
            } => {
                // Reuse original invocation identifier so the fingerprinted
                // approval lease (scoped to that identifier) can still be matched
                // and claimed.
                invocation_context.invocation_id = *resume_invocation_id;
                invocation_context.resource_scope.invocation_id = *resume_invocation_id;
                // Restore the original correlation identifier so it flows through the
                // full capability lifecycle and matches any fingerprinted lease.
                // Prefer the host-reconstituted value from the replay payload (§5.3
                // Stage 2a-i); fall back to the wire prior-approval identity (kept on
                // the wire this slice, Stage 2a-ii).
                if let Some(correlation_id) = resume_correlation_id {
                    invocation_context.correlation_id = correlation_id;
                } else if let Some(pa) = auth_resume.prior_approval.as_ref() {
                    invocation_context.correlation_id = pa.correlation_id;
                }
                invocation_context.validate().map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "capability auth resume context is invalid",
                    )
                })?;
            }
            ResolvedResumeMode::None => {}
        }
        let invocation_id = invocation_context.invocation_id;
        let correlation_id = invocation_context.correlation_id;
        let requested_capability_id = request.capability_id.clone();
        let provider = capability.provider.clone();
        let runtime = capability.runtime;
        // Link this invocation to its staged input ref now that both are known,
        // so the still-running activity frame can surface the input argument
        // before the result completes.
        self.result_writer.record_running_invocation(
            &self.run_context,
            invocation_id,
            effective_input_ref,
        );
        let capability_activity_id = CapabilityActivityId::from_uuid(invocation_id.as_uuid());
        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        Box::pin(
            self.emit_capability_milestone(LoopHostMilestoneKind::CapabilityInvoked {
                activity_id: capability_activity_id,
                capability_id: request.capability_id.clone(),
            }),
        )
        .await?;
        // Only a FRESH dispatch mints a replay payload; an approval/auth resume
        // reuses the invocation id and its already-persisted payload (write-once),
        // so re-persisting would collide. Captured before `resume_mode` is
        // consumed by the dispatch match below.
        let is_fresh_dispatch = matches!(resume_mode, ResolvedResumeMode::None);
        let outcome = match resume_mode {
            ResolvedResumeMode::Approval { resume, .. } => {
                // Chain-boxing: each port delegation is boxed so the stacked
                // decorator chain never compiles into a single oversized poll
                // frame (see reborn_integration_model_recovery stack-overflow).
                Box::pin(dispatch_runtime_capability_resume(
                    self.runtime.as_ref(),
                    invocation_context,
                    resume.approval_request_id,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                ))
                .await
            }
            ResolvedResumeMode::Auth {
                resume: auth_resume,
                ..
            } => {
                let prior_approval_id = auth_resume
                    .prior_approval
                    .as_ref()
                    .map(|pa| pa.approval_request_id);
                tracing::debug!(
                    invocation_id = %invocation_id,
                    auth_resume = true,
                    approval_request_id = prior_approval_id.map(|id| id.to_string()).as_deref().unwrap_or("none"),
                    "capability auth-resume re-dispatch with preserved invocation identity"
                );
                // Chain-boxing: each port delegation is boxed so the stacked
                // decorator chain never compiles into a single oversized poll
                // frame (see reborn_integration_model_recovery stack-overflow).
                Box::pin(dispatch_runtime_capability_auth_resume(
                    self.runtime.as_ref(),
                    invocation_context,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                    prior_approval_id,
                ))
                .await
            }
            ResolvedResumeMode::None => {
                // Chain-boxing: each port delegation is boxed so the stacked
                // decorator chain never compiles into a single oversized poll
                // frame (see reborn_integration_model_recovery stack-overflow).
                Box::pin(dispatch_runtime_capability(
                    self.runtime.as_ref(),
                    invocation_context,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                ))
                .await
            }
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(HostRuntimeError::Unavailable { reason }) => {
                runtime_failed_outcome_for_host_runtime_unavailable(
                    requested_capability_id.clone(),
                    reason,
                )
            }
            Err(error) => {
                let host_error = host_runtime_error(error);
                let terminal_milestone = LoopHostMilestoneKind::CapabilityFailed {
                    activity_id: capability_activity_id,
                    capability_id: requested_capability_id.clone(),
                    provider: Some(provider),
                    runtime: Some(runtime),
                    reason_kind: host_error.kind.failure_kind(),
                    // Host/infra fault, not a model-visible tool error: keep the
                    // detail server-side, surface only the kind.
                    safe_summary: None,
                };
                guard.commit();
                return self
                    .complete_terminal_milestone(
                        &idempotency_key,
                        invocation_id,
                        Err(host_error),
                        Some(terminal_milestone),
                    )
                    .await;
            }
        };
        // Persist the host-private replay payload BEFORE returning the gate to the
        // loop, so a later resume turn can reconstitute {input, estimate} host-side
        // without the loop carrying raw tool args (arch-simplification §5.3 Stage
        // 2a-i; charter: agent-loop state never stores raw tool args). No-op unless
        // this is a fresh dispatch that produced an approval/auth gate.
        //
        // The dispatch reservation is committed only AFTER this fallible store
        // write succeeds (#6287 IronLoop): committing before it means a transient
        // store error would `?`-return with the reservation still `InFlight` and
        // the committed guard skipping its cleanup, stranding retries/duplicates
        // waiting on the key forever. On error the uncommitted guard clears the
        // reservation and wakes waiters so one re-dispatches.
        if is_fresh_dispatch {
            self.persist_replay_payload_for_fresh_gate(
                invocation_id,
                effective_input_ref,
                &input,
                &estimate,
                correlation_id,
                &outcome,
            )
            .await?;
        }
        guard.commit();
        self.finish_runtime_outcome(
            &idempotency_key,
            RuntimeOutcomeCompletion {
                input_ref: effective_input_ref,
                invocation_id,
                correlation_id,
                requested_capability_id: &requested_capability_id,
                provider,
                runtime,
                outcome,
            },
        )
        .await
    }
}

/// The [`GateRef`] a resolution channel renders its gate record from, when the
/// channel is gate-shaped. `Done`/`Denied` carry none; `Suspended(Process)`
/// tracks a process ref (no gate record) so it also answers `None`.
fn gate_ref_for_resolution(resolution: &Resolution) -> Option<GateRef> {
    match resolution {
        Resolution::Blocked(blocked) => Some(*blocked.gate_ref()),
        Resolution::Suspended(suspension) => suspension.gate_ref().copied(),
        Resolution::Done(_) | Resolution::Denied(_) => None,
    }
}

/// Map a gate-record store failure to a fail-closed host error. The bound cause
/// (which may carry a host path) is logged server-side at `warn` — a genuine
/// host storage fault operators must see — and never interpolated into the
/// model-visible summary (capability-access contract).
fn gate_record_store_error(error: ApprovalStoreError) -> AgentLoopHostError {
    tracing::warn!(error = %error, "failed to persist capability gate record at loop host seam");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "failed to persist capability gate record",
    )
}

/// Transitional default [`GateRecordStorePort`] used until composition wires a
/// durable store into the capability-port factory via
/// [`HostRuntimeLoopCapabilityPortFactory::with_gate_record_store`].
///
/// It is a deliberate no-op, not fail-closed: the persisted [`GateRecord`] has
/// **no consumer yet** — the resume-turn render path that loads it by `GateRef`
/// (and the loop-ref↔minted-ref association it needs) is the explicit follow-up
/// slice (this PR mints the ref at the seam; the association + read land next).
/// Until then, skipping the write changes no observable behavior, so an unwired
/// path keeps producing gates exactly as before rather than regressing them.
/// When the durable store is wired into every composition path, the follow-up
/// flips this default to fail-closed.
#[derive(Debug, Default)]
struct NoopGateRecordStore;

#[async_trait]
impl GateRecordStorePort for NoopGateRecordStore {
    async fn save(
        &self,
        _scope: ResourceScope,
        _gate_ref: GateRef,
        _record: GateRecord,
    ) -> Result<(), ApprovalStoreError> {
        // silent-ok: transitional no-op — the gate record has no reader until the
        // resume-read follow-up; skipping the durable write is behavior-preserving
        // and never regresses an unwired composition path's existing gates.
        tracing::debug!("gate record store not wired; skipping durable gate-record persistence");
        Ok(())
    }

    async fn load(
        &self,
        _scope: &ResourceScope,
        _gate_ref: GateRef,
    ) -> Result<Option<GateRecord>, ApprovalStoreError> {
        Ok(None)
    }
}

/// Map a replay-payload store failure to a fail-closed host error. The bound
/// cause (which may carry a host path) is logged server-side at `warn` — a
/// genuine host storage fault operators must see — and never interpolated into
/// the model-visible summary (capability-access contract). Mirrors
/// `gate_record_store_error`.
fn replay_payload_store_error(error: ReplayPayloadStoreError) -> AgentLoopHostError {
    tracing::warn!(error = %error, "failed to persist/load capability replay payload at loop host seam");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "failed to access capability replay payload",
    )
}

/// Transitional fail-closed default [`ReplayPayloadStorePort`] used until composition
/// wires a durable store into the capability-port factory via
/// [`HostRuntimeLoopCapabilityPortFactory::with_replay_payload_store`].
///
/// Unlike [`NoopGateRecordStore`] this is deliberately fail-closed on read: the
/// replay payload has a real consumer (the resume-read path,
/// `replay_payload_for_resume`), so an unwired store that silently returned an
/// empty payload would dispatch a resume with the WRONG (empty) input. `save`
/// no-ops (an unwired factory persists nothing) and `load` returns `Ok(None)`,
/// which the resume-read path treats as a sanitized terminal failure.
#[derive(Debug, Default)]
struct NoopReplayPayloadStore;

#[async_trait]
impl ReplayPayloadStorePort for NoopReplayPayloadStore {
    async fn save(
        &self,
        _scope: ResourceScope,
        _invocation_id: InvocationId,
        _payload: ReplayPayload,
    ) -> Result<(), ReplayPayloadStoreError> {
        // silent-ok: transitional no-op — an unwired factory persists nothing; the
        // fail-closed `load` below turns any resume that needs a payload into a
        // sanitized terminal failure rather than a silent empty-input dispatch.
        tracing::debug!(
            "replay payload store not wired; skipping durable replay-payload persistence"
        );
        Ok(())
    }

    async fn load(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<ReplayPayload>, ReplayPayloadStoreError> {
        Ok(None)
    }
}

async fn dispatch_runtime_capability(
    runtime: &(dyn HostRuntime + Send + Sync),
    context: ExecutionContext,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    if is_process_sandbox_capability(&capability_id) {
        runtime
            .spawn_capability((context, capability_id, estimate, input))
            .await
    } else {
        runtime
            .invoke_capability((context, capability_id, estimate, input))
            .await
    }
}

async fn dispatch_runtime_capability_resume(
    runtime: &(dyn HostRuntime + Send + Sync),
    context: ExecutionContext,
    approval_request_id: ApprovalRequestId,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    if is_process_sandbox_capability(&capability_id) {
        runtime
            .resume_spawn_capability((context, approval_request_id, capability_id, estimate, input))
            .await
    } else {
        runtime
            .resume_capability((context, approval_request_id, capability_id, estimate, input))
            .await
    }
}

/// Auth-resume dispatch: always uses `auth_resume_capability` (no spawn
/// variant; sandbox spawns do not go through approval/auth gates).
async fn dispatch_runtime_capability_auth_resume(
    runtime: &(dyn HostRuntime + Send + Sync),
    context: ExecutionContext,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    approval_request_id: Option<ApprovalRequestId>,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    runtime
        .auth_resume_capability((context, capability_id, estimate, input, approval_request_id))
        .await
}

async fn dispatch_runtime_capability_auth_decline(
    runtime: &(dyn HostRuntime + Send + Sync),
    context: ExecutionContext,
    capability_id: CapabilityId,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    runtime
        .decline_auth_capability((context, capability_id))
        .await
}

fn is_process_sandbox_capability(capability_id: &CapabilityId) -> bool {
    capability_id.as_str() == ironclaw_host_api::capability::PROCESS_SANDBOX_CAPABILITY_ID
}

fn provider_schema_is_resolved(schema: &serde_json::Value) -> bool {
    // The hot capability catalog owns canonical JSON Schema validation and the
    // selected LLM provider owns wire-format shaping (including top-level
    // `oneOf` flattening). The loop must not maintain a second, narrower schema
    // dialect: doing so silently delists valid, versioned extension schemas
    // before they reach the provider adapter. It only enforces the boundary the
    // provider cannot resolve safely on its own.
    !schema_contains_external_ref(schema, 0)
}

fn provider_tool_name(
    capability_id: &CapabilityId,
    existing: &HashMap<ProviderToolName, CapabilityId>,
) -> ProviderToolName {
    let base = provider_tool_name_base(capability_id.as_str());
    if let Ok(name) = ProviderToolName::new(base.clone())
        && existing
            .get(&name)
            .is_none_or(|existing_id| existing_id == capability_id)
    {
        return name;
    }
    provider_tool_name_with_digest(&base, capability_id.as_str(), existing, 0)
}

fn provider_tool_name_with_digest(
    base: &str,
    capability_id: &str,
    existing: &HashMap<ProviderToolName, CapabilityId>,
    attempt: u16,
) -> ProviderToolName {
    let digest_input = if attempt == 0 {
        capability_id.to_string()
    } else {
        format!("{capability_id}#{attempt}")
    };
    let digest = sha256_digest_token(digest_input.as_bytes());
    let suffix = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let suffix = &suffix[..PROVIDER_TOOL_NAME_DIGEST_BYTES]; // safety: sha256 hex digest is ASCII and longer than the fixed suffix.
    let prefix_len = PROVIDER_TOOL_NAME_MAX_BYTES.saturating_sub("__".len() + suffix.len());
    let prefix = if base.len() <= prefix_len {
        base
    } else {
        let prefix_end = base
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= prefix_len)
            .last()
            .unwrap_or(0);
        &base[..prefix_end] // safety: prefix_end comes from char_indices(), so it is a UTF-8 boundary.
    };
    let candidate = format!("{prefix}__{suffix}");
    let candidate = ProviderToolName::new(candidate)
        .expect("provider tool name generator must produce provider-safe names"); // safety: `prefix` is sanitized and `suffix` is a fixed ASCII hex digest slice.
    if existing
        .get(&candidate)
        .is_none_or(|existing_id| existing_id.as_str() == capability_id)
        || attempt == u16::MAX
    {
        return candidate;
    }
    provider_tool_name_with_digest(base, capability_id, existing, attempt + 1)
}

fn provider_tool_name_base(capability_id: &str) -> String {
    let mut name = String::with_capacity(capability_id.len());
    for character in capability_id.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
            name.push(character);
        } else if character == '.' {
            name.push_str("__");
        } else {
            name.push('_');
        }
    }
    if name.is_empty() {
        "tool".to_string()
    } else {
        name
    }
}

fn should_retry_result_write(
    outcome: &RuntimeCapabilityOutcome,
    result: &Result<GatedResolution, AgentLoopHostError>,
) -> bool {
    matches!(outcome, RuntimeCapabilityOutcome::Completed(_))
        && matches!(
            result,
            Err(error)
                if matches!(
                    error.kind,
                    AgentLoopHostErrorKind::Unavailable
                        | AgentLoopHostErrorKind::TranscriptWriteFailed
                )
        )
}

struct VisibleInvocationContextRequest<'a> {
    base: &'a ExecutionContext,
    run_context: &'a LoopRunContext,
    activity_id: CapabilityActivityId,
    capability_id: &'a CapabilityId,
    capability: &'a RuntimeSurfaceCapabilitySnapshot,
    trust: ironclaw_host_api::runtime::TrustClass,
    allowed_effects: &'a [EffectKind],
    execution_mounts: &'a MountView,
}

fn invocation_context_from_visible(
    request: VisibleInvocationContextRequest<'_>,
) -> Result<ExecutionContext, AgentLoopHostError> {
    let mut context =
        auth_decline_context_from_visible(request.base, request.run_context, request.activity_id)?;
    let loop_driver_extension = context.extension_id.clone();
    context.runtime = request.capability.runtime;
    context.trust = request.trust;
    context.grants = invocation_grants_from_visible(
        request.base,
        request.capability_id,
        &loop_driver_extension,
        request.allowed_effects,
    )?;
    // Mount propagation is host-authority only: visible-request contexts must arrive with no
    // caller-supplied mounts, while this invocation context receives the execution mounts that the
    // authority resolver selected for the run and capability dispatch.
    context.mounts = request.execution_mounts.clone();
    context.validate().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability execution context is invalid",
        )
    })?;
    Ok(context)
}

/// Reconstruct only the host-sealed identity required to terminalize a denied
/// auth gate. This deliberately does not consult the current capability
/// surface, provider trust, grants, mounts, or input: the admitted invocation's
/// durable `BlockedAuth` record is the authority, and `CapabilityHost` validates
/// its exact scope, actor, activity, and capability before mutation.
fn auth_decline_context_from_visible(
    base: &ExecutionContext,
    run_context: &LoopRunContext,
    activity_id: CapabilityActivityId,
) -> Result<ExecutionContext, AgentLoopHostError> {
    let mut context = base.clone();
    context.extension_id = loop_driver_execution_extension_id(run_context)?;
    let invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
    context.invocation_id = invocation_id;
    context.correlation_id = CorrelationId::new();
    context.process_id = None;
    context.parent_process_id = None;
    context.resource_scope.invocation_id = invocation_id;
    // Prompt-visible run identity: tool calls within the same turn-run share
    // it, so run-scoped policy state (e.g. coding read-before-edit) carries
    // across tool calls of one run but never leaks into a later run.
    let run_id = ironclaw_host_api::ids::RunId::from_uuid(run_context.run_id.as_uuid());
    context.run_id = Some(run_id);
    // Authoritative origin (§5.2.1): a tool call inside an agent loop turn-run is
    // model-initiated, so the loop ingress seals `LoopRun`. The kernel would also
    // reconstruct this from `run_id`, but stamping `origin` explicitly makes the
    // loop the authoritative source rather than relying on the compat fallback.
    context.origin = Some(
        match run_context
            .product_context
            .as_ref()
            .map(|product_context| product_context.origin)
        {
            Some(ironclaw_turns::TurnOriginKind::ScheduledTrigger) => {
                InvocationOrigin::ScheduledLoopRun(run_id)
            }
            Some(ironclaw_turns::TurnOriginKind::WebUi)
            | Some(ironclaw_turns::TurnOriginKind::Inbound)
            | None => InvocationOrigin::LoopRun(run_id),
        },
    );
    context.authenticated_actor_user_id = run_context.actor().map(|actor| actor.user_id.clone());
    context.validate().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability execution context is invalid",
        )
    })?;
    Ok(context)
}

/// Derives the execution extension id for a loop driver.
///
/// Valid extension ids are preserved as-is. Other loop-driver ids are sanitized into a lowercase
/// slug, truncated to leave room for entropy, and suffixed with a digest fragment so separators,
/// case changes, non-ASCII input, and other slug collisions remain distinct.
pub fn loop_driver_execution_extension_id(
    run_context: &LoopRunContext,
) -> Result<ExtensionId, AgentLoopHostError> {
    let raw = run_context.loop_driver_id.as_str();
    if let Ok(extension_id) = ExtensionId::new(raw) {
        return Ok(extension_id);
    }

    let digest = sha256_digest_token(raw.as_bytes());
    let digest_hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let slug = extension_id_slug(raw);
    let prefix_budget = 128usize
        .saturating_sub("loop-driver-".len())
        .saturating_sub("-".len())
        .saturating_sub(16);
    let mut candidate = slug.chars().take(prefix_budget).collect::<String>();
    if candidate.is_empty() {
        candidate.push_str("driver");
    }
    ExtensionId::new(format!("loop-driver-{candidate}-{}", &digest_hex[..16])).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "loop driver id could not be represented as an execution extension",
        )
    })
}

fn extension_id_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_separator = false;
    for byte in value.bytes() {
        let next = match byte {
            b'a'..=b'z' | b'0'..=b'9' => {
                last_separator = false;
                byte as char
            }
            b'A'..=b'Z' => {
                last_separator = false;
                byte.to_ascii_lowercase() as char
            }
            b'_' | b'-' => {
                if last_separator {
                    continue;
                }
                last_separator = true;
                '-'
            }
            b'.' => {
                if slug.is_empty() || last_separator {
                    continue;
                }
                last_separator = true;
                '.'
            }
            _ => {
                if last_separator {
                    continue;
                }
                last_separator = true;
                '-'
            }
        };
        slug.push(next);
    }
    while slug.ends_with(['-', '.']) {
        slug.pop();
    }
    if slug
        .as_bytes()
        .first()
        .is_none_or(|first| !(first.is_ascii_lowercase() || first.is_ascii_digit()))
    {
        slug.insert_str(0, "driver");
    }
    slug
}

fn invocation_grants_from_visible(
    base: &ExecutionContext,
    capability_id: &CapabilityId,
    loop_driver_extension: &ExtensionId,
    allowed_effects: &[EffectKind],
) -> Result<CapabilitySet, AgentLoopHostError> {
    let mut filtered = CapabilitySet::default();
    for grant in &base.grants.grants {
        if grant.capability != *capability_id {
            continue;
        }
        if !grant_principal_matches_visible_context(&grant.grantee, base, loop_driver_extension)
            || !matches!(grant.issued_by, Principal::HostRuntime)
            || !effects_are_covered(&grant.constraints.allowed_effects, allowed_effects)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unauthorized,
                "capability execution context carries an untrusted grant",
            ));
        }
        filtered.grants.push(grant.clone());
    }
    Ok(filtered)
}

fn grant_principal_matches_visible_context(
    principal: &Principal,
    context: &ExecutionContext,
    loop_driver_extension: &ExtensionId,
) -> bool {
    match principal {
        Principal::Tenant(id) => id == &context.tenant_id,
        Principal::User(id) => id == &context.user_id,
        Principal::Agent(id) => context.agent_id.as_ref() == Some(id),
        Principal::Project(id) => context.project_id.as_ref() == Some(id),
        Principal::Mission(id) => context.mission_id.as_ref() == Some(id),
        Principal::Thread(id) => context.thread_id.as_ref() == Some(id),
        Principal::Extension(id) => id == loop_driver_extension,
        Principal::HostRuntime | Principal::System(_) => false,
    }
}

fn effects_are_covered(required: &[EffectKind], allowed: &[EffectKind]) -> bool {
    required.iter().all(|effect| allowed.contains(effect))
}

fn invocation_idempotency_key(
    run_context: &LoopRunContext,
    request: &LoopRequest,
    input_ref: &CapabilityInputRef,
) -> Result<IdempotencyKey, AgentLoopHostError> {
    // Each mode must hash to a distinct key: a colliding key would replay the
    // prior mode's recorded outcome (e.g. an auth re-dispatch receiving the
    // original cached ApprovalRequired gate) instead of dispatching.
    let resume_scope = match (
        request.approval_resume.as_ref(),
        request.auth_resume.as_ref(),
    ) {
        (Some(resume), _) => format!(
            "resume:{}:{}",
            resume.approval_request_id, resume.resume_token
        ),
        (None, Some(auth_resume))
            if matches!(
                auth_resume.disposition,
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            ) =>
        {
            "auth-denied".to_string()
        }
        (None, Some(auth_resume)) => {
            let resume_token = auth_resume
                .resume_token
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "missing".to_string());
            format!(
                "auth-resume:{}:{}",
                auth_resume
                    .prior_approval
                    .as_ref()
                    .map(|pa| pa.approval_request_id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                resume_token
            )
        }
        (None, None) => "dispatch".to_string(),
    };
    let payload = format!(
        "loop-capability\nrun={}\nsurface={}\ncapability={}\ninput={}\nmode={}",
        run_context.run_id,
        request.surface_version.as_str(),
        request.capability_id.as_str(),
        input_ref.as_str(),
        resume_scope
    );
    IdempotencyKey::new(format!(
        "loop-capability:{}",
        sha256_digest_token(payload.as_bytes())
    ))
    .map_err(host_runtime_error)
}

fn auth_decline_idempotency_key(
    run_context: &LoopRunContext,
    activity_id: CapabilityActivityId,
    invocation_id: InvocationId,
    capability_id: &CapabilityId,
) -> Result<IdempotencyKey, AgentLoopHostError> {
    // Auth denial terminalizes an already-admitted durable invocation. Its
    // replay identity must therefore remain stable when the current surface or
    // input reference changes after the invocation entered BlockedAuth.
    let payload = format!(
        "loop-capability-auth-decline\nrun={}\nactivity={}\ninvocation={}\ncapability={}\nmode=auth-denied",
        run_context.run_id,
        activity_id,
        invocation_id,
        capability_id.as_str(),
    );
    IdempotencyKey::new(format!(
        "loop-capability:{}",
        sha256_digest_token(payload.as_bytes())
    ))
    .map_err(host_runtime_error)
}

fn provider_tool_call_input_ref(
    run_context: &LoopRunContext,
    tool_call: &ProviderToolCall,
) -> Result<CapabilityInputRef, AgentLoopHostError> {
    let turn_id = tool_call.turn_id.as_deref().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool call is missing a provider turn id",
        )
    })?;
    let arguments = serde_json::to_string(&tool_call.arguments).map_err(|error| {
        let safe_summary = error.to_string();
        crate::raw_agent_loop_host_error(
            "capability_provider_tool_call",
            "serialize_arguments",
            AgentLoopHostErrorKind::InvalidInvocation,
            safe_summary,
            error,
        )
    })?;
    let payload = format!(
        "provider-tool-input\nrun={}\nprovider={}\nmodel={}\nturn={}\ncall={}\ntool={}\narguments={}",
        run_context.run_id,
        tool_call.provider_id,
        tool_call.provider_model_id,
        turn_id,
        tool_call.id,
        tool_call.name,
        arguments
    );
    let digest = sha256_digest_token(payload.as_bytes());
    let digest = digest.strip_prefix("sha256:").unwrap_or(&digest);
    CapabilityInputRef::new(format!("{PROVIDER_TOOL_CALL_INPUT_REF_PREFIX}{digest}")).map_err(
        |_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "provider tool-call input ref could not be represented",
            )
        },
    )
}

fn is_provider_tool_call_input_ref(input_ref: &CapabilityInputRef) -> bool {
    input_ref
        .as_str()
        .starts_with(PROVIDER_TOOL_CALL_INPUT_REF_PREFIX)
}

fn loop_surface_version(
    version: &str,
) -> Result<ironclaw_loop_contracts::CapabilitySurfaceVersion, AgentLoopHostError> {
    ironclaw_loop_contracts::CapabilitySurfaceVersion::new(version).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "host runtime capability surface version could not be represented",
        )
    })
}

async fn runtime_outcome_to_loop(
    run_context: &LoopRunContext,
    result_writer: &(dyn LoopCapabilityResultWriter + Send + Sync),
    conversion: RuntimeOutcomeConversion<'_>,
) -> Result<GatedResolution, AgentLoopHostError> {
    ensure_runtime_outcome_matches(conversion.requested_capability_id, &conversion.outcome)?;
    Ok(match conversion.outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            let write_result = result_writer
                .write_capability_result(CapabilityResultWrite {
                    run_context,
                    input_ref: conversion.input_ref,
                    invocation_id: conversion.invocation_id,
                    capability_id: &completed.capability_id,
                    output: completed.output.clone(),
                    display_preview: completed.display_preview.clone(),
                    durable_persistence: DurablePersistence::Persist,
                })
                .await?;
            GatedResolution::bare(resolution::completed(
                write_result.result_ref,
                "capability completed".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                false,
                write_result.byte_len,
                write_result.output_digest,
                write_result.model_observation,
            ))
        }
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            // Raw input/estimate no longer ride the loop-facing resolution; the
            // host persists them in the replay-payload store at the fresh gate
            // raise (see `persist_replay_payload_for_fresh_gate`) and reconstitutes
            // them on resume (arch-simplification §5.3 Stage 2a-i).
            resolution::approval_required(
                loop_gate_ref("approval", gate.approval_request_id.to_string())?,
                blocked_summary(gate.reason).to_string(),
                Some(ironclaw_loop_contracts::CapabilityApprovalResume {
                    approval_request_id: gate.approval_request_id,
                    resume_token: resume_token_from_invocation_id(conversion.invocation_id)?,
                    correlation_id: conversion.correlation_id,
                    input_ref: conversion.input_ref.clone(),
                }),
            )
        }
        RuntimeCapabilityOutcome::AuthRequired(gate) => resolution::auth_required(
            loop_gate_ref("auth", gate.gate_id.to_string())?,
            gate.credential_requirements,
            blocked_summary(gate.reason).to_string(),
            Some(ironclaw_loop_contracts::CapabilityAuthResume::resolved(
                resume_token_from_invocation_id(conversion.invocation_id)?,
                None,
            )),
        ),
        RuntimeCapabilityOutcome::ResourceBlocked(gate) => resolution::resource_blocked(
            loop_gate_ref("resource", gate.gate_id.to_string())?,
            blocked_summary(gate.reason).to_string(),
        ),
        RuntimeCapabilityOutcome::SpawnedProcess(process) => {
            GatedResolution::bare(resolution::spawned_process(
                LoopProcessRef::new(format!("process:{}", process.process_id)).map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "process ref could not be represented",
                    )
                })?,
            ))
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            let capability_id = failure.capability_id.clone();
            let class = runtime_failure_to_loop(failure)?;
            // Surface actionable failure detail (e.g. invalid-input field issues)
            // to the per-tool UI preview by staging a display-preview record.
            // Without this the projection falls back to the bare error kind. The
            // model-visible observation is unaffected.
            if let LoopFailureClass::Failed {
                safe_summary,
                detail,
                ..
            } = &class
                && let Some(summary) = failure_display_summary(safe_summary, detail)
            {
                result_writer
                    .stage_capability_failure_preview(
                        run_context,
                        conversion.invocation_id,
                        &capability_id,
                        &summary,
                    )
                    .await;
            }
            GatedResolution::bare(class.into_resolution())
        }
    })
}

/// A runtime failure classified onto its loop channel — either a model-visible
/// recoverable failure or a terminal denial. Private to the seam: the failure
/// path needs the raw fields both to build the `Resolution` (via the producer
/// constructors) and to stage the per-tool display preview.
enum LoopFailureClass {
    Failed {
        error_kind: FailureKind,
        safe_summary: String,
        detail: CapabilityFailureDetail,
    },
    Denied {
        reason_kind: CapabilityDeniedReasonKind,
        safe_summary: String,
    },
}

impl LoopFailureClass {
    fn into_resolution(self) -> Resolution {
        match self {
            LoopFailureClass::Failed {
                error_kind,
                safe_summary,
                detail,
            } => resolution::failed(error_kind, safe_summary, detail),
            LoopFailureClass::Denied {
                reason_kind,
                safe_summary,
            } => resolution::denied(reason_kind, safe_summary).resolution,
        }
    }
}

fn runtime_terminal_milestone(
    activity_id: CapabilityActivityId,
    provider: ExtensionId,
    runtime: RuntimeKind,
    outcome: &RuntimeCapabilityOutcome,
) -> Result<Option<LoopHostMilestoneKind>, AgentLoopHostError> {
    Ok(match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            Some(LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id: completed.capability_id.clone(),
                provider,
                runtime,
                output_bytes: completed.usage.output_bytes,
            })
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            let safe_summary = runtime_failure_loop_safe_summary(failure);
            Some(LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: failure.capability_id.clone(),
                provider: Some(provider),
                runtime: Some(runtime),
                reason_kind: failure.kind,
                // Sanitized, host-authored message (e.g. "invalid JSON: ...")
                // so the live per-tool UI card shows the real reason, not just
                // the bare error kind.
                safe_summary,
            })
        }
        RuntimeCapabilityOutcome::ApprovalRequired(_)
        | RuntimeCapabilityOutcome::AuthRequired(_)
        | RuntimeCapabilityOutcome::ResourceBlocked(_)
        | RuntimeCapabilityOutcome::SpawnedProcess(_) => None,
    })
}

fn runtime_failure_to_loop(
    failure: RuntimeCapabilityFailure,
) -> Result<LoopFailureClass, AgentLoopHostError> {
    match failure.disposition() {
        CapabilityFailureDisposition::ModelVisibleToolError => {
            runtime_model_visible_failure_to_loop(failure)
        }
        CapabilityFailureDisposition::RetrySameCall => {
            let detail = runtime_failure_detail_to_loop(failure.detail.clone())
                .unwrap_or_else(|| runtime_failure_diagnostic_detail(&failure));
            Ok(LoopFailureClass::Failed {
                error_kind: failure.kind,
                safe_summary: runtime_failure_safe_summary(
                    &failure,
                    "capability invocation failed",
                ),
                detail,
            })
        }
    }
}

/// Build a model-visible, hardened diagnostic from a runtime failure's raw
/// message when the failure has no structured detail. Preserves the real cause
/// (paths, schema refs, codes) that the strict safe-summary validator drops,
/// while redacting secret VALUES through the full leak-detector registry +
/// prefix matcher and fencing any surviving injection payload
/// ([`crate::scrub_model_visible_detail`]).
fn runtime_failure_diagnostic_detail(
    failure: &RuntimeCapabilityFailure,
) -> CapabilityFailureDetail {
    // Prefer the private in-process cause channel: the public `message` fails
    // closed (kind-only for wild raw causes), so the full descriptive cause
    // rides `model_visible_cause` and only becomes model-visible through this
    // scrub (full registry + injection fencing).
    let raw = failure
        .model_visible_cause()
        .map(str::to_owned)
        .or_else(|| failure.safe_summary());
    let text = raw
        .as_deref()
        .and_then(|raw| {
            if failure.kind == FailureKind::InputEncode
                && is_process_sandbox_capability(&failure.capability_id)
            {
                sandbox_model_visible_diagnostic_text(raw)
            } else {
                model_visible_diagnostic_text(raw)
            }
        })
        .unwrap_or_else(|| ModelDiagnostic::unavailable().into_inner());
    CapabilityFailureDetail::Diagnostic { text }
}

/// Sandbox validation diagnostics still cross the legacy host-api verdict
/// boundary as a `SafeSummary`. Apply the full secret scrub and injection fence
/// first, then normalize only the delimiters that boundary rejects. This keeps
/// corrective detail model-visible without allowing credentials or bare
/// instructions through, and preserves the previous 400-byte budget.
fn sandbox_model_visible_diagnostic_text(raw: &str) -> Option<String> {
    const MAX_BYTES: usize = 400;

    let scrubbed = crate::model_visible_scrub::scrub_model_visible_detail_compact(raw);
    let normalized: String = scrubbed
        .chars()
        .map(|character| match character {
            '`' => '\'',
            '{' | '}' | '[' | ']' | '<' | '>' | '/' | '\\' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect();
    let mut text = normalized.trim().to_string();
    if text.len() > MAX_BYTES {
        let mut end = MAX_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    if text.is_empty() { None } else { Some(text) }
}

/// Prepare free text for the model-visible diagnostic channel: scrub secret
/// values through the full registry and prefix matcher, fence surviving prompt
/// injection text, and replace control characters the model-observation
/// validator rejects (everything but `\n`, `\r`, `\t`) with spaces. This keeps
/// one stray escape byte from invalidating — and thereby dropping — the whole
/// observation.
fn model_visible_diagnostic_text(raw: &str) -> Option<String> {
    let scrubbed = crate::scrub_model_visible_detail(raw);
    let normalized: String = scrubbed
        .chars()
        .map(|character| {
            if character == '\0'
                || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    if normalized.trim().is_empty() {
        return None;
    }
    Some(normalized)
}

fn diagnostic_detail_from_raw(raw: &str) -> CapabilityFailureDetail {
    let text = model_visible_diagnostic_text(raw)
        .unwrap_or_else(|| ModelDiagnostic::unavailable().into_inner());
    CapabilityFailureDetail::Diagnostic { text }
}

fn runtime_model_visible_failure_to_loop(
    failure: RuntimeCapabilityFailure,
) -> Result<LoopFailureClass, AgentLoopHostError> {
    if matches!(
        failure.kind,
        FailureKind::Authorization | FailureKind::PolicyDenied
    ) {
        return Ok(LoopFailureClass::Denied {
            reason_kind: denied_reason_kind_for(failure.kind)?,
            safe_summary: runtime_failure_safe_summary(&failure, "capability authorization denied"),
        });
    }

    let error_kind = failure.kind;
    let safe_summary = runtime_failure_safe_summary(&failure, "capability invocation failed");
    let detail = runtime_failure_detail_to_loop(failure.detail.clone())
        .unwrap_or_else(|| runtime_failure_diagnostic_detail(&failure));
    Ok(LoopFailureClass::Failed {
        error_kind,
        safe_summary,
        detail,
    })
}

fn runtime_failure_detail_to_loop(
    detail: Option<DispatchFailureDetail>,
) -> Option<CapabilityFailureDetail> {
    detail.and_then(dispatch_failure_detail_to_loop)
}

fn dispatch_failure_detail_to_loop(
    detail: DispatchFailureDetail,
) -> Option<CapabilityFailureDetail> {
    match detail {
        DispatchFailureDetail::InvalidInput { issues } => {
            Some(CapabilityFailureDetail::InvalidInput {
                issues: issues
                    .into_iter()
                    .map(dispatch_input_issue_to_loop)
                    .collect(),
            })
        }
        // Raw failure cause the host runtime preserved because the strict
        // safe-summary validator rejected it (paths, newlines). Scrub secret
        // values and normalize control characters before the model sees it.
        DispatchFailureDetail::Diagnostic { text } => model_visible_diagnostic_text(&text)
            .map(|text| CapabilityFailureDetail::Diagnostic { text }),
        // Host-authored remediation: already validated at construction (bounded,
        // newline-only control characters, credential-VALUE shapes rejected), so
        // it passes through verbatim. Running it through
        // `model_visible_diagnostic_text` would be a no-op at best and a
        // vocabulary scrub at worst — the text NAMES config keys on purpose.
        DispatchFailureDetail::HostRemediation { text } => {
            Some(CapabilityFailureDetail::HostRemediation { text })
        }
    }
}

fn dispatch_input_issue_to_loop(issue: DispatchInputIssue) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path: issue.path,
        code: issue.code,
        expected: issue.expected,
        received: issue.received,
        schema_path: issue.schema_path,
    }
}

fn runtime_failed_outcome_for_host_runtime_unavailable(
    capability_id: CapabilityId,
    reason: String,
) -> RuntimeCapabilityOutcome {
    let host_error = host_runtime_error(HostRuntimeError::Unavailable { reason });
    RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure::new(
        capability_id,
        FailureKind::Unavailable,
        Some(host_error.safe_summary),
    ))
}

fn ensure_runtime_outcome_matches(
    expected: &CapabilityId,
    outcome: &RuntimeCapabilityOutcome,
) -> Result<(), AgentLoopHostError> {
    let actual = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => &completed.capability_id,
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::AuthRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::ResourceBlocked(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::SpawnedProcess(process) => &process.capability_id,
        RuntimeCapabilityOutcome::Failed(failure) => &failure.capability_id,
    };
    if actual != expected {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "host runtime returned outcome for a different capability",
        ));
    }
    Ok(())
}

/// Maps an authorization/policy runtime failure to a leak-safe denied reason
/// identifier.
///
/// `FailureKind::Authorization.as_str()` is the literal string
/// `"authorization"`, which the loop-safe identifier validator rejects as a
/// sensitive marker (it guards against leaking `Authorization:` header
/// material into identifiers). Passing it straight into
/// `capability_denied_reason_kind` therefore turned every authorization denial
/// into an internal "could not be represented" error, which the executor
/// mapped to `HostUnavailable` and the planned driver recorded as a terminal
/// "driver unavailable" failure — borking the whole run (observed when a Gmail
/// extension activation failed authorization on auth-resume). Use stable,
/// non-leaky tags so the denial surfaces to the model as a clean `Denied`
/// outcome instead.
fn denied_reason_kind_for(
    kind: FailureKind,
) -> Result<CapabilityDeniedReasonKind, AgentLoopHostError> {
    let reason = match kind {
        FailureKind::Authorization => "auth_denied",
        FailureKind::PolicyDenied => "policy_denied",
        other => other.as_str(),
    };
    capability_denied_reason_kind(reason)
}

fn capability_denied_reason_kind(
    value: impl Into<String>,
) -> Result<CapabilityDeniedReasonKind, AgentLoopHostError> {
    CapabilityDeniedReasonKind::unknown(value).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability denied reason kind could not be represented",
        )
    })
}

fn runtime_safe_summary(message: Option<String>, fallback: &'static str) -> String {
    message
        .and_then(|summary| LoopSafeSummary::new(summary).ok())
        .map(|summary| summary.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn runtime_failure_safe_summary(
    failure: &RuntimeCapabilityFailure,
    fallback: &'static str,
) -> String {
    let fallback = runtime_failure_fallback_summary(failure.kind, fallback);
    failure
        .safe_summary()
        .and_then(|summary| LoopSafeSummary::new(summary).ok())
        .map(|summary| summary.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn runtime_failure_loop_safe_summary(
    failure: &RuntimeCapabilityFailure,
) -> Option<LoopSafeSummary> {
    match failure.safe_summary() {
        Some(summary) => {
            if let Ok(summary) = LoopSafeSummary::new(summary.clone()) {
                return Some(summary);
            }
            if matches!(failure.kind, FailureKind::InputEncode) {
                return Some(runtime_input_encode_summary());
            }
            Some(LoopSafeSummary::capability_failure_summary(summary))
        }
        None if matches!(failure.kind, FailureKind::InputEncode) => {
            Some(runtime_input_encode_summary())
        }
        None => None,
    }
}

fn runtime_failure_fallback_summary(kind: FailureKind, fallback: &'static str) -> &'static str {
    if matches!(kind, FailureKind::InputEncode) {
        RuntimeDispatchErrorKind::InputEncode.human_summary()
    } else {
        fallback
    }
}

fn runtime_input_encode_summary() -> LoopSafeSummary {
    LoopSafeSummary::tool_input_could_not_be_encoded()
}

fn loop_gate_ref(kind: &str, id: String) -> Result<LoopGateRef, AgentLoopHostError> {
    LoopGateRef::new(format!("gate:{kind}-{id}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability gate ref could not be represented",
        )
    })
}

fn blocked_summary(reason: RuntimeBlockedReason) -> &'static str {
    match reason {
        RuntimeBlockedReason::ApprovalRequired => "capability requires approval",
        RuntimeBlockedReason::AuthRequired => "capability requires authentication",
        RuntimeBlockedReason::ResourceLimit => "capability is blocked by resource limits",
        RuntimeBlockedReason::ResourceUnavailable => "capability resources are unavailable",
    }
}

fn resume_token_from_invocation_id(
    invocation_id: InvocationId,
) -> Result<CapabilityResumeToken, AgentLoopHostError> {
    CapabilityResumeToken::new(invocation_id.to_string()).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("capability resume token is invalid: {reason}"),
        )
    })
}

fn invocation_id_from_resume_token(
    resume_token: &CapabilityResumeToken,
) -> Result<InvocationId, AgentLoopHostError> {
    InvocationId::parse(resume_token.as_str()).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability approval resume token is invalid",
        )
    })
}

fn ensure_resume_invocation_matches_activity(
    resume_invocation_id: InvocationId,
    requested_invocation_id: InvocationId,
    resume_kind: &'static str,
) -> Result<(), AgentLoopHostError> {
    if resume_invocation_id == requested_invocation_id {
        return Ok(());
    }
    Err(AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!("capability {resume_kind} resume activity identity does not match resume token"),
    ))
}

fn host_runtime_error(error: HostRuntimeError) -> AgentLoopHostError {
    match error {
        HostRuntimeError::InvalidRequest { reason } => crate::raw_agent_loop_host_error(
            "host_runtime_capability",
            "invoke",
            AgentLoopHostErrorKind::InvalidInvocation,
            runtime_safe_summary(
                Some(reason.clone()),
                "host runtime rejected capability request",
            ),
            reason,
        ),
        HostRuntimeError::Unavailable { reason } => crate::raw_agent_loop_host_error(
            "host_runtime_capability",
            "invoke",
            AgentLoopHostErrorKind::Unavailable,
            runtime_safe_summary(
                Some(reason.clone()),
                "host runtime capability service is unavailable",
            ),
            reason,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod capability_info_tests;
    mod decorators;
    mod failure_display;
    mod gates;
    mod invocation_context;
    mod provider_arguments;
    mod provider_dispatch;
    mod provider_registration;
    mod provider_schema;
    mod resume;
    mod runtime_capability;
    mod runtime_lifecycle_tests;
    mod sandbox_mounts;

    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use ironclaw_host_api::{
        action::NetworkPolicy,
        capability::{CapabilityDescriptor, CapabilityGrant, GrantConstraints, PermissionMode},
        ids::{AgentId, CapabilityGrantId, ProjectId, TenantId, UserId},
        mount::{MountGrant, MountPermissions},
        path::{MountAlias, VirtualPath},
        resolution::{Blocked, Suspension, ToolVerdict},
        resource::{ResourceEstimate, ResourceUsage},
        result_meta::{FailureKind, ModelFailureDiagnostic},
        runtime::{RuntimeKind, TrustClass},
        safe_summary::SafeSummary,
    };
    use ironclaw_host_runtime::{
        CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion,
        HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalResume, RuntimeCapabilityCompleted,
        RuntimeCapabilityFailure, RuntimeInvocation, RuntimeStatusRequest, SurfaceKind,
        VisibleCapability, VisibleCapabilityAccess, VisibleCapabilitySurface,
    };
    use ironclaw_loop_contracts::{
        InMemoryRunProfileResolver, LoopDriverId, RunProfileResolutionRequest, RunProfileResolver,
    };
    use ironclaw_sandbox::{SandboxProcessPlan, ValidatedSandboxProcessPlan};
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use ironclaw_turns::{TurnActor, TurnId, TurnRunId, TurnScope};

    use crate::{capability_info, capability_surface_filter::CapabilitySurfaceVisibleFilter};

    #[tokio::test]
    async fn decorating_factory_with_no_decorators_delegates_to_inner() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(DecoratorTestPort {
            label: "inner",
            log: Arc::clone(&log),
        });
        let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(DecoratorTestFactory {
            port: inner,
        }));

        let port = factory
            .create_capability_port(&loop_run_context(&execution_context("decorator-empty")).await)
            .await
            .expect("decorated port");

        let error = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect_err("test inner port should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(&*log.lock().expect("log lock"), &["inner"]);
    }

    fn provider_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: ProviderToolName::new("demo__echo").expect("provider tool name"),
            arguments: serde_json::json!({"message":"hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    struct FallbackInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for FallbackInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "fallback input resolver should not be used",
            ))
        }
    }

    /// Inner resolver that records every
    /// `record_provider_tool_call_display_input` call, so a test can assert the
    /// `ProviderToolCallInputResolver` decorator forwards the display hook with
    /// the resolved capability id.
    #[derive(Default)]
    struct DisplayInputRecordingResolver {
        recorded: Mutex<Vec<(String, String, serde_json::Value)>>,
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for DisplayInputRecordingResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "inner resolver should not resolve in this test",
            ))
        }

        fn record_provider_tool_call_display_input(
            &self,
            _run_context: &LoopRunContext,
            input_ref: &CapabilityInputRef,
            capability_id: &CapabilityId,
            tool_call: &ProviderToolCall,
        ) {
            self.recorded.lock().expect("recorded lock").push((
                input_ref.as_str().to_string(),
                capability_id.as_str().to_string(),
                tool_call.arguments.clone(),
            ));
        }
    }

    /// Captures every input callback the port forwards, so tests can drive the
    /// real `invoke_capability` call site and assert the observer fired.
    #[derive(Debug, Default)]
    struct RecordingTrajectoryObserver {
        inputs: Mutex<Vec<(String, String, serde_json::Value)>>,
    }

    impl CapabilityTrajectoryObserver for RecordingTrajectoryObserver {
        fn on_capability_input(
            &self,
            call_id: &str,
            capability_id: &str,
            arguments: &serde_json::Value,
        ) {
            self.inputs.lock().expect("inputs lock").push((
                call_id.to_string(),
                capability_id.to_string(),
                arguments.clone(),
            ));
        }
    }

    fn visible_request(
        context: ExecutionContext,
    ) -> ironclaw_host_runtime::VisibleCapabilityRequest {
        ironclaw_host_runtime::VisibleCapabilityRequest::new(
            context,
            SurfaceKind::new("test").expect("valid surface kind"),
        )
    }

    struct DecoratorTestFactory {
        port: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for DecoratorTestFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Ok(Arc::clone(&self.port))
        }
    }

    struct FailingDecoratorFactory {
        error: AgentLoopHostError,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for FailingDecoratorFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Err(self.error.clone())
        }
    }

    struct DecoratorTestPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for DecoratorTestPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<ironclaw_loop_contracts::VisibleCapabilitySurface, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} failed", label = self.label),
            ))
        }

        async fn invoke_capability(
            &self,
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }
    }

    struct LoggingDecorator {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl LoopCapabilityPortDecorator for LoggingDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            Arc::new(LoggingDecoratorPort {
                label: self.label,
                log: Arc::clone(&self.log),
                inner,
            })
        }
    }

    struct LoggingDecoratorPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        inner: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPort for LoggingDecoratorPort {
        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<ironclaw_loop_contracts::VisibleCapabilitySurface, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.visible_capabilities(request).await
        }

        async fn invoke_capability(
            &self,
            request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.invoke_capability_batch(request).await
        }
    }

    struct NoopDecorator {
        decorate_calls: Arc<AtomicUsize>,
    }

    impl LoopCapabilityPortDecorator for NoopDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            self.decorate_calls.fetch_add(1, Ordering::SeqCst);
            inner
        }
    }

    fn execution_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/execution").expect("valid mount alias"),
            VirtualPath::new("/projects/execution").expect("valid virtual path"),
            MountPermissions::read_only(),
        )])
        .expect("valid mount view")
    }

    fn mount_view(alias: &str, target: &str) -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new(alias).expect("valid mount alias"),
            VirtualPath::new(target).expect("valid virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("valid mount view")
    }

    fn dispatch_capability_grant(
        capability_id: &CapabilityId,
        grantee: &ExtensionId,
    ) -> CapabilityGrant {
        capability_grant_with_effects(capability_id, grantee, vec![EffectKind::DispatchCapability])
    }

    fn capability_grant_with_effects(
        capability_id: &CapabilityId,
        grantee: &ExtensionId,
        allowed_effects: Vec<EffectKind>,
    ) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects,
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn dispatch_trust_decision() -> TrustDecision {
        trust_decision_with_effects(vec![EffectKind::DispatchCapability])
    }

    fn trust_decision_with_effects(allowed_effects: Vec<EffectKind>) -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    fn visible_capability(id: CapabilityId, provider: ExtensionId) -> VisibleCapability {
        visible_capability_with_runtime_effects(
            id,
            provider,
            RuntimeKind::FirstParty,
            vec![EffectKind::DispatchCapability],
        )
    }

    fn visible_capability_with_runtime_effects(
        id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        effects: Vec<EffectKind>,
    ) -> VisibleCapability {
        VisibleCapability {
            descriptor: CapabilityDescriptor {
                id,
                provider,
                runtime,
                trust_ceiling: TrustClass::UserTrusted,
                description: "demo capability".to_string(),
                parameters_schema: serde_json::json!({"type":"object"}),
                effects,
                default_permission: PermissionMode::Allow,
                runtime_credentials: Vec::new(),
                network_targets: Vec::new(),
                max_egress_bytes: None,
                resource_profile: None,
                origin_gate_matrix: None,
                standard_op: None,
            },
            description_trust: Default::default(),
            access: VisibleCapabilityAccess::Available,
            estimated_resources: ResourceEstimate::default(),
        }
    }

    fn dummy_runtime() -> Arc<dyn HostRuntime> {
        Arc::new(NoopHostRuntime)
    }

    fn dummy_input_resolver() -> Arc<dyn LoopCapabilityInputResolver> {
        Arc::new(NoopCapabilityIo)
    }

    fn dummy_result_writer() -> Arc<dyn LoopCapabilityResultWriter> {
        Arc::new(NoopCapabilityIo)
    }

    fn dummy_milestone_sink() -> Arc<dyn LoopHostMilestoneSink> {
        Arc::new(ironclaw_loop_contracts::InMemoryLoopHostMilestoneSink::default())
    }

    /// Deterministic in-memory [`GateRecordStorePort`] fake for seam tests: records
    /// every write and answers `load` by the exact `(scope, gate_ref)` a gate
    /// outcome was persisted under. Keyed by `GateRef` (a freshly-minted uuid,
    /// globally unique) with the scope carried in the value for the wrong-scope
    /// isolation check the durable store applies. The durable
    /// `GateRecordStore` round-trip itself is covered by
    /// `ironclaw_approvals`'s `gate_record_store_contract`; this fake pins that
    /// the loop_host seam calls `save` with the right record and gate ref.
    #[derive(Debug, Default)]
    struct RecordingGateRecordStore {
        saves: Mutex<Vec<(ResourceScope, GateRef, GateRecord)>>,
    }

    impl RecordingGateRecordStore {
        fn saved(&self) -> Vec<(ResourceScope, GateRef, GateRecord)> {
            self.saves.lock().expect("gate record saves lock").clone()
        }
    }

    #[async_trait]
    impl GateRecordStorePort for RecordingGateRecordStore {
        async fn save(
            &self,
            scope: ResourceScope,
            gate_ref: GateRef,
            record: GateRecord,
        ) -> Result<(), ApprovalStoreError> {
            self.saves
                .lock()
                .expect("gate record saves lock")
                .push((scope, gate_ref, record));
            Ok(())
        }

        async fn load(
            &self,
            scope: &ResourceScope,
            gate_ref: GateRef,
        ) -> Result<Option<GateRecord>, ApprovalStoreError> {
            Ok(self
                .saves
                .lock()
                .expect("gate record saves lock")
                .iter()
                .find(|(saved_scope, saved_ref, _)| saved_scope == scope && *saved_ref == gate_ref)
                .map(|(_, _, record)| record.clone()))
        }
    }

    /// Fails the first `save` with a backend fault, then delegates to an inner
    /// [`RecordingGateRecordStore`] — for the transient-fault retry test.
    #[derive(Debug, Default)]
    struct FailOnceGateRecordStore {
        failed_once: Mutex<bool>,
        inner: RecordingGateRecordStore,
    }

    #[async_trait]
    impl GateRecordStorePort for FailOnceGateRecordStore {
        async fn save(
            &self,
            scope: ResourceScope,
            gate_ref: GateRef,
            record: GateRecord,
        ) -> Result<(), ApprovalStoreError> {
            let fail_now = {
                let mut failed_once = self.failed_once.lock().expect("fail-once lock");
                let first = !*failed_once;
                *failed_once = true;
                first
            };
            if fail_now {
                return Err(ApprovalStoreError::Backend(
                    "injected transient store fault".to_string(),
                ));
            }
            self.inner.save(scope, gate_ref, record).await
        }

        async fn load(
            &self,
            scope: &ResourceScope,
            gate_ref: GateRef,
        ) -> Result<Option<GateRecord>, ApprovalStoreError> {
            self.inner.load(scope, gate_ref).await
        }
    }

    /// Blocks the FIRST `save` until released (announcing entry via a permit) so
    /// a test can cancel the persist future while it is parked in `save`; later
    /// saves delegate straight through. The cancellation test drops the future
    /// instead of releasing, so `release` is never fired. For the
    /// reservation-cleanup regression.
    struct BlockingGateRecordStore {
        inner: RecordingGateRecordStore,
        entered: tokio::sync::Semaphore,
        release: Notify,
        blocked: std::sync::atomic::AtomicBool,
    }

    impl BlockingGateRecordStore {
        fn new() -> Self {
            Self {
                inner: RecordingGateRecordStore::default(),
                entered: tokio::sync::Semaphore::new(0),
                release: Notify::new(),
                blocked: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn saved(&self) -> Vec<(ResourceScope, GateRef, GateRecord)> {
            self.inner.saved()
        }
    }

    #[async_trait]
    impl GateRecordStorePort for BlockingGateRecordStore {
        async fn save(
            &self,
            scope: ResourceScope,
            gate_ref: GateRef,
            record: GateRecord,
        ) -> Result<(), ApprovalStoreError> {
            if !self.blocked.swap(true, std::sync::atomic::Ordering::SeqCst) {
                // First save: announce entry (reservation is now InFlight) and
                // block until released — the cancellation test drops this future
                // here instead of releasing.
                self.entered.add_permits(1);
                self.release.notified().await;
            }
            self.inner.save(scope, gate_ref, record).await
        }

        async fn load(
            &self,
            scope: &ResourceScope,
            gate_ref: GateRef,
        ) -> Result<Option<GateRecord>, ApprovalStoreError> {
            self.inner.load(scope, gate_ref).await
        }
    }

    /// Deterministic in-memory [`ReplayPayloadStorePort`] fake for seam tests: the
    /// port `save`s the raw replay payload at a fresh gate raise and `load`s it on
    /// resume. Keyed by `InvocationId` (globally unique per invocation); the scope
    /// is recorded for assertions but `load` is scope-insensitive because these
    /// crate-tier tests pin the write/read WIRING, not scope isolation — the
    /// durable `ReplayPayloadStore`'s wrong-scope-looks-unknown check is
    /// covered by `ironclaw_capabilities`' own contract test and the full-infra
    /// cross-tenant integration scenario.
    #[derive(Debug, Default)]
    struct RecordingReplayPayloadStore {
        saves: Mutex<std::collections::HashMap<InvocationId, (ResourceScope, ReplayPayload)>>,
    }

    impl RecordingReplayPayloadStore {
        fn get(&self, invocation_id: InvocationId) -> Option<ReplayPayload> {
            self.saves
                .lock()
                .expect("replay payload saves lock")
                .get(&invocation_id)
                .map(|(_, payload)| payload.clone())
        }

        /// Pre-seed a payload as if a prior fresh gate raise had persisted it, for
        /// tests that inject a resume without a preceding raise.
        fn seed(&self, scope: ResourceScope, invocation_id: InvocationId, payload: ReplayPayload) {
            self.saves
                .lock()
                .expect("replay payload saves lock")
                .insert(invocation_id, (scope, payload));
        }
    }

    #[async_trait]
    impl ReplayPayloadStorePort for RecordingReplayPayloadStore {
        async fn save(
            &self,
            scope: ResourceScope,
            invocation_id: InvocationId,
            payload: ReplayPayload,
        ) -> Result<(), ReplayPayloadStoreError> {
            use std::collections::hash_map::Entry;
            match self
                .saves
                .lock()
                .expect("replay payload saves lock")
                .entry(invocation_id)
            {
                Entry::Occupied(_) => {
                    Err(ReplayPayloadStoreError::ReplayPayloadAlreadyExists { invocation_id })
                }
                Entry::Vacant(slot) => {
                    slot.insert((scope, payload));
                    Ok(())
                }
            }
        }

        async fn load(
            &self,
            _scope: &ResourceScope,
            invocation_id: InvocationId,
        ) -> Result<Option<ReplayPayload>, ReplayPayloadStoreError> {
            Ok(self.get(invocation_id))
        }
    }

    const RECORDING_OUTPUT_BYTES: u64 = 12;

    async fn runtime_capability_port(
        capability_id: &CapabilityId,
        provider_id: &ExtensionId,
        runtime: Arc<dyn HostRuntime>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        thread_id: &str,
    ) -> HostRuntimeLoopCapabilityPort {
        let mut context = execution_context(thread_id);
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            capability_id,
            &loop_driver_extension,
        ));
        HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer,
            milestone_sink,
        )
        .port_for_run_context(run_context)
    }

    /// Like [`runtime_capability_port`] but wires an explicit
    /// [`GateRecordStorePort`], so seam tests can observe the durable gate record the
    /// port persists at the capability seam.
    async fn runtime_capability_port_with_gate_store(
        capability_id: &CapabilityId,
        provider_id: &ExtensionId,
        runtime: Arc<dyn HostRuntime>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        gate_record_store: Arc<dyn GateRecordStorePort>,
        thread_id: &str,
    ) -> HostRuntimeLoopCapabilityPort {
        let mut context = execution_context(thread_id);
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            capability_id,
            &loop_driver_extension,
        ));
        HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer,
            milestone_sink,
        )
        .with_gate_record_store(gate_record_store)
        .port_for_run_context(run_context)
    }

    /// Like [`runtime_capability_port`] but wires an explicit
    /// [`ReplayPayloadStorePort`], so resume seam tests can round-trip the raw replay
    /// payload the host persists at a gate raise and reconstitutes on resume.
    async fn runtime_capability_port_with_replay_store(
        capability_id: &CapabilityId,
        provider_id: &ExtensionId,
        runtime: Arc<dyn HostRuntime>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        replay_payload_store: Arc<dyn ReplayPayloadStorePort>,
        thread_id: &str,
    ) -> HostRuntimeLoopCapabilityPort {
        let mut context = execution_context(thread_id);
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            capability_id,
            &loop_driver_extension,
        ));
        HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer,
            milestone_sink,
        )
        .with_replay_payload_store(replay_payload_store)
        .port_for_run_context(run_context)
    }

    async fn visible_runtime_invocation(port: &HostRuntimeLoopCapabilityPort) -> LoopRequest {
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
            .await
            .expect("provider tool call registers");
        LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        }
    }

    fn resume_token_for_different_activity(
        activity_id: CapabilityActivityId,
    ) -> CapabilityResumeToken {
        loop {
            let invocation_id = InvocationId::new();
            if invocation_id.as_uuid() != activity_id.as_uuid() {
                return CapabilityResumeToken::new(invocation_id.to_string())
                    .expect("valid resume token");
            }
        }
    }

    async fn invoke_visible_runtime_capability(
        port: &HostRuntimeLoopCapabilityPort,
    ) -> Result<Resolution, AgentLoopHostError> {
        port.invoke_capability(visible_runtime_invocation(port).await)
            .await
    }

    struct RecordingHostRuntime {
        capabilities: Mutex<Vec<VisibleCapability>>,
        requests: Mutex<Vec<RuntimeInvocation>>,
        spawn_requests: Mutex<Vec<RuntimeInvocation>>,
        spawn_attempts: AtomicUsize,
        spawn_failure: Mutex<Option<RuntimeCapabilityFailure>>,
    }

    impl RecordingHostRuntime {
        fn new(capabilities: Vec<VisibleCapability>) -> Self {
            Self {
                capabilities: Mutex::new(capabilities),
                requests: Mutex::new(Vec::new()),
                spawn_requests: Mutex::new(Vec::new()),
                spawn_attempts: AtomicUsize::new(0),
                spawn_failure: Mutex::new(None),
            }
        }

        fn with_spawn_failure(self, failure: RuntimeCapabilityFailure) -> Self {
            *self.spawn_failure.lock().expect("spawn failure lock") = Some(failure);
            self
        }

        fn set_capabilities(&self, capabilities: Vec<VisibleCapability>) {
            *self.capabilities.lock().expect("capabilities lock") = capabilities;
        }

        fn take_requests(&self) -> Vec<RuntimeInvocation> {
            self.requests.lock().expect("requests lock").clone()
        }

        fn take_spawn_requests(&self) -> Vec<RuntimeInvocation> {
            self.spawn_requests
                .lock()
                .expect("spawn requests lock")
                .clone()
        }

        fn spawn_attempts(&self) -> usize {
            self.spawn_attempts.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl HostRuntime for RecordingHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.requests
                .lock()
                .expect("requests lock")
                .push(request.clone());
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.1,
                    output: serde_json::json!({"ok": true}),
                    display_preview: None,
                    usage: ResourceUsage::default().set_output_bytes(RECORDING_OUTPUT_BYTES),
                },
            )))
        }

        async fn spawn_capability(
            &self,
            mut request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.spawn_attempts.fetch_add(1, Ordering::Relaxed);
            if let Some(failure) = self
                .spawn_failure
                .lock()
                .expect("spawn failure lock")
                .clone()
            {
                return Ok(RuntimeCapabilityOutcome::Failed(failure));
            }
            if is_process_sandbox_capability(&request.1) {
                let plan = match serde_json::from_value::<SandboxProcessPlan>(request.3.clone()) {
                    Ok(plan) => plan,
                    Err(error) => {
                        return Ok(RuntimeCapabilityOutcome::Failed(
                            RuntimeCapabilityFailure::new(
                                request.1,
                                FailureKind::InputEncode,
                                Some(
                                    "process sandbox capability input must be a SandboxProcessPlan"
                                        .to_string(),
                                ),
                            )
                            .with_model_visible_cause(error.to_string()),
                        ));
                    }
                };
                let plan = match ValidatedSandboxProcessPlan::new(plan) {
                    Ok(plan) => plan,
                    Err(error) => {
                        return Ok(RuntimeCapabilityOutcome::Failed(
                            RuntimeCapabilityFailure::new(
                                request.1,
                                FailureKind::InputEncode,
                                Some(
                                    "process sandbox capability input failed SandboxProcessPlan validation"
                                        .to_string(),
                                ),
                            )
                            .with_model_visible_cause(error.to_string()),
                        ));
                    }
                };
                request.3 = serde_json::to_value(plan.into_plan())
                    .expect("validated sandbox plan must serialize in test runtime");
            }
            self.spawn_requests
                .lock()
                .expect("spawn requests lock")
                .push(request.clone());
            Ok(RuntimeCapabilityOutcome::SpawnedProcess(
                ironclaw_host_runtime::RuntimeProcessHandle {
                    process_id: ironclaw_host_api::ids::ProcessId::new(),
                    capability_id: request.1,
                },
            ))
        }

        async fn resume_capability(
            &self,
            _request: RuntimeApprovalResume,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("recording host runtime should not resume")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
                capabilities: self.capabilities.lock().expect("capabilities lock").clone(),
            })
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("recording host runtime should not cancel work")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("recording host runtime should not report status")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("recording host runtime should not report health")
        }
    }

    struct RecordingResumeHostRuntime {
        capabilities: Vec<VisibleCapability>,
        resume_requests: Mutex<Vec<RuntimeApprovalResume>>,
    }

    impl RecordingResumeHostRuntime {
        fn new(capabilities: Vec<VisibleCapability>) -> Self {
            Self {
                capabilities,
                resume_requests: Mutex::new(Vec::new()),
            }
        }

        fn resume_request_count(&self) -> usize {
            self.resume_requests
                .lock()
                .expect("resume requests lock")
                .len()
        }

        fn resume_requests(&self) -> Vec<RuntimeApprovalResume> {
            self.resume_requests
                .lock()
                .expect("resume requests lock")
                .clone()
        }
    }

    #[async_trait]
    impl HostRuntime for RecordingResumeHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("recording resume runtime should not fresh-dispatch")
        }

        async fn resume_capability(
            &self,
            request: RuntimeApprovalResume,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.resume_requests
                .lock()
                .expect("resume requests lock")
                .push(request.clone());
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.2,
                    output: serde_json::json!({"resumed": true}),
                    display_preview: None,
                    usage: ResourceUsage::default().set_output_bytes(RECORDING_OUTPUT_BYTES),
                },
            )))
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
                capabilities: self.capabilities.clone(),
            })
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("recording resume runtime should not cancel work")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("recording resume runtime should not report status")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("recording resume runtime should not report health")
        }
    }

    struct QueuedHostRuntime {
        capabilities: Vec<VisibleCapability>,
        outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    }

    impl QueuedHostRuntime {
        fn new(
            capabilities: Vec<VisibleCapability>,
            outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
        ) -> Self {
            Self {
                capabilities,
                outcomes: Mutex::new(VecDeque::from(outcomes)),
            }
        }
    }

    #[async_trait]
    impl HostRuntime for QueuedHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.outcomes
                .lock()
                .expect("outcomes lock")
                .pop_front()
                .expect("queued host runtime outcome")
        }

        async fn resume_capability(
            &self,
            _request: RuntimeApprovalResume,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("queued host runtime should not resume")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
                capabilities: self.capabilities.clone(),
            })
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("queued host runtime should not cancel work")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("queued host runtime should not report status")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("queued host runtime should not report health")
        }
    }

    #[derive(Default)]
    struct FailOnceTerminalMilestoneSink {
        failures: AtomicUsize,
        milestones: Mutex<Vec<ironclaw_loop_contracts::LoopHostMilestone>>,
    }

    impl FailOnceTerminalMilestoneSink {
        fn milestones(&self) -> Vec<ironclaw_loop_contracts::LoopHostMilestone> {
            self.milestones.lock().expect("milestones lock").clone()
        }
    }

    #[async_trait]
    impl LoopHostMilestoneSink for FailOnceTerminalMilestoneSink {
        async fn publish_loop_milestone(
            &self,
            milestone: ironclaw_loop_contracts::LoopHostMilestone,
        ) -> Result<(), AgentLoopHostError> {
            let is_terminal = matches!(
                &milestone.kind,
                ironclaw_loop_contracts::LoopHostMilestoneKind::CapabilityCompleted { .. }
                    | ironclaw_loop_contracts::LoopHostMilestoneKind::CapabilityFailed { .. }
            );
            if is_terminal && self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "terminal milestone sink unavailable",
                ));
            }
            self.milestones
                .lock()
                .expect("milestones lock")
                .push(milestone);
            Ok(())
        }
    }

    struct StaticInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for StaticInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({"ok": true}))
        }
    }

    struct JsonInputResolver(serde_json::Value);

    #[async_trait]
    impl LoopCapabilityInputResolver for JsonInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(self.0.clone())
        }
    }

    struct ProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for ProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "run": {
                    "command": "echo",
                    "args": ["ok"]
                }
            }))
        }
    }

    struct InvalidProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for InvalidProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "run": {
                    "command": ""
                }
            }))
        }
    }

    struct MalformedProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for MalformedProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "not_run": true
            }))
        }
    }

    struct StaticResultWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for StaticResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            let result_ref = LoopResultRef::new("result:mount-test").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok(CapabilityWriteResult::without_output_digest(result_ref, 0))
        }
    }

    #[derive(Default)]
    struct FailOnceResultWriter {
        attempts: AtomicUsize,
    }

    impl FailOnceResultWriter {
        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for FailOnceResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::TranscriptWriteFailed,
                    "transient result write failure",
                ));
            }
            let result_ref = LoopResultRef::new("result:capability-info-retry").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok(CapabilityWriteResult::without_output_digest(result_ref, 0))
        }
    }

    #[derive(Default)]
    struct RecordingResultWriter {
        records: Mutex<Vec<(CapabilityId, serde_json::Value)>>,
        display_previews: Mutex<Vec<Option<CapabilityDisplayOutputPreview>>>,
        failure_previews: Mutex<Vec<(InvocationId, CapabilityId, String)>>,
    }

    impl RecordingResultWriter {
        fn records(&self) -> Vec<(CapabilityId, serde_json::Value)> {
            self.records.lock().expect("records lock").clone()
        }

        fn display_previews(&self) -> Vec<Option<CapabilityDisplayOutputPreview>> {
            self.display_previews
                .lock()
                .expect("display previews lock")
                .clone()
        }

        fn failure_previews(&self) -> Vec<(InvocationId, CapabilityId, String)> {
            self.failure_previews
                .lock()
                .expect("failure previews lock")
                .clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for RecordingResultWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            let output_digest = ContentDigest::from_json_value(&write.output).map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("capability result output digest could not be built: {error}"),
                )
            })?;
            self.records
                .lock()
                .expect("records lock")
                .push((write.capability_id.clone(), write.output));
            self.display_previews
                .lock()
                .expect("display previews lock")
                .push(write.display_preview);
            let result_ref = LoopResultRef::new("result:capability-info").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok(CapabilityWriteResult {
                result_ref,
                byte_len: 0,
                output_digest: Some(output_digest),
                model_observation: None,
            })
        }

        async fn stage_capability_failure_preview(
            &self,
            _run_context: &LoopRunContext,
            invocation_id: InvocationId,
            capability_id: &CapabilityId,
            summary: &str,
        ) {
            self.failure_previews
                .lock()
                .expect("failure previews lock")
                .push((invocation_id, capability_id.clone(), summary.to_string()));
        }
    }

    struct NoopHostRuntime;

    #[async_trait]
    impl HostRuntime for NoopHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn resume_capability(
            &self,
            _request: RuntimeApprovalResume,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }
    }

    struct NoopCapabilityIo;

    #[async_trait]
    impl LoopCapabilityInputResolver for NoopCapabilityIo {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            unreachable!("noop capability io should not be called")
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for NoopCapabilityIo {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            unreachable!("noop capability io should not be called")
        }
    }

    fn execution_context(thread: &str) -> ExecutionContext {
        let thread_id = ironclaw_host_api::ids::ThreadId::new(thread).expect("valid thread id");
        let mut context = ExecutionContext::local_default(
            UserId::new("user-capability-port").expect("valid user"),
            ExtensionId::new("loop-driver").expect("valid extension"),
            RuntimeKind::FirstParty,
            TrustClass::System,
            CapabilitySet::default(),
            MountView::default(),
        )
        .expect("valid context");
        context.tenant_id = TenantId::new("tenant-capability-port").expect("valid tenant");
        context.agent_id = Some(AgentId::new("agent-capability-port").expect("valid agent"));
        context.project_id =
            Some(ProjectId::new("project-capability-port").expect("valid project"));
        context.thread_id = Some(thread_id.clone());
        context.resource_scope.tenant_id = context.tenant_id.clone();
        context.resource_scope.agent_id = context.agent_id.clone();
        context.resource_scope.project_id = context.project_id.clone();
        context.resource_scope.thread_id = Some(thread_id);
        context
    }

    async fn loop_run_context(context: &ExecutionContext) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                context.tenant_id.clone(),
                context.agent_id.clone(),
                context.project_id.clone(),
                context.thread_id.clone().expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }
}
