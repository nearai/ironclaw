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
    NetworkPolicy, ProcessId, ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind,
    SecretHandle,
};
use ironclaw_host_api::{
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeCredentialTarget,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    is_sensitive_runtime_request_header, is_sensitive_runtime_response_header,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse,
};
use ironclaw_safety::{LeakDetector, params_contain_manual_credentials};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use ironclaw_trust::TrustDecision;
use secrecy::ExposeSecret;
use serde_json::Value;
use std::{fmt, sync::Arc};
use thiserror::Error;

mod obligations;
mod production;
mod services;

pub use obligations::{
    BuiltinObligationHandler, BuiltinObligationServices, NetworkObligationPolicyStore,
    ProcessObligationLifecycleStore, RuntimeSecretInjectionStore, RuntimeSecretInjectionStoreError,
};
pub use production::DefaultHostRuntime;
pub use services::{HostRuntimeServices, RegisteredRuntimeHealth};

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

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkPolicySource {
    StagedObligation,
    RequestPolicyFallback,
}

#[derive(Debug, Clone)]
pub struct HostHttpEgressService<N, S> {
    network: N,
    secrets: S,
    secret_injections: Option<Arc<RuntimeSecretInjectionStore>>,
    network_policy_store: Option<Arc<NetworkObligationPolicyStore>>,
    network_policy_source: NetworkPolicySource,
}

impl<N, S> HostHttpEgressService<N, S> {
    /// Construct host HTTP egress in production fail-closed mode.
    ///
    /// Callers must attach a [`NetworkObligationPolicyStore`] with
    /// [`Self::with_network_policy_store`] before executing network requests.
    /// Without that store, egress fails before transport rather than trusting a
    /// caller-supplied policy.
    pub fn new(network: N, secrets: S) -> Self {
        Self {
            network,
            secrets,
            secret_injections: None,
            network_policy_store: None,
            network_policy_source: NetworkPolicySource::StagedObligation,
        }
    }

    /// Construct host HTTP egress that uses the policy embedded in each request.
    ///
    /// This is intentionally named as a test/legacy seam: production Reborn
    /// runtime egress must consume staged `ApplyNetworkPolicy` handoffs from
    /// [`NetworkObligationPolicyStore`] instead of trusting runtime/caller
    /// request policy fields.
    pub fn new_with_request_policy_for_tests(network: N, secrets: S) -> Self {
        Self {
            network,
            secrets,
            secret_injections: None,
            network_policy_store: None,
            network_policy_source: NetworkPolicySource::RequestPolicyFallback,
        }
    }

    pub fn with_secret_injection_store(mut self, store: Arc<RuntimeSecretInjectionStore>) -> Self {
        self.secret_injections = Some(store);
        self
    }

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
        self.network_policy_store = Some(store);
        self.network_policy_source = NetworkPolicySource::StagedObligation;
        self
    }

    pub fn network(&self) -> &N {
        &self.network
    }

    pub fn secrets(&self) -> &S {
        &self.secrets
    }

    fn network_policy_for_request(
        &self,
        request: &mut RuntimeHttpEgressRequest,
    ) -> Result<NetworkPolicy, RuntimeHttpEgressError> {
        if let Some(store) = &self.network_policy_store {
            return store
                .take(&request.scope, &request.capability_id)
                .ok_or_else(|| RuntimeHttpEgressError::Network {
                    reason: "network_policy_missing".to_string(),
                    request_bytes: 0,
                    response_bytes: 0,
                });
        }

        match self.network_policy_source {
            NetworkPolicySource::StagedObligation => Err(RuntimeHttpEgressError::Network {
                reason: "network_policy_missing".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            }),
            NetworkPolicySource::RequestPolicyFallback => {
                Ok(std::mem::take(&mut request.network_policy))
            }
        }
    }
}

