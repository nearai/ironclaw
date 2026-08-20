//! Host runtime service for IronClaw Reborn.
//!
//! `ironclaw_host_runtime` is the narrow boundary upper Reborn services build
//! against. It surfaces both:
//!
//! - the [`HostRuntime`] trait — the stable contract upper turn/loop services
//!   depend on;
//! - [`DefaultHostRuntime`] — the production composition that wraps
//!   [`ironclaw_capabilities::CapabilityHost`] (which itself coordinates
//!   authorization, approvals, process invocation lifecycle, and process spawn) behind
//!   that contract.
//!
//! The service preserves three important boundaries:
//!
//! - callers see structured capability outcomes instead of lower substrate
//!   handles;
//! - approval/auth/resource waits are suspension states, not errors;
//! - caller/workflow origin taxonomy is intentionally kept outside this lower
//!   service. Authority remains in [`ExecutionContext`] (principals, grants,
//!   leases, policy); projection selection is an opaque [`SurfaceKind`] label
//!   the host treats as a cache/version dimension only. Caller-authority
//!   filtering of which surface a particular UI or upper service is allowed to
//!   render is intentionally an upper-layer concern — the host does not bake
//!   in upper-stack vocabulary (e.g. agent loop / adapter / admin).
#![warn(unreachable_pub)]

use async_trait::async_trait;
use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    dispatch::{CapabilityDisplayOutputPreview, DispatchFailureDetail, ProviderDiagnostic},
    ids::{ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId, ProcessId, SecretHandle},
    resource::{ResourceEstimate, ResourceScope, ResourceUsage},
    result_meta::{FailureFate, FailureKind},
    runtime::RuntimeKind,
    runtime_policy::{DeploymentMode, EffectiveRuntimePolicy, RuntimeProfile},
    scope::ExecutionContext,
};
use ironclaw_trust::TrustDecision;
use serde_json::Value;
use std::{collections::BTreeMap, env, fmt};
use thiserror::Error;

mod capability_catalog;
mod capability_response_processor;
mod document_output;
mod egress;
mod extension_contracts;
mod first_party;
mod first_party_tools;
mod http_body;
mod invocation_services;
mod latency;
pub mod memory_binding;
pub mod memory_context;
pub mod memory_native_extension;
pub mod memory_provider;
mod obligations;
mod post_edit_check;
mod process_aliases;
mod process_output;
mod process_port;
mod production;
mod services;
mod standard_op_output;
mod surface;
mod user_profile_source;
mod wasm_credentials;

pub use user_profile_source::MemoryBackedUserProfileSource;

pub use memory_native_extension::native_memory_first_party_package;

