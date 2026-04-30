//! Contract-first host runtime facade for IronClaw Reborn.
//!
//! `ironclaw_host_runtime` is the narrow boundary upper Reborn services build
//! against. It is intentionally a contract and testkit first: production
//! composition will wire the concrete capability host, dispatcher, runtimes,
//! process services, network, secrets, filesystem, resources, and events in a
//! later slice.
//!
//! The facade preserves three important boundaries:
//!
//! - callers see structured capability outcomes instead of lower substrate
//!   handles;
//! - approval/auth/resource waits are suspension states, not errors;
//! - caller/workflow origin taxonomy is intentionally kept outside this lower
//!   facade; authority remains in `ExecutionContext` and projection selection
//!   remains explicit surface metadata.

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDescriptor, CapabilityId, CorrelationId, ExecutionContext,
    ProcessId, ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind, SecretHandle,
};
use serde_json::Value;
use std::fmt;
use thiserror::Error;

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

/// Which host-filtered surface a caller is asking to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CapabilitySurfaceKind {
    AgentLoop,
    Adapter,
    Admin,
}

/// Request to invoke one capability through the composed host runtime.
///
/// Caller/workflow origin is intentionally not part of this lower contract.
/// Host runtime authorization must be derived from [`ExecutionContext`],
/// principals, grants, leases, and policy; upper workflow services can attach
/// audit labels outside this facade when they need product-specific origin
/// vocabulary.
#[derive(Debug, Clone, PartialEq)]
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
    pub idempotency_key: Option<IdempotencyKey>,
}

/// Request to list host-filtered visible capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleCapabilityRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
    /// Projection surface selection only; this is not authority and must not
    /// grant or bypass authorization.
    pub surface_kind: CapabilitySurfaceKind,
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
pub struct CancelRuntimeWorkRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
    pub reason: CancelReason,
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
pub struct RuntimeStatusRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
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

/// Testkit for upper Reborn stack contract tests.
pub mod testkit {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Mutex, MutexGuard},
    };

    #[derive(Debug, Default)]
    struct FakeHostRuntimeState {
        outcomes: VecDeque<RuntimeCapabilityOutcome>,
        visible_surfaces: VecDeque<VisibleCapabilitySurface>,
        cancel_outcomes: VecDeque<CancelRuntimeWorkOutcome>,
        status: HostRuntimeStatus,
        health: HostRuntimeHealth,
        invocations: Vec<RuntimeCapabilityRequest>,
        visible_requests: Vec<VisibleCapabilityRequest>,
        cancellations: Vec<CancelRuntimeWorkRequest>,
        status_requests: Vec<RuntimeStatusRequest>,
        health_calls: usize,
    }

    /// Scripted fake host runtime.
    ///
    /// The fake records calls and returns queued outcomes. It intentionally does
    /// not simulate authorization, resources, processes, secrets, network, or
    /// events; those semantics belong to kernel/component tests.
    #[derive(Debug, Default)]
    pub struct FakeHostRuntime {
        state: Mutex<FakeHostRuntimeState>,
    }

    impl FakeHostRuntime {
        pub fn new() -> Self {
            Self::default()
        }

        fn lock_state(&self) -> MutexGuard<'_, FakeHostRuntimeState> {
            match self.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            }
        }

        pub fn with_outcome(self, outcome: RuntimeCapabilityOutcome) -> Self {
            self.lock_state().outcomes.push_back(outcome);
            self
        }

        pub fn with_visible_surface(self, surface: VisibleCapabilitySurface) -> Self {
            self.lock_state().visible_surfaces.push_back(surface);
            self
        }

        pub fn with_cancel_outcome(self, outcome: CancelRuntimeWorkOutcome) -> Self {
            self.lock_state().cancel_outcomes.push_back(outcome);
            self
        }

        pub fn with_status(self, status: HostRuntimeStatus) -> Self {
            self.lock_state().status = status;
            self
        }

        pub fn with_health(self, health: HostRuntimeHealth) -> Self {
            self.lock_state().health = health;
            self
        }

        pub fn recorded_invocations(&self) -> Vec<RuntimeCapabilityRequest> {
            self.lock_state().invocations.clone()
        }

        pub fn recorded_visible_capability_requests(&self) -> Vec<VisibleCapabilityRequest> {
            self.lock_state().visible_requests.clone()
        }

        pub fn recorded_cancellations(&self) -> Vec<CancelRuntimeWorkRequest> {
            self.lock_state().cancellations.clone()
        }

        pub fn recorded_status_requests(&self) -> Vec<RuntimeStatusRequest> {
            self.lock_state().status_requests.clone()
        }

        pub fn recorded_health_calls(&self) -> usize {
            self.lock_state().health_calls
        }
    }

    #[async_trait]
    impl HostRuntime for FakeHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            let mut state = self.lock_state();
            state.invocations.push(request);
            state.outcomes.pop_front().ok_or_else(|| {
                HostRuntimeError::unavailable("scripted invoke_capability outcome exhausted")
            })
        }

        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            let mut state = self.lock_state();
            state.visible_requests.push(request);
            state.visible_surfaces.pop_front().ok_or_else(|| {
                HostRuntimeError::unavailable("scripted visible_capabilities surface exhausted")
            })
        }

        async fn cancel_work(
            &self,
            request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            let mut state = self.lock_state();
            state.cancellations.push(request);
            state.cancel_outcomes.pop_front().ok_or_else(|| {
                HostRuntimeError::unavailable("scripted cancel_work outcome exhausted")
            })
        }

        async fn runtime_status(
            &self,
            request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            let mut state = self.lock_state();
            state.status_requests.push(request);
            Ok(state.status.clone())
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            let mut state = self.lock_state();
            state.health_calls += 1;
            Ok(state.health.clone())
        }
    }
}