impl<N, S> RuntimeHttpEgress for HostHttpEgressService<N, S>
where
    N: NetworkHttpEgress,
    S: SecretStore,
{
    fn execute(
        &self,
        mut request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let network_policy = self.network_policy_for_request(&mut request)?;
        validate_runtime_request(&request)?;

        let mut redaction_values = Vec::new();
        let mut credential_materials = Vec::new();
        let credential_injections = std::mem::take(&mut request.credential_injections);
        for injection in &credential_injections {
            let Some(value) = credential_value_for_injection(
                &mut credential_materials,
                &self.secrets,
                self.secret_injections.as_deref(),
                &request,
                injection,
            )?
            else {
                continue;
            };
            apply_credential_injection(&mut request, &injection.target, &value)?;
            redaction_values.extend(redaction_values_for_secret(&value));
        }

        let response = self
            .network
            .execute(NetworkHttpRequest {
                scope: request.scope,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                policy: network_policy,
                response_body_limit: request.response_body_limit,
                timeout_ms: request.timeout_ms,
            })
            .map_err(runtime_network_error)?;
        let credentials_injected = !redaction_values.is_empty();
        let (response, response_redacted) = sanitize_runtime_response(response, &redaction_values)?;
        Ok(runtime_response(
            response,
            credentials_injected || response_redacted,
        ))
    }
}

struct RuntimeCredentialMaterialCacheEntry {
    key: RuntimeCredentialMaterialKey,
    value: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
enum RuntimeCredentialMaterialKey {
    SecretStoreLease {
        handle: SecretHandle,
    },
    StagedObligation {
        capability_id: CapabilityId,
        handle: SecretHandle,
    },
}

impl RuntimeCredentialMaterialKey {
    fn for_injection(injection: &RuntimeCredentialInjection) -> Self {
        match &injection.source {
            RuntimeCredentialSource::SecretStoreLease => Self::SecretStoreLease {
                handle: injection.handle.clone(),
            },
            RuntimeCredentialSource::StagedObligation { capability_id } => Self::StagedObligation {
                capability_id: capability_id.clone(),
                handle: injection.handle.clone(),
            },
        }
    }
}

fn credential_value_for_injection<S>(
    cache: &mut Vec<RuntimeCredentialMaterialCacheEntry>,
    secrets: &S,
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    request: &RuntimeHttpEgressRequest,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<String>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    let key = RuntimeCredentialMaterialKey::for_injection(injection);
    if let Some(entry) = cache.iter().find(|entry| entry.key == key) {
        return match &entry.value {
            Some(value) => Ok(Some(value.clone())),
            None => missing_runtime_credential(injection.required).map(|_| None),
        };
    }

    let value = secret_material_for_injection(secrets, secret_injections, request, injection)?
        .map(|material| material.expose_secret().to_string());
    cache.push(RuntimeCredentialMaterialCacheEntry {
        key,
        value: value.clone(),
    });
    Ok(value)
}

fn secret_material_for_injection<S>(
    secrets: &S,
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    request: &RuntimeHttpEgressRequest,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    match &injection.source {
        RuntimeCredentialSource::SecretStoreLease => {
            lease_secret_for_injection(secrets, request, injection)
        }
        RuntimeCredentialSource::StagedObligation { capability_id } => {
            take_staged_secret_for_injection(secret_injections, request, capability_id, injection)
        }
    }
}

fn take_staged_secret_for_injection(
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    request: &RuntimeHttpEgressRequest,
    capability_id: &CapabilityId,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError> {
    let Some(secret_injections) = secret_injections else {
        return missing_runtime_credential(injection.required);
    };
    match secret_injections.take(&request.scope, capability_id, &injection.handle) {
        Ok(Some(material)) => Ok(Some(material)),
        Ok(None) => missing_runtime_credential(injection.required),
        Err(_) => Err(RuntimeHttpEgressError::Credential {
            reason: "runtime credential injection store unavailable".to_string(),
        }),
    }
}

fn missing_runtime_credential(
    required: bool,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError> {
    if required {
        Err(RuntimeHttpEgressError::Credential {
            reason: "required credential is unavailable".to_string(),
        })
    } else {
        Ok(None)
    }
}

fn lease_secret_for_injection<S>(
    secrets: &S,
    request: &RuntimeHttpEgressRequest,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    let metadata = block_on_secret(secrets.metadata(&request.scope, &injection.handle))?;
    if metadata.is_none() {
        if injection.required {
            return Err(RuntimeHttpEgressError::Credential {
                reason: "required credential is unavailable".to_string(),
            });
        }
        return Ok(None);
    }
    let lease = block_on_secret(secrets.lease_once(&request.scope, &injection.handle))?;
    let material = block_on_secret(secrets.consume(&request.scope, lease.id))?;
    Ok(Some(material))
}

fn block_on_secret<T>(
    future: impl std::future::Future<Output = Result<T, SecretStoreError>> + Send,
) -> Result<T, RuntimeHttpEgressError>
where
    T: Send,
{
    let joined = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| RuntimeHttpEgressError::Credential {
                        reason: "secret store runtime unavailable".to_string(),
                    })?;
                runtime
                    .block_on(future)
                    .map_err(|error| RuntimeHttpEgressError::Credential {
                        reason: sanitized_secret_error(&error),
                    })
            })
            .join()
    });
    joined.unwrap_or_else(|_| {
        Err(RuntimeHttpEgressError::Credential {
            reason: "secret store worker panicked".to_string(),
        })
    })
}