pub use capability_catalog::{
    HotCapabilityCatalog, HotCapabilityRecord, MAX_HOT_PROMPT_BYTES, MAX_HOT_SCHEMA_BYTES,
    publish_hot_capability_catalog,
};
pub use egress::{
    HostHttpEgressService, HostRuntimeCredentialMaterial, HostRuntimeHttpEgressPort,
    HostRuntimeHttpEgressRequest, RuntimeSecretMaterialStager, RuntimeSecretStageError,
};
pub use extension_contracts::{
    discover_extensions_tolerant_bounded, discover_extensions_tolerant_bounded_with_contracts,
    discover_extensions_with_default_host_api_contracts,
    discover_extensions_with_default_host_api_contracts_and_catalog,
};
pub use first_party::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
pub use first_party_tools::{
    APPLY_PATCH_CAPABILITY_ID, ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID,
    BUILTIN_FIRST_PARTY_PROVIDER, BuiltinFirstPartyTools, DOCUMENT_EDIT_CAPABILITY_ID,
    ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTML_TO_PDF_CAPABILITY_ID,
    HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID,
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID, MemoryToolProfile, NATIVE_MEMORY_FIRST_PARTY_PROVIDER,
    NativeMemoryToolHandler, OUTBOUND_DELIVER_CAPABILITY_ID, PROFILE_SET_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID, SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_UPDATE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID, TIME_CAPABILITY_ID,
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, TRACE_COMMONS_CREDITS_CAPABILITY_ID,
    TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID, TriggerCreateHook,
    WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers,
    builtin_first_party_handlers_for_process_backend,
    builtin_first_party_handlers_with_trigger_create_hook,
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend,
    builtin_first_party_package, builtin_first_party_package_for_process_backend,
    ensure_memory_mount, finish_memory_tool_result, map_memory_service_error,
    memory_invocation_for_request, memory_tool_profiles, normalize_memory_tool_input,
    register_memory_tool_handler, register_native_memory_tools,
    register_outbound_deliver_first_party_handler, register_reply_attachment_first_party_handler,
};
#[cfg(any(test, feature = "test-support"))]
pub use first_party_tools::{
    TriggerManagementClock, builtin_first_party_handlers_with_trigger_clock,
};
pub use http_body::{RuntimeHttpBodyStore, RuntimeHttpBodyStoreError};
pub use invocation_services::{
    ConfiguredInvocationServicesResolver, InvocationServices, InvocationServicesError,
    InvocationServicesResolutionRequest, InvocationServicesResolver, ToolCallHttpEgress,
};
pub use obligations::{
    BuiltinObligationHandler, BuiltinObligationServices, LEAK_REDACT_FAILED_CODE,
    ProcessObligationLifecycleStore, RuntimeCredentialAccessSecret,
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver,
};
pub use post_edit_check::{
    POST_EDIT_CHECK_ENV, POST_EDIT_CHECK_TIMEOUT_ENV, PostEditCheckConfig,
    PostEditCheckConfigError, PostEditCheckService,
};
pub use process_port::{HostProcessPort, RuntimeProcessPort, UserSandboxProcessPort};
pub use production::DefaultHostRuntime;
// The sandbox lane (`sandbox_process`) moved to `ironclaw_sandbox` with the
// WS3 merge; its Docker/CA cone is a runtimes-layer concern, and nothing
// outside this crate ever consumed these re-exports.
/// Scoped cleanup guard consumed by the generic extension activation
/// transaction's composition adapter. Raw obligation handoff stores remain
/// private; `reborn_host_runtime_services_do_not_expose_lower_substrate_handles`
/// enforces that direct path stays closed.
pub use services::ProductAuthRuntimeHandoffGuard;
pub use services::{
    ExtensionLaneToolBinder, ExtensionToolBindError, HostRuntimeServices,
    ProductAuthCredentialStageError, ProductAuthProviderRuntimePorts,
    ProductionEventStoreWiringError, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssue, ProductionWiringIssueKind, ProductionWiringReport,
    RegisteredRuntimeHealth,
};
pub use surface::{VisibleCapability, VisibleCapabilityAccess};
/// Stable, validated idempotency key supplied by upper turn/loop services.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, HostRuntimeError> {
        validate_bounded_contract_string(value.into(), "idempotency key", 256).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for IdempotencyKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<IdempotencyKey> for String {
    fn from(value: IdempotencyKey) -> Self {
        value.into_string()
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_bounded_contract_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, HostRuntimeError> {
    if value.is_empty() {
        return Err(HostRuntimeError::invalid_request(format!(
            "{label} must not be empty"
        )));
    }
    if value.len() > max_bytes {
        return Err(HostRuntimeError::invalid_request(format!(
            "{label} must be at most {max_bytes} bytes"
        )));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(HostRuntimeError::invalid_request(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    Ok(value)
}

/// Host-runtime-local gate id for non-approval suspension states.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeGateId(String);

impl RuntimeGateId {
    pub fn new() -> Self {
        Self(CorrelationId::new().to_string())
    }

    pub fn from_stable_suffix(suffix: &str) -> Result<Self, HostRuntimeError> {
        Ok(Self(validate_bounded_contract_string(
            suffix.to_string(),
            "runtime gate id",
            128,
        )?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RuntimeGateId {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<str> for RuntimeGateId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<RuntimeGateId> for String {
    fn from(value: RuntimeGateId) -> Self {
        value.0
    }
}

impl fmt::Display for RuntimeGateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Version token for the host-filtered visible capability surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilitySurfaceVersion(String);

impl CapabilitySurfaceVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, HostRuntimeError> {
        validate_bounded_contract_string(value.into(), "capability surface version", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CapabilitySurfaceVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<CapabilitySurfaceVersion> for String {
    fn from(value: CapabilitySurfaceVersion) -> Self {
        value.0
    }
}

impl fmt::Display for CapabilitySurfaceVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque projection-surface label supplied by the caller.
///
/// The host treats this as a cache/version dimension only — it must not bake
/// in upper-stack vocabulary (agent loop, adapter, admin, …) and must not
/// derive authority or filtering decisions from the label. Upper layers are
/// responsible for deciding which surface label a given caller is allowed to
/// render; this lower service simply returns the projection associated with
/// whatever label is presented.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SurfaceKind(String);

impl SurfaceKind {
    pub fn new(value: impl Into<String>) -> Result<Self, HostRuntimeError> {
        validate_bounded_contract_string(value.into(), "surface kind", 64).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for SurfaceKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<SurfaceKind> for String {
    fn from(value: SurfaceKind) -> Self {
        value.into_string()
    }
}

impl fmt::Display for SurfaceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Request to list host-filtered visible capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VisibleCapabilityRequest {
    /// Authority envelope used for the same grant/trust checks as invocation.
    pub context: ExecutionContext,
    /// Projection surface selection only; this is not authority and must not
    /// grant or bypass authorization. The host treats this as an opaque
    /// cache/version dimension; deciding which surface labels a given caller
    /// may request is an upper-layer concern.
    pub surface_kind: SurfaceKind,
    /// Caller/host-supplied trust decisions keyed by capability provider.
    ///
    /// `DefaultHostRuntime` does not evaluate trust while computing visibility;
    /// missing provider trust fails closed by omitting that provider's
    /// capabilities from the surface.
    pub provider_trust: BTreeMap<ExtensionId, TrustDecision>,
    /// Upper/profile-supplied visibility ceiling. This only narrows what is
    /// shown; it never grants authority or bypasses invocation authorization.
    pub policy: CapabilitySurfacePolicy,
}

impl VisibleCapabilityRequest {
    pub fn new(context: ExecutionContext, surface_kind: SurfaceKind) -> Self {
        Self {
            context,
            surface_kind,
            provider_trust: BTreeMap::new(),
            policy: CapabilitySurfacePolicy::default(),
        }
    }

    pub fn with_provider_trust(
        mut self,
        provider_trust: BTreeMap<ExtensionId, TrustDecision>,
    ) -> Self {
        self.provider_trust = provider_trust;
        self
    }

    pub fn with_policy(mut self, policy: CapabilitySurfacePolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Host-filtered visible capability surface.
///
/// Entries are returned in filtered registry order for deterministic rendering.
/// The version fingerprint canonicalizes unordered inputs (policy allow-lists
/// and visible capability set) so semantically equivalent projections do not
/// churn when callers permute allow-list values or registry insertion order
/// changes. Visibility remains informational only; invocation authority is
/// re-checked by [`HostRuntime::invoke_capability`].
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleCapabilitySurface {
    /// Stable token for the semantic visible surface under this request policy.
    pub version: CapabilitySurfaceVersion,
    /// Typed visible capabilities, including access status and selected
    /// resource estimate.
    pub capabilities: Vec<VisibleCapability>,
}

/// Successful capability completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityCompleted {
    pub capability_id: CapabilityId,
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
}

/// Approval suspension state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeApprovalGate {
    pub approval_request_id: ApprovalRequestId,
    pub capability_id: CapabilityId,
    pub reason: RuntimeBlockedReason,
}

/// Auth/credential suspension state.
#[derive(Clone)]
pub struct RuntimeAuthGate {
    pub gate_id: RuntimeGateId,
    pub capability_id: CapabilityId,
    pub reason: RuntimeBlockedReason,
    pub required_secrets: Vec<SecretHandle>,
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    provider_diagnostic: Option<Box<ProviderDiagnostic>>,
}

impl RuntimeAuthGate {
    pub fn new(
        gate_id: RuntimeGateId,
        capability_id: CapabilityId,
        reason: RuntimeBlockedReason,
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Self {
        Self {
            gate_id,
            capability_id,
            reason,
            required_secrets,
            credential_requirements,
            provider_diagnostic: None,
        }
    }

    pub fn with_provider_diagnostic(mut self, diagnostic: Option<Box<ProviderDiagnostic>>) -> Self {
        self.provider_diagnostic = diagnostic;
        self
    }

    pub fn provider_diagnostic(&self) -> Option<&ProviderDiagnostic> {
        self.provider_diagnostic.as_deref()
    }
}

impl fmt::Debug for RuntimeAuthGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `required_secrets` handle names are omitted in full and reduced to a
        // count, matching `DispatchError::AuthRequired`'s Debug convention
        // (`ironclaw_host_api::dispatch`) so secret-handle identifiers never
        // reach logs from this gate either.
        formatter
            .debug_struct("RuntimeAuthGate")
            .field("gate_id", &self.gate_id)
            .field("capability_id", &self.capability_id)
            .field("reason", &self.reason)
            .field(
                "required_secrets",
                &format!("[{} handle(s) redacted]", self.required_secrets.len()),
            )
            .field("credential_requirements", &self.credential_requirements)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RuntimeAuthGate {
    fn eq(&self, other: &Self) -> bool {
        self.gate_id == other.gate_id
            && self.capability_id == other.capability_id
            && self.reason == other.reason
            && self.required_secrets == other.required_secrets
            && self.credential_requirements == other.credential_requirements
    }
}

impl Eq for RuntimeAuthGate {}

#[cfg(test)]
mod runtime_auth_gate_debug_tests {
    use super::*;

    #[test]
    fn debug_redacts_required_secret_handle_names() {
        let gate = RuntimeAuthGate::new(
            RuntimeGateId::new(),
            CapabilityId::new("notion.search").unwrap(),
            RuntimeBlockedReason::AuthRequired,
            vec![SecretHandle::new("notion-oauth-token").unwrap()],
            Vec::new(),
        );

        let rendered = format!("{gate:?}");

        assert!(!rendered.contains("notion-oauth-token"), "{rendered}");
        assert!(rendered.contains("1 handle(s) redacted"), "{rendered}");
    }
}

/// Resource suspension state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeResourceGate {
    pub gate_id: RuntimeGateId,
    pub capability_id: CapabilityId,
    pub reason: RuntimeBlockedReason,
    pub estimate: ResourceEstimate,
}

/// Spawned/background process summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProcessHandle {
    pub process_id: ProcessId,
    pub capability_id: CapabilityId,
}

/// Sanitized capability failure outcome.
///
/// `message` is the public label: it is persisted into run-state rows,
/// published on the runtime event sink, and rendered by product surfaces, so
/// producers keep it host-authored/strict-validated (wild raw causes degrade
/// to the kind's fixed sentence). The raw descriptive cause rides
/// `model_visible_cause` instead — an in-process-only channel.
#[derive(Clone, Eq)]
pub struct RuntimeCapabilityFailure {
    pub capability_id: CapabilityId,
    pub kind: FailureKind,
    pub message: Option<String>,
    pub detail: Option<DispatchFailureDetail>,
    /// Registry-scrubbed descriptive cause for the model-visible Diagnostic
    /// channel ONLY. Deliberately absent from `Debug`/`PartialEq` and never
    /// persisted or published by run-state/event writers — the loop-support
    /// seam (`runtime_failure_diagnostic_detail`) re-scrubs and injection-
    /// fences it before it reaches the model.
    model_visible_cause: Option<String>,
    /// Whether the failed capability is bound to a standard messaging WRITE
    /// op (`descriptor.standard_op.map(|op| op.is_write()) == Some(true)`).
    /// Read only by [`RuntimeCapabilityFailure::disposition`]'s retry
    /// carve-out (pre-merge amendment W1): retrying a write blind risks a
    /// duplicate side effect the model cannot see or undo (e.g. a message
    /// sent twice), so a write's retryable-kind failure must never resolve to
    /// `RetrySameCall` — the model decides whether to retry a write, not the
    /// host. Deliberately excluded from `Debug`/`PartialEq`, mirroring
    /// `model_visible_cause`: a construction-time policy input, not part of
    /// the failure's public identity.
    is_standard_write: bool,
}

impl fmt::Debug for RuntimeCapabilityFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `model_visible_cause` is intentionally omitted and raw Diagnostic
        // text is redacted: Debug renders flow into tracing logs and
        // test/public assertions, and either channel may carry backend paths
        // or provider text. Structured invalid-input and host-remediation
        // details remain useful and are already bounded by their contracts.
        let mut debug = f.debug_struct("RuntimeCapabilityFailure");
        debug
            .field("capability_id", &self.capability_id)
            .field("kind", &self.kind)
            .field("message", &self.message);
        match &self.detail {
            Some(DispatchFailureDetail::Diagnostic { .. }) => {
                debug.field("detail", &"<diagnostic redacted>");
            }
            detail => {
                debug.field("detail", detail);
            }
        }
        debug.finish_non_exhaustive()
    }
}

impl PartialEq for RuntimeCapabilityFailure {
    fn eq(&self, other: &Self) -> bool {
        // Mirror the `Debug` exclusion: `model_visible_cause` is a private
        // diagnostic channel, so equality compares only the public fields.
        // Otherwise two failures differing only in the hidden cause would fail
        // `assert_eq!` while their `Debug` diffs print identical.
        self.capability_id == other.capability_id
            && self.kind == other.kind
            && self.message == other.message
            && self.detail == other.detail
    }
}

/// Outcomes returned by capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCapabilityOutcome {
    Completed(Box<RuntimeCapabilityCompleted>),
    ApprovalRequired(RuntimeApprovalGate),
    AuthRequired(RuntimeAuthGate),
    ResourceBlocked(RuntimeResourceGate),
    SpawnedProcess(RuntimeProcessHandle),
    Failed(RuntimeCapabilityFailure),
}

impl RuntimeCapabilityOutcome {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Completed(_) => "completed",
            Self::ApprovalRequired(_) => "approval_required",
            Self::AuthRequired(_) => "auth_required",
            Self::ResourceBlocked(_) => "resource_blocked",
            Self::SpawnedProcess(_) => "spawned_process",
            Self::Failed(_) => "failed",
        }
    }
}

// `RuntimeCapabilityOutcome` is an in-process host-runtime return value, never
// a wire type: it carries capability output `serde_json::Value`s alongside
// internal gate/failure state, and nothing downstream is entitled to
// serialize or deserialize it directly (projections and transports build
// their own typed wire shapes from it). Pin that with a compile-time check so
// an incidental `#[derive(Serialize)]`/`#[derive(Deserialize)]` added later
// fails the build instead of silently opening a serialization surface.
static_assertions::assert_not_impl_any!(
    RuntimeCapabilityOutcome: serde::Serialize,
    serde::de::DeserializeOwned
);

/// Stable reasons for capability suspension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeBlockedReason {
    ApprovalRequired,
    AuthRequired,
    ResourceLimit,
    ResourceUnavailable,
}

/// Opt-in local diagnostic switch for raw HTTP egress failures.
///
/// Raw transport errors can contain URLs, query strings, host paths, proxy
/// details, or credential-shaped text. Keep this disabled unless debugging a
/// trusted `Standalone` or `LocalYolo` run. Hosted and enterprise deployments
/// never enable raw diagnostics from this environment variable alone.
pub(crate) const UNSAFE_RAW_HTTP_EGRESS_ERRORS_ENV: &str = "IRONCLAW_UNSAFE_RAW_HTTP_EGRESS_ERRORS";

pub(crate) fn runtime_policy_allows_unsafe_raw_http_diagnostics(
    policy: Option<&EffectiveRuntimePolicy>,
) -> bool {
    policy.is_some_and(|policy| {
        local_runtime_allows_unsafe_raw_http_diagnostics(policy.deployment, policy.resolved_profile)
    })
}

pub(crate) fn local_runtime_allows_unsafe_raw_http_diagnostics(
    deployment: DeploymentMode,
    profile: RuntimeProfile,
) -> bool {
    matches!(deployment, DeploymentMode::LocalSingleUser)
        && matches!(
            profile,
            RuntimeProfile::LocalHost | RuntimeProfile::LocalYolo
        )
}

pub(crate) fn unsafe_raw_http_diagnostics_enabled(runtime_allows_raw: bool) -> bool {
    runtime_allows_raw && env::var(UNSAFE_RAW_HTTP_EGRESS_ERRORS_ENV).as_deref() == Ok("1")
}

#[cfg(test)]
mod raw_http_diagnostic_policy_tests {
    use super::*;

    #[test]
    fn raw_http_diagnostics_are_limited_to_standalone_and_yolo_profiles() {
        assert!(local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalHost,
        ));
        assert!(local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalYolo,
        ));
        assert!(!local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalSafe,
        ));
        assert!(!local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedYoloTenantScoped,
        ));
        assert!(!local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseYoloDedicated,
        ));
    }
}

/// Agent-loop handling decision for a sanitized runtime capability failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityFailureDisposition {
    /// Return a normal tool error observation to the model in the same loop.
    ModelVisibleToolError,
    /// Retry the same runtime invocation before exposing anything to the model.
    /// The loop recovery strategy owns the retry budget and post-exhaustion
    /// fallback; the host-runtime disposition only classifies the first outcome.
    RetrySameCall,
}

const MAX_RUNTIME_FAILURE_SUMMARY_CHARS: usize = 512;

impl RuntimeCapabilityFailure {
    pub fn new(capability_id: CapabilityId, kind: FailureKind, message: Option<String>) -> Self {
        Self {
            capability_id,
            kind,
            message,
            detail: None,
            model_visible_cause: None,
            is_standard_write: false,
        }
    }

    pub fn with_detail(mut self, detail: DispatchFailureDetail) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Marks this failure as originating from a capability bound to a
    /// standard messaging write op. See the field doc for why this changes
    /// [`Self::disposition`]'s outcome.
    pub fn with_is_standard_write(mut self, is_standard_write: bool) -> Self {
        self.is_standard_write = is_standard_write;
        self
    }

    /// Attach the registry-scrubbed descriptive cause for the model-visible
    /// Diagnostic channel. Never rendered in `Debug`, run-state rows, or
    /// runtime events.
    pub fn with_model_visible_cause(mut self, cause: impl Into<String>) -> Self {
        self.model_visible_cause = Some(cause.into());
        self
    }

    /// Return the scrubbed cause for the loop adapter's model-visible
    /// Diagnostic seam. This value is never a public display label.
    pub fn model_visible_cause(&self) -> Option<&str> {
        self.model_visible_cause.as_deref()
    }

    pub fn safe_summary(&self) -> Option<String> {
        let summary = self.message.as_deref()?.trim();
        if summary.is_empty() {
            return None;
        }

        Some(bounded_runtime_failure_summary(summary))
    }

    pub fn disposition(&self) -> CapabilityFailureDisposition {
        capability_failure_disposition(self.kind, self.is_standard_write)
    }
}

fn bounded_runtime_failure_summary(summary: &str) -> String {
    const ELLIPSIS: &str = "...";
    let mut chars = summary.chars();
    let bounded: String = chars
        .by_ref()
        .take(MAX_RUNTIME_FAILURE_SUMMARY_CHARS)
        .collect();
    if chars.next().is_some() {
        let truncated_limit = MAX_RUNTIME_FAILURE_SUMMARY_CHARS - ELLIPSIS.chars().count();
        let bounded: String = bounded.chars().take(truncated_limit).collect();
        format!("{bounded}{ELLIPSIS}")
    } else {
        bounded
    }
}

/// Central disposition policy for runtime capability failures.
///
/// Delegates the recoverability decision to the unified
/// [`FailureKind::fate`] projection instead of re-deriving a local retryable
/// set — re-declared domains drift, and the drift is where recoverability
/// died (#6284). Only `Retry`-fated kinds are retried before the model sees
/// anything. `Park` and `Terminal` fates are not expected to reach this
/// disposition on the production paths (gates suspend as
/// `AuthRequired`/`ApprovalRequired` outcomes and cancellation ends the run
/// upstream — intent, not a code-enforced invariant); if a lane nevertheless
/// mints one, it conservatively surfaces as a model-visible tool error rather
/// than burning retry budget. Security
/// isolation failures must use a separate quarantine path instead of this
/// generic failure disposition.
///
/// `is_standard_write` carves out one exception to the retryable-kind rule
/// (pre-merge amendment W1): a capability bound to a standard messaging write
/// op (`StandardMessagingOp::is_write() == true`) must never receive
/// `RetrySameCall`, however transient/backend/network the failure kind looks.
/// A same-call retry after a write dispatch of unknown outcome risks a
/// duplicate side effect the model cannot see or undo (e.g. a message sent
/// twice) — the model decides whether to retry a write, not the host. Read
/// ops and bespoke tools (`is_standard_write == false`) keep today's
/// retry-by-kind behavior unchanged.
pub fn capability_failure_disposition(
    kind: FailureKind,
    is_standard_write: bool,
) -> CapabilityFailureDisposition {
    if is_standard_write && matches!(kind.fate(), FailureFate::Retry) {
        return CapabilityFailureDisposition::ModelVisibleToolError;
    }

    match kind.fate() {
        FailureFate::Retry => CapabilityFailureDisposition::RetrySameCall,
        FailureFate::ModelVisible | FailureFate::Park | FailureFate::Terminal => {
            CapabilityFailureDisposition::ModelVisibleToolError
        }
    }
}

/// Work ids tracked by the host runtime for status/cancellation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeWorkId {
    Invocation(ironclaw_host_api::ids::InvocationId),
    Process(ProcessId),
    Gate(RuntimeGateId),
}

