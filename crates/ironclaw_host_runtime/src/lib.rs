//! Host runtime facade for IronClaw Reborn.
//!
//! `ironclaw_host_runtime` is the narrow boundary upper Reborn services build
//! against. It surfaces both:
//!
//! - the [`HostRuntime`] trait — the stable contract upper turn/loop services
//!   depend on;
//! - [`DefaultHostRuntime`] — the production composition that wraps
//!   [`ironclaw_capabilities::CapabilityHost`] (which itself coordinates
//!   authorization, approvals, run-state lifecycle, and process spawn) behind
//!   that contract.
//!
//! The facade preserves three important boundaries:
//!
//! - callers see structured capability outcomes instead of lower substrate
//!   handles;
//! - approval/auth/resource waits are suspension states, not errors;
//! - caller/workflow origin taxonomy is intentionally kept outside this lower
//!   facade. Authority remains in [`ExecutionContext`] (principals, grants,
//!   leases, policy); projection selection is an opaque [`SurfaceKind`] label
//!   the host treats as a cache/version dimension only. Caller-authority
//!   filtering of which surface a particular UI or upper service is allowed to
//!   render is intentionally an upper-layer concern — the host does not bake
//!   in upper-stack vocabulary (e.g. agent loop / adapter / admin).
#![warn(unreachable_pub)]

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, CorrelationId, ExecutionContext, ExtensionId, ProcessId,
    ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind, SecretHandle,
    runtime_policy::{DeploymentMode, EffectiveRuntimePolicy, RuntimeProfile},
};
use ironclaw_trust::TrustDecision;
use serde_json::Value;
use std::{collections::BTreeMap, env, fmt};
use thiserror::Error;

mod capability_catalog;
mod egress;
mod extension_contracts;
mod first_party;
mod first_party_tools;
mod http_body;
mod invocation_services;
pub mod memory_context;
mod obligations;
mod planner;
mod process_aliases;
mod process_output;
mod process_port;
mod production;
mod sandbox_process;
mod services;
mod surface;
mod turn_scheduler;
mod wasm_credentials;

pub use capability_catalog::{
    HotCapabilityCatalog, HotCapabilityRecord, MAX_HOT_PROMPT_BYTES, MAX_HOT_SCHEMA_BYTES,
    publish_hot_capability_catalog,
};
pub use egress::HostHttpEgressService;
pub use extension_contracts::{
    default_host_api_contract_registry, default_host_port_catalog,
    discover_extensions_with_default_host_api_contracts,
    discover_extensions_with_default_host_api_contracts_and_catalog,
};
pub use first_party::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
pub use first_party_tools::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, BuiltinFirstPartyTools,
    ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID,
    HTTP_SAVE_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID, TIME_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers, builtin_first_party_package,
};
pub use http_body::{RuntimeHttpBodyStore, RuntimeHttpBodyStoreError};
pub use invocation_services::{
    InvocationServices, InvocationServicesError, InvocationServicesResolutionRequest,
    InvocationServicesResolver, LocalInvocationServicesResolver,
};
pub use obligations::{
    BuiltinObligationHandler, BuiltinObligationServices, LEAK_REDACT_FAILED_CODE,
    ProcessObligationLifecycleStore,
};
pub use planner::{ExecutionPlan, PlannerError, plan_capability};
pub use process_output::{SavedCommandOutput, SavedCommandOutputSanitization};
pub use process_port::{
    CommandExecutionOutput, CommandExecutionRequest, LocalHostProcessPort, RuntimeProcessError,
    RuntimeProcessPort, SandboxCommandTransport, TenantSandboxProcessPort,
};
pub use production::DefaultHostRuntime;
pub use sandbox_process::{
    RebornSandboxConfig, RebornSandboxContainerIdentity, RebornSandboxNetworkBroker,
    RebornSandboxScopeKey, RebornSandboxSecretBroker, RebornSandboxWorkspaceMode,
    RebornScopedSandboxCommandTransport,
};
pub use services::{
    HostRuntimeServices, ProductAuthProviderRuntimePorts, ProductionEventStoreWiringError,
    ProductionWiringComponent, ProductionWiringConfig, ProductionWiringIssue,
    ProductionWiringIssueKind, ProductionWiringReport, RegisteredRuntimeHealth,
};
pub use surface::{CapabilitySurfacePolicy, VisibleCapability, VisibleCapabilityAccess};
pub use turn_scheduler::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};

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
/// render; this lower facade simply returns the projection associated with
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