fn sanitized_secret_error(error: &SecretStoreError) -> String {
    match error {
        SecretStoreError::UnknownSecret { .. } => "credential is unavailable".to_string(),
        SecretStoreError::UnknownLease { .. } => "credential lease is unavailable".to_string(),
        SecretStoreError::LeaseConsumed { .. } => "credential lease was already used".to_string(),
        SecretStoreError::LeaseRevoked { .. } => "credential lease was revoked".to_string(),
        SecretStoreError::StoreUnavailable { .. } => "credential store unavailable".to_string(),
    }
}

fn apply_credential_injection(
    request: &mut RuntimeHttpEgressRequest,
    target: &RuntimeCredentialTarget,
    value: &str,
) -> Result<(), RuntimeHttpEgressError> {
    match target {
        RuntimeCredentialTarget::Header { name, prefix } => {
            if !valid_injected_header_name(name) {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection header name is invalid".to_string(),
                });
            }
            let injected = match prefix {
                Some(prefix) => format!("{prefix}{value}"),
                None => value.to_string(),
            };
            if injected.chars().any(char::is_control) {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection header value is invalid".to_string(),
                });
            }
            request.headers.push((name.clone(), injected));
        }
        RuntimeCredentialTarget::QueryParam { name } => {
            let mut url =
                url::Url::parse(&request.url).map_err(|_| RuntimeHttpEgressError::Credential {
                    reason: "credential injection target URL is invalid".to_string(),
                })?;
            url.query_pairs_mut().append_pair(name, value);
            request.url = url.to_string();
        }
    }
    Ok(())
}

fn valid_injected_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn runtime_network_error(error: NetworkHttpError) -> RuntimeHttpEgressError {
    RuntimeHttpEgressError::Network {
        reason: error.stable_reason().to_string(),
        request_bytes: error.request_bytes(),
        response_bytes: error.response_bytes(),
    }
}