/// Cancellation reason supplied by upper turn/loop services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CancelReason {
    UserRequested,
    TurnCancelled,
    Shutdown,
    Timeout,
}

/// Request to cancel active work in one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CancelRuntimeWorkRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
    pub reason: CancelReason,
}

impl CancelRuntimeWorkRequest {
    pub fn new(scope: ResourceScope, correlation_id: CorrelationId, reason: CancelReason) -> Self {
        Self {
            scope,
            correlation_id,
            reason,
        }
    }
}

/// Result of best-effort cancellation fanout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CancelRuntimeWorkOutcome {
    pub cancelled: Vec<RuntimeWorkId>,
    pub already_terminal: Vec<RuntimeWorkId>,
    pub unsupported: Vec<RuntimeWorkId>,
}

/// Request to inspect active work for a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuntimeStatusRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
}

impl RuntimeStatusRequest {
    pub fn new(scope: ResourceScope, correlation_id: CorrelationId) -> Self {
        Self {
            scope,
            correlation_id,
        }
    }
}

/// Redacted summary for active host runtime work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWorkSummary {
    pub work_id: RuntimeWorkId,
    pub capability_id: Option<CapabilityId>,
    pub runtime: Option<RuntimeKind>,
}

/// Redacted host runtime status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostRuntimeStatus {
    pub active_work: Vec<RuntimeWorkSummary>,
}

