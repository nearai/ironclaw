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

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDescriptor, CapabilityId, CorrelationId, ExecutionContext,
    ProcessId, ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind, SecretHandle,
};
use ironclaw_trust::TrustDecision;
use serde_json::Value;
use std::fmt;
use thiserror::Error;

mod production;

pub use production::DefaultHostRuntime;

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
    /// Host-controlled trust decision for the package providing this capability.
    ///
    /// The host evaluates trust against its own policy before constructing this
    /// request — callers must not synthesize trust decisions or override the
    /// host's policy. The decision flows through to the underlying capability
    /// host so authorization, approval, and authority ceiling are derived from
    /// the host-validated trust posture, not caller-asserted claims.
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

/// Request to list host-filtered visible capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VisibleCapabilityRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
    /// Projection surface selection only; this is not authority and must not
    /// grant or bypass authorization. The host treats this as an opaque
    /// cache/version dimension; deciding which surface labels a given caller
    /// may request is an upper-layer concern.
    pub surface_kind: SurfaceKind,
}

impl VisibleCapabilityRequest {
    pub fn new(
        scope: ResourceScope,
        correlation_id: CorrelationId,
        surface_kind: SurfaceKind,
    ) -> Self {
        Self {
            scope,
            correlation_id,
            surface_kind,
        }
    }
}

/// Host-filtered visible capability surface.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleCapabilitySurface {
    pub version: CapabilitySurfaceVersion,
    pub descriptors: Vec<CapabilityDescriptor>,
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

/// Outcomes returned by capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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

/// Stable reasons for capability suspension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeBlockedReason {
    ApprovalRequired,
    AuthRequired,
    ResourceLimit,
    ResourceUnavailable,
}

/// Stable, sanitized failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeFailureKind {
    Authorization,
    Backend,
    Cancelled,
    Dispatcher,
    InvalidInput,
    MissingRuntime,
    Network,
    OutputTooLarge,
    Process,
    Resource,
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
            Self::InvalidInput => "invalid_input",
            Self::MissingRuntime => "missing_runtime",
            Self::Network => "network",
            Self::OutputTooLarge => "output_too_large",
            Self::Process => "process",
            Self::Resource => "resource",
            Self::Unknown => "unknown",
        }
    }
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

/// Contract for the Reborn host runtime facade.
#[async_trait]
pub trait HostRuntime: Send + Sync {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError>;

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
#[non_exhaustive]
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