fn validate_runtime_request(
    request: &RuntimeHttpEgressRequest,
) -> Result<(), RuntimeHttpEgressError> {
    if let Some((name, _)) = request
        .headers
        .iter()
        .find(|(name, _)| is_sensitive_runtime_request_header(name))
    {
        return Err(RuntimeHttpEgressError::Request {
            reason: format!("sensitive_header_denied:{name}"),
            request_bytes: 0,
            response_bytes: 0,
        });
    }

    if runtime_request_contains_manual_credentials(request) {
        return Err(RuntimeHttpEgressError::Request {
            reason: "manual_credentials_denied".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    }

    let detector = LeakDetector::new();
    detector
        .scan_http_request(&request.url, &request.headers, Some(&request.body))
        .map_err(|_| runtime_request_leak_error())?;
    scan_decoded_url_for_leaks(&detector, &request.url)?;
    Ok(())
}

fn runtime_request_contains_manual_credentials(request: &RuntimeHttpEgressRequest) -> bool {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
        .collect::<Vec<_>>();
    let params = serde_json::json!({
        "url": request.url,
        "headers": headers,
    });
    params_contain_manual_credentials(&params)
}

fn scan_decoded_url_for_leaks(
    detector: &LeakDetector,
    raw_url: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return Ok(());
    };

    scan_component_for_leaks(detector, parsed.path())?;
    if let Some(query) = parsed.query() {
        scan_component_for_leaks(detector, query)?;
    }
    if !parsed.username().is_empty() {
        scan_component_for_leaks(detector, parsed.username())?;
    }
    if let Some(password) = parsed.password() {
        scan_component_for_leaks(detector, password)?;
    }
    for (name, value) in parsed.query_pairs() {
        detector
            .scan_and_clean(name.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
        detector
            .scan_and_clean(value.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
}

fn scan_component_for_leaks(
    detector: &LeakDetector,
    component: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let decoded = percent_decode_lossy(component);
    if decoded != component {
        detector
            .scan_and_clean(&decoded)
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn runtime_request_leak_error() -> RuntimeHttpEgressError {
    RuntimeHttpEgressError::Request {
        reason: "credential_leak_blocked".to_string(),
        request_bytes: 0,
        response_bytes: 0,
    }
}

fn sanitize_runtime_response(
    response: NetworkHttpResponse,
    redaction_values: &[String],
) -> Result<(NetworkHttpResponse, bool), RuntimeHttpEgressError> {
    let NetworkHttpResponse {
        status,
        headers,
        body,
        usage,
    } = response;
    let mut redaction_applied = false;
    let mut sanitized_headers = Vec::new();
    let detector = LeakDetector::new();

    for (name, value) in headers {
        if is_sensitive_runtime_response_header(&name) {
            redaction_applied = true;
            continue;
        }
        let exact_redacted = redact(value, redaction_values);
        if exact_redacted.contains("[REDACTED]") {
            redaction_applied = true;
        }
        let cleaned = detector.scan_and_clean(&exact_redacted).map_err(|_| {
            RuntimeHttpEgressError::Response {
                reason: "response_leak_blocked".to_string(),
                request_bytes: usage.request_bytes,
                response_bytes: usage.response_bytes,
            }
        })?;
        if cleaned != exact_redacted {
            redaction_applied = true;
        }
        sanitized_headers.push((name, cleaned));
    }

    let body_text = String::from_utf8_lossy(&body).into_owned();
    let exact_redacted = redact(body_text, redaction_values);
    let exact_body_redacted = exact_redacted.contains("[REDACTED]");
    if exact_body_redacted {
        redaction_applied = true;
    }
    let cleaned =
        detector
            .scan_and_clean(&exact_redacted)
            .map_err(|_| RuntimeHttpEgressError::Response {
                reason: "response_leak_blocked".to_string(),
                request_bytes: usage.request_bytes,
                response_bytes: usage.response_bytes,
            })?;
    let leak_detector_redacted = cleaned != exact_redacted;
    if leak_detector_redacted {
        redaction_applied = true;
    }
    let body = if exact_body_redacted || leak_detector_redacted {
        cleaned.into_bytes()
    } else {
        body
    };

    Ok((
        NetworkHttpResponse {
            status,
            headers: sanitized_headers,
            body,
            usage,
        },
        redaction_applied,
    ))
}

fn runtime_response(
    response: NetworkHttpResponse,
    redaction_applied: bool,
) -> RuntimeHttpEgressResponse {
    RuntimeHttpEgressResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
        request_bytes: response.usage.request_bytes,
        response_bytes: response.usage.response_bytes,
        redaction_applied,
    }
}

fn redact(mut text: String, values: &[String]) -> String {
    for value in values {
        if !value.is_empty() {
            text = text.replace(value, "[REDACTED]");
        }
    }
    text
}

fn redaction_values_for_secret(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    push_redaction_value(&mut values, value.to_string());
    let encoded = url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    push_redaction_value(&mut values, encoded.clone());
    push_redaction_value(&mut values, lowercase_percent_escapes(&encoded));
    let plus_encoded = encoded.replace("%20", "+");
    push_redaction_value(&mut values, plus_encoded.clone());
    push_redaction_value(&mut values, lowercase_percent_escapes(&plus_encoded));
    values
}

fn push_redaction_value(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn lowercase_percent_escapes(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            output.push('%');
            output.push((bytes[index + 1] as char).to_ascii_lowercase());
            output.push((bytes[index + 2] as char).to_ascii_lowercase());
            index += 3;
            continue;
        }
        output.push(bytes[index] as char);
        index += 1;
    }
    output
}