/// Host runtime readiness information.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostRuntimeHealth {
    pub ready: bool,
    pub missing_runtime_backends: Vec<RuntimeKind>,
}

/// Backend health probe for concrete runtime implementations.
///
/// The host runtime asks this port about the runtime kinds required by the
/// current visible capability registry. Implementations should return the
/// subset of `required` that is not currently available. Callers must treat a
/// missing probe as "unknown/unready" whenever the registry requires at least
/// one runtime backend.
#[async_trait]
pub trait RuntimeBackendHealth: Send + Sync {
    async fn missing_runtime_backends(
        &self,
        required: &[RuntimeKind],
    ) -> Result<Vec<RuntimeKind>, HostRuntimeError>;
}

/// Contract for the Reborn host runtime service.
pub type RuntimeInvocation = (ExecutionContext, CapabilityId, ResourceEstimate, Value);
pub type RuntimeApprovalResume = (
    ExecutionContext,
    ApprovalRequestId,
    CapabilityId,
    ResourceEstimate,
    Value,
);
pub type RuntimeAuthResume = (
    ExecutionContext,
    CapabilityId,
    ResourceEstimate,
    Value,
    Option<ApprovalRequestId>,
);
pub type RuntimeAuthDecline = (ExecutionContext, CapabilityId);