/// Request to invoke one capability through the composed host runtime.
///
/// Caller/workflow origin is intentionally not part of this lower contract.
/// Host runtime authorization must be derived from [`ExecutionContext`],
/// principals, grants, leases, and policy; upper workflow services can attach
/// audit labels outside this facade when they need product-specific origin
/// vocabulary.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RuntimeCapabilityRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    /// Advisory pre-flight estimate supplied by the caller.
    ///
    /// Production host-runtime implementations must treat this as a hint only:
    /// resource authorization, reservation, and reconciliation remain host-owned
    /// and must not trust caller estimates as binding limits or actual usage.
    pub estimate: ResourceEstimate,
    pub input: Value,
    /// Caller-supplied dedup hint.
    ///
    /// **This field is currently advisory at this layer.** The composed
    /// capability host does not yet implement caller-driven idempotent
    /// retries, so two `invoke_capability` calls carrying the same key will
    /// both execute. Upper turn/loop services that need at-most-once
    /// semantics must dedupe themselves until idempotency lands in the
    /// capability host. The field is kept on the contract surface so that
    /// shape doesn't break when dedup is wired through downstream.
    ///
    /// The host runtime still validates and forwards the key into
    /// observability spans for audit/tracing.
    pub idempotency_key: Option<IdempotencyKey>,
    /// Legacy caller-supplied trust decision kept for transitional request-shape
    /// compatibility.
    ///
    /// [`DefaultHostRuntime`](crate::DefaultHostRuntime) ignores this value: it
    /// resolves the capability provider's package identity, evaluates the
    /// host-owned policy, stamps the resulting effective trust onto the
    /// execution context, and passes that host-owned decision to the capability
    /// host. Callers must not rely on this field to widen or narrow authority.
    pub trust_decision: TrustDecision,
}

impl RuntimeCapabilityRequest {
    pub fn new(
        context: ExecutionContext,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: Value,
        trust_decision: TrustDecision,
    ) -> Self {
        Self {
            context,
            capability_id,
            estimate,
            input,
            idempotency_key: None,
            trust_decision,
        }
    }

    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
    }
}

/// Request to resume one approval-blocked capability through the composed host runtime.
///
/// The shape mirrors [`RuntimeCapabilityRequest`] but additionally carries the
/// approval request selected by an upper approval workflow. Like invoke requests,
/// `trust_decision` is transitional compatibility data: the default host runtime
/// evaluates provider trust itself before delegating to `CapabilityHost`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RuntimeCapabilityResumeRequest {
    pub context: ExecutionContext,
    pub approval_request_id: ApprovalRequestId,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
    pub idempotency_key: Option<IdempotencyKey>,
    pub trust_decision: TrustDecision,
}

impl RuntimeCapabilityResumeRequest {
    pub fn new(
        context: ExecutionContext,
        approval_request_id: ApprovalRequestId,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: Value,
        trust_decision: TrustDecision,
    ) -> Self {
        Self {
            context,
            approval_request_id,
            capability_id,
            estimate,
            input,
            idempotency_key: None,
            trust_decision,
        }
    }

    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuthGate {
    pub gate_id: RuntimeGateId,
    pub capability_id: CapabilityId,
    pub reason: RuntimeBlockedReason,
    pub required_secrets: Vec<SecretHandle>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityFailure {
    pub capability_id: CapabilityId,
    pub kind: RuntimeFailureKind,
    pub message: Option<String>,
}

/// Explicit fallback for outcome categories that the loop adapter cannot handle
/// yet. New first-class outcome variants should be added to
/// [`RuntimeCapabilityOutcome`] and exhaustively mapped by consumers instead of
/// being hidden behind wildcard matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityUnknown {
    pub capability_id: CapabilityId,
    pub kind: String,
    pub message: Option<String>,
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
    Unknown(RuntimeCapabilityUnknown),
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
            Self::Unknown(_) => "unknown",
        }
    }
}

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
/// trusted `LocalDev` or `LocalYolo` run. Hosted and enterprise deployments
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
            RuntimeProfile::LocalDev | RuntimeProfile::LocalYolo
        )
}

