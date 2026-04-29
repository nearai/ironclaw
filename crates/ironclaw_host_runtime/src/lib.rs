//! Contract-first host runtime facade for IronClaw Reborn.
//!
//! `ironclaw_host_runtime` is the narrow boundary upper Reborn services build
//! against. It is intentionally a contract and testkit first: production
//! composition will wire the concrete capability host, dispatcher, runtimes,
//! process services, network, secrets, filesystem, resources, and events in a
//! later slice.
//!
//! The facade preserves two important boundaries:
//!
//! - callers see structured capability outcomes instead of lower substrate
//!   handles;
//! - approval/auth/resource waits are suspension states, not errors.

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
        let value = value.into();
        if value.is_empty() {
            return Err(HostRuntimeError::invalid_request(
                "idempotency key must not be empty",
            ));
        }
        if value.len() > 256 {
            return Err(HostRuntimeError::invalid_request(
                "idempotency key must be at most 256 bytes",
            ));
        }
        if value.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(HostRuntimeError::invalid_request(
                "idempotency key must not contain NUL/control characters",
            ));
        }
        Ok(Self(value))
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
        let value = value.into();
        if value.is_empty() {
            return Err(HostRuntimeError::invalid_request(
                "capability surface version must not be empty",
            ));
        }
        if value.len() > 128 {
            return Err(HostRuntimeError::invalid_request(
                "capability surface version must be at most 128 bytes",
            ));
        }
        if value.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(HostRuntimeError::invalid_request(
                "capability surface version must not contain NUL/control characters",
            ));
        }
        Ok(Self(value))
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

/// Upper-layer caller category for audit/projection context.
///
/// This is not authority. Authority still comes from the scoped execution
/// context, grants, leases, and kernel policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeCaller {
    TurnCoordinator,
    AgentLoopHost,
    Adapter,
    SystemService,
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
    pub caller: RuntimeCaller,
    pub idempotency_key: Option<IdempotencyKey>,
}

/// Request to list host-filtered visible capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleCapabilityRequest {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
    pub caller: RuntimeCaller,
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
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Debug, Default)]
    struct FakeHostRuntimeState {
        outcomes: VecDeque<RuntimeCapabilityOutcome>,
        visible_surfaces: VecDeque<VisibleCapabilitySurface>,
        cancelled_work: VecDeque<Vec<RuntimeWorkId>>,
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

        pub fn with_outcome(self, outcome: RuntimeCapabilityOutcome) -> Self {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .outcomes
                .push_back(outcome);
            self
        }

        pub fn with_visible_surface(self, surface: VisibleCapabilitySurface) -> Self {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .visible_surfaces
                .push_back(surface);
            self
        }

        pub fn with_cancelled_work(self, work: Vec<RuntimeWorkId>) -> Self {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .cancelled_work
                .push_back(work);
            self
        }

        pub fn with_status(self, status: HostRuntimeStatus) -> Self {
            self.state.lock().expect("fake mutex poisoned").status = status;
            self
        }

        pub fn with_health(self, health: HostRuntimeHealth) -> Self {
            self.state.lock().expect("fake mutex poisoned").health = health;
            self
        }

        pub fn recorded_invocations(&self) -> Vec<RuntimeCapabilityRequest> {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .invocations
                .clone()
        }

        pub fn recorded_visible_capability_requests(&self) -> Vec<VisibleCapabilityRequest> {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .visible_requests
                .clone()
        }

        pub fn recorded_cancellations(&self) -> Vec<CancelRuntimeWorkRequest> {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .cancellations
                .clone()
        }

        pub fn recorded_status_requests(&self) -> Vec<RuntimeStatusRequest> {
            self.state
                .lock()
                .expect("fake mutex poisoned")
                .status_requests
                .clone()
        }

        pub fn recorded_health_calls(&self) -> usize {
            self.state.lock().expect("fake mutex poisoned").health_calls
        }
    }

    #[async_trait]
    impl HostRuntime for FakeHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            let mut state = self.state.lock().expect("fake mutex poisoned");
            state.invocations.push(request);
            state.outcomes.pop_front().ok_or_else(|| {
                HostRuntimeError::unavailable("scripted invoke_capability outcome exhausted")
            })
        }

        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            let mut state = self.state.lock().expect("fake mutex poisoned");
            state.visible_requests.push(request);
            state.visible_surfaces.pop_front().ok_or_else(|| {
                HostRuntimeError::unavailable("scripted visible_capabilities surface exhausted")
            })
        }

        async fn cancel_work(
            &self,
            request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            let mut state = self.state.lock().expect("fake mutex poisoned");
            state.cancellations.push(request);
            let cancelled = state.cancelled_work.pop_front().unwrap_or_default();
            Ok(CancelRuntimeWorkOutcome {
                cancelled,
                already_terminal: Vec::new(),
                unsupported: Vec::new(),
            })
        }

        async fn runtime_status(
            &self,
            request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            let mut state = self.state.lock().expect("fake mutex poisoned");
            state.status_requests.push(request);
            Ok(state.status.clone())
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            let mut state = self.state.lock().expect("fake mutex poisoned");
            state.health_calls += 1;
            Ok(state.health.clone())
        }
    }
}