#[async_trait]
pub trait HostRuntime: Send + Sync {
    async fn invoke_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError>;

    /// Default: this host runtime does not implement capability spawn.
    ///
    /// The kind is [`FailureKind::UnsupportedRunner`] — model-visible and
    /// **non-retryable** — because "this implementation does not provide the
    /// operation" is a permanent property of the implementation, not a
    /// temporary outage. `Unavailable` would be `FailureFate::Retry` and would
    /// burn the whole capability retry budget on a call that can never succeed.
    async fn spawn_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (_, capability_id, _, _) = request;
        Ok(RuntimeCapabilityOutcome::Failed(
            RuntimeCapabilityFailure::new(
                capability_id,
                FailureKind::UnsupportedRunner,
                Some("capability spawn is unsupported by this host runtime".to_string()),
            ),
        ))
    }

    async fn resume_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError>;

    /// Re-dispatch after an auth gate has been resolved.
    ///
    /// Production hosts override this to route through
    /// `CapabilityHost::auth_resume_json` which handles the `BlockedAuth`
    /// run-state and optionally claims the prior approval lease.
    ///
    /// The default implementation returns an explicit `Failed` outcome so that
    /// test stubs that do not override this method fail loudly instead of
    /// silently falling back to a fresh `invoke_capability` call (which would
    /// bypass run-state validation and the approval-lease-claim path).  Any
    /// `HostRuntime` implementation that participates in auth-resume flows must
    /// provide an explicit override.
    ///
    /// The kind is [`FailureKind::UnsupportedRunner`] (non-retryable) for the
    /// same reason as [`HostRuntime::spawn_capability`]: a missing override is
    /// permanent, so retrying it only burns budget.
    async fn auth_resume_capability(
        &self,
        request: RuntimeAuthResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (_, capability_id, _, _, _) = request;
        Ok(RuntimeCapabilityOutcome::Failed(
            RuntimeCapabilityFailure::new(
                capability_id,
                FailureKind::UnsupportedRunner,
                Some("capability auth-resume is unsupported by this host runtime".to_string()),
            ),
        ))
    }

    /// Terminalize a capability invocation whose auth gate was denied by the
    /// user. Implementations must durably fail the exact blocked invocation and
    /// must not dispatch the capability. The default fails closed because it
    /// cannot provide that durable evidence.
    async fn decline_auth_capability(
        &self,
        _request: RuntimeAuthDecline,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable(
            "capability auth decline is unsupported by this host runtime",
        ))
    }

    /// Default: this host runtime does not implement spawn resume. Permanent,
    /// so [`FailureKind::UnsupportedRunner`] rather than the retryable
    /// `Unavailable` — see [`HostRuntime::spawn_capability`].
    async fn resume_spawn_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (_, _, capability_id, _, _) = request;
        Ok(RuntimeCapabilityOutcome::Failed(
            RuntimeCapabilityFailure::new(
                capability_id,
                FailureKind::UnsupportedRunner,
                Some("capability spawn resume is unsupported by this host runtime".to_string()),
            ),
        ))
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError>;

    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError>;

    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError>;

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError>;
}