pub(crate) fn unsafe_raw_http_diagnostics_enabled(runtime_allows_raw: bool) -> bool {
    runtime_allows_raw && env::var(UNSAFE_RAW_HTTP_EGRESS_ERRORS_ENV).as_deref() == Ok("1")
}

#[cfg(test)]
mod raw_http_diagnostic_policy_tests {
    use super::*;

    #[test]
    fn raw_http_diagnostics_are_limited_to_local_dev_and_yolo_profiles() {
        assert!(local_runtime_allows_unsafe_raw_http_diagnostics(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalDev,
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

/// Stable, sanitized failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeFailureKind {
    Authorization,
    Backend,
    Cancelled,
    Dispatcher,
    Internal,
    InvalidInput,
    InvalidOutput,
    MissingRuntime,
    Network,
    OperationFailed,
    OutputTooLarge,
    PolicyDenied,
    Process,
    Resource,
    Transient,
    Unavailable,
    Unknown,
}

impl RuntimeFailureKind {
    /// Returns a stable, snake_case identifier for use in metrics/tracing.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Backend => "backend",
            Self::Cancelled => "cancelled",
            Self::Dispatcher => "dispatcher",
            Self::Internal => "internal",
            Self::InvalidInput => "invalid_input",
            Self::InvalidOutput => "invalid_output",
            Self::MissingRuntime => "missing_runtime",
            Self::Network => "network",
            Self::OperationFailed => "operation_failed",
            Self::OutputTooLarge => "output_too_large",
            Self::PolicyDenied => "policy_denied",
            Self::Process => "process",
            Self::Resource => "resource",
            Self::Transient => "transient",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
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
    pub fn new(
        capability_id: CapabilityId,
        kind: RuntimeFailureKind,
        message: Option<String>,
    ) -> Self {
        Self {
            capability_id,
            kind,
            message,
        }
    }

    pub fn safe_summary(&self) -> Option<String> {
        let summary = self.message.as_deref()?.trim();
        if summary.is_empty() {
            return None;
        }

        Some(bounded_runtime_failure_summary(summary))
    }

    pub fn disposition(&self) -> CapabilityFailureDisposition {
        capability_failure_disposition(self.kind)
    }
}

fn bounded_runtime_failure_summary(summary: &str) -> String {
    let mut chars = summary.chars();
    let bounded: String = chars
        .by_ref()
        .take(MAX_RUNTIME_FAILURE_SUMMARY_CHARS)
        .collect();
    if chars.next().is_some() {
        format!("{bounded}...")
    } else {
        bounded
    }
}

/// Central disposition policy for runtime capability failures.
///
/// Runtime failures should be surfaced through normal model-visible tool-error
/// handling whenever they are not retryable infrastructure outages. Security
/// isolation failures must use a separate quarantine path instead of this
/// generic failure disposition.
pub const fn capability_failure_disposition(
    kind: RuntimeFailureKind,
) -> CapabilityFailureDisposition {
    if matches!(kind, RuntimeFailureKind::InvalidInput) {
        return CapabilityFailureDisposition::ModelVisibleToolError;
    }

    if runtime_failure_is_retryable(kind) {
        return CapabilityFailureDisposition::RetrySameCall;
    }

    CapabilityFailureDisposition::ModelVisibleToolError
}

const fn runtime_failure_is_retryable(kind: RuntimeFailureKind) -> bool {
    matches!(
        kind,
        RuntimeFailureKind::Internal
            | RuntimeFailureKind::Backend
            | RuntimeFailureKind::Network
            | RuntimeFailureKind::Transient
            | RuntimeFailureKind::Unavailable
    )
}

/// Work ids tracked by the host runtime for status/cancellation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeWorkId {
    Invocation(ironclaw_host_api::InvocationId),
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

/// Contract for the Reborn host runtime facade.
#[async_trait]
pub trait HostRuntime: Send + Sync {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError>;

    async fn spawn_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Ok(RuntimeCapabilityOutcome::Failed(
            RuntimeCapabilityFailure::new(
                request.capability_id,
                RuntimeFailureKind::Unavailable,
                Some("capability spawn is unsupported by this host runtime".to_string()),
            ),
        ))
    }

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError>;

    async fn resume_spawn_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Ok(RuntimeCapabilityOutcome::Failed(
            RuntimeCapabilityFailure::new(
                request.capability_id,
                RuntimeFailureKind::Unavailable,
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