/// Sanitized host runtime infrastructure/contract errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HostRuntimeError {
    #[error("invalid host runtime request: {reason}")]
    InvalidRequest { reason: String },
    #[error("host runtime unavailable: {reason}")]
    Unavailable { reason: String },
}

impl HostRuntimeError {
    pub fn invalid_request(reason: impl Into<String>) -> Self {
        Self::InvalidRequest {
            reason: reason.into(),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod unsupported_operation_default_tests {
    use super::*;
    use ironclaw_host_api::{
        capability::CapabilitySet, ids::UserId, mount::MountView, runtime::TrustClass,
    };

    /// A `HostRuntime` that implements only the required methods, so every
    /// optional operation falls through to the trait's default body.
    struct DefaultsOnlyRuntime;

    #[async_trait]
    impl HostRuntime for DefaultsOnlyRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeInvocation,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }

        async fn resume_capability(
            &self,
            _request: RuntimeApprovalResume,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("test only exercises the optional-operation defaults")
        }
    }

    fn context() -> ExecutionContext {
        ExecutionContext::local_default(
            UserId::new("user").expect("user id"), // safety: test-only static fixture
            ExtensionId::new("caller").expect("extension id"), // safety: test-only static fixture
            RuntimeKind::Wasm,
            TrustClass::UserTrusted,
            CapabilitySet::default(),
            MountView::default(),
        )
        .expect("execution context") // safety: test-only static fixture
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("echo.say").expect("capability id") // safety: test-only static fixture
    }

    fn failure(outcome: RuntimeCapabilityOutcome) -> RuntimeCapabilityFailure {
        match outcome {
            RuntimeCapabilityOutcome::Failed(failure) => failure,
            other => panic!("expected a Failed outcome, got {}", other.kind()),
        }
    }

    /// Regression (#6684 review): "this host runtime does not implement the
    /// operation" is a **permanent** property of the implementation — it cannot
    /// become true on the next attempt. Minting `FailureKind::Unavailable`
    /// (fate `Retry`) made the capability retry budget burn down to zero on a
    /// call that can never succeed, the same budget-burn class this PR fixed
    /// for `NetworkDenied`. The honest kind is `UnsupportedRunner`:
    /// model-visible, non-retryable.
    #[tokio::test]
    async fn unsupported_operation_defaults_are_permanent_not_retryable() {
        let runtime = DefaultsOnlyRuntime;

        let spawn = failure(
            runtime
                .spawn_capability((
                    context(),
                    capability_id(),
                    ResourceEstimate::default(),
                    Value::Null,
                ))
                .await
                .expect("default spawn body returns an outcome, not an error"), // safety: test-only assertion
        );
        let auth_resume = failure(
            runtime
                .auth_resume_capability((
                    context(),
                    capability_id(),
                    ResourceEstimate::default(),
                    Value::Null,
                    None,
                ))
                .await
                .expect("default auth-resume body returns an outcome, not an error"), // safety: test-only assertion
        );
        let spawn_resume = failure(
            runtime
                .resume_spawn_capability((
                    context(),
                    ApprovalRequestId::new(),
                    capability_id(),
                    ResourceEstimate::default(),
                    Value::Null,
                ))
                .await
                .expect("default spawn-resume body returns an outcome, not an error"), // safety: test-only assertion
        );

        for (label, failure) in [
            ("spawn_capability", spawn),
            ("auth_resume_capability", auth_resume),
            ("resume_spawn_capability", spawn_resume),
        ] {
            assert_eq! // safety: test-only assertion
            (
                failure.kind,
                FailureKind::UnsupportedRunner,
                "{label} default must name the permanent unsupported-operation kind"
            );
            assert! // safety: test-only assertion
            (
                !failure.kind.is_retryable(),
                "{label} default must not consume retry budget on a permanently unsupported operation"
            );
            assert_eq! // safety: test-only assertion
            (
                failure.kind.fate(),
                FailureFate::ModelVisible,
                "{label} default must surface to the model so it can route around the gap"
            );
        }
    }
}
