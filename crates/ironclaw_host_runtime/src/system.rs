//! Host-owned System capability adapter contracts.
//!
//! System capabilities are kernel/control-plane only. They are registered by
//! host composition, invoked through the internal [`SystemHost`] facade with a
//! host-minted [`SystemInvocationAuthority`], and still execute through the
//! normal `CapabilityHost -> RuntimeDispatcher` path.

use std::{collections::HashMap, fmt, panic::AssertUnwindSafe, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::FutureExt;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_capabilities::{
    CapabilityHost, CapabilityInvocationError, CapabilityInvocationRequest,
};
use ironclaw_events::AuditSink;
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage, CapabilityGrant,
    CapabilityGrantId, CapabilityId, Decision, DecisionSummary, DenyReason, DispatchError,
    EffectKind, ExecutionContext, GrantConstraints, MountView, NetworkPolicy, PackageSource,
    Principal, ResourceEstimate, ResourceReservationId, ResourceScope, ResourceUsage,
    RuntimeDispatchErrorKind, RuntimeKind, SystemServiceId, TrustClass,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::RunStateStore;
use ironclaw_trust::{TrustDecision, TrustError, TrustPolicy};
use serde_json::Value;

use crate::{
    BuiltinObligationHandler, HostRuntimeError, RuntimeCapabilityCompleted,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome, RuntimeFailureKind,
};
use ironclaw_host_api::CapabilityDispatcher;

tokio::task_local! {
    static SYSTEM_AUTHORITY: SystemInvocationAuthority;
}

/// Host-minted idempotency/correlation id for one System operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SystemOperationId(String);

impl SystemOperationId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostRuntimeError> {
        let value = validate_system_token(value.into(), "system operation id", 128)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SystemOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque host-minted authority for one System invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInvocationAuthority {
    issuer: SystemServiceId,
    reason: String,
    operation_id: SystemOperationId,
    resource_scope: ResourceScope,
    _sealed: (),
}

impl SystemInvocationAuthority {
    pub fn host_minted(
        issuer: SystemServiceId,
        reason: impl Into<String>,
        operation_id: SystemOperationId,
        resource_scope: ResourceScope,
    ) -> Result<Self, HostRuntimeError> {
        Ok(Self {
            issuer,
            reason: validate_system_token(reason.into(), "system authority reason", 128)?,
            operation_id,
            resource_scope,
            _sealed: (),
        })
    }

    pub fn issuer(&self) -> &SystemServiceId {
        &self.issuer
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn operation_id(&self) -> &SystemOperationId {
        &self.operation_id
    }

    pub fn resource_scope(&self) -> &ResourceScope {
        &self.resource_scope
    }
}

#[async_trait]
pub trait SystemInvocationAuthorityVerifier: Send + Sync {
    async fn verify_system_authority(
        &self,
        authority: &SystemInvocationAuthority,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<(), HostRuntimeError>;
}

/// System handler input after sealed authority verification and CapabilityHost authorization.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SystemCapabilityRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub input: Value,
    pub authority: SystemInvocationAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemCapabilityResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

impl SystemCapabilityResult {
    pub fn new(output: Value, usage: ResourceUsage) -> Self {
        Self { output, usage }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("system capability dispatch failed: {kind}")]
pub struct SystemCapabilityError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl SystemCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    pub fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

#[async_trait]
pub trait SystemCapabilityHandler: Send + Sync {
    async fn dispatch(
        &self,
        request: SystemCapabilityRequest,
    ) -> Result<SystemCapabilityResult, SystemCapabilityError>;
}

#[derive(Clone, Default)]
pub struct SystemCapabilityRegistry {
    handlers: HashMap<CapabilityId, Arc<dyn SystemCapabilityHandler>>,
}

impl SystemCapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handler<T>(mut self, capability_id: CapabilityId, handler: Arc<T>) -> Self
    where
        T: SystemCapabilityHandler + 'static,
    {
        self.insert_handler(capability_id, handler);
        self
    }

    pub fn insert_handler<T>(&mut self, capability_id: CapabilityId, handler: Arc<T>)
    where
        T: SystemCapabilityHandler + 'static,
    {
        let handler: Arc<dyn SystemCapabilityHandler> = handler;
        self.handlers.insert(capability_id, handler);
    }

    pub fn get(&self, capability_id: &CapabilityId) -> Option<Arc<dyn SystemCapabilityHandler>> {
        self.handlers.get(capability_id).cloned()
    }

    pub fn contains_handler(&self, capability_id: &CapabilityId) -> bool {
        self.handlers.contains_key(capability_id)
    }
}

#[derive(Debug, Clone)]
pub struct SystemCapabilityInvocationRequest {
    pub authority: SystemInvocationAuthority,
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

impl SystemCapabilityInvocationRequest {
    pub fn new(
        authority: SystemInvocationAuthority,
        context: ExecutionContext,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: Value,
    ) -> Self {
        Self {
            authority,
            context,
            capability_id,
            estimate,
            input,
        }
    }
}

pub struct SystemHost {
    registry: Arc<ExtensionRegistry>,
    dispatcher: Arc<dyn CapabilityDispatcher>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    trust_policy: Arc<dyn TrustPolicy>,
    authority_verifier: Arc<dyn SystemInvocationAuthorityVerifier>,
    audit_sink: Arc<dyn AuditSink>,
    run_state: Option<Arc<dyn RunStateStore>>,
    obligation_handler: Arc<BuiltinObligationHandler>,
}

impl SystemHost {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        registry: Arc<ExtensionRegistry>,
        dispatcher: Arc<dyn CapabilityDispatcher>,
        authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
        trust_policy: Arc<dyn TrustPolicy>,
        authority_verifier: Arc<dyn SystemInvocationAuthorityVerifier>,
        audit_sink: Arc<dyn AuditSink>,
        run_state: Option<Arc<dyn RunStateStore>>,
        obligation_handler: Arc<BuiltinObligationHandler>,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            trust_policy,
            authority_verifier,
            audit_sink,
            run_state,
            obligation_handler,
        }
    }

    pub async fn invoke_system_capability(
        &self,
        request: SystemCapabilityInvocationRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let SystemCapabilityInvocationRequest {
            authority,
            mut context,
            capability_id,
            estimate,
            input,
        } = request;
        let scope = context.resource_scope.clone();

        if authority.resource_scope() != &scope {
            self.emit_system_audit(
                &authority,
                &scope,
                &capability_id,
                false,
                None,
                Some("scope_mismatch"),
            )
            .await?;
            return Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id,
                kind: RuntimeFailureKind::Authorization,
                message: Some("system authority scope mismatch".to_string()),
            }));
        }

        if let Err(error) = self
            .authority_verifier
            .verify_system_authority(&authority, &scope, &capability_id)
            .await
        {
            self.emit_system_audit(
                &authority,
                &scope,
                &capability_id,
                false,
                None,
                Some("authority_denied"),
            )
            .await?;
            tracing::debug!(capability_id = %capability_id, error = %error, "system authority verification failed");
            return Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id,
                kind: RuntimeFailureKind::Authorization,
                message: Some("system authority denied".to_string()),
            }));
        }

        let Some(descriptor) = self.registry.get_capability(&capability_id) else {
            self.emit_system_audit(
                &authority,
                &scope,
                &capability_id,
                false,
                None,
                Some("unknown_capability"),
            )
            .await?;
            return Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id,
                kind: RuntimeFailureKind::MissingRuntime,
                message: Some("system capability is not declared".to_string()),
            }));
        };
        if descriptor.runtime != RuntimeKind::System {
            self.emit_system_audit(
                &authority,
                &scope,
                &capability_id,
                false,
                None,
                Some("runtime_mismatch"),
            )
            .await?;
            return Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id,
                kind: RuntimeFailureKind::Authorization,
                message: Some("system host can only invoke system capabilities".to_string()),
            }));
        }

        let trust_decision = match evaluate_system_trust(
            self.registry.as_ref(),
            self.trust_policy.as_ref(),
            &capability_id,
        ) {
            Ok(decision) if decision.effective_trust.class() == TrustClass::System => decision,
            Ok(_) | Err(_) => {
                self.emit_system_audit(
                    &authority,
                    &scope,
                    &capability_id,
                    false,
                    None,
                    Some("trust_denied"),
                )
                .await?;
                return Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                    capability_id,
                    kind: RuntimeFailureKind::Authorization,
                    message: Some("system trust policy denied capability".to_string()),
                }));
            }
        };
        context.trust = TrustClass::System;
        context.runtime = RuntimeKind::System;
        context.grants.grants.push(system_dispatch_grant(
            &capability_id,
            &context,
            &descriptor.effects,
        ));

        let host = self.capability_host();
        let invocation = CapabilityInvocationRequest {
            context,
            capability_id: capability_id.clone(),
            estimate,
            input,
            trust_decision,
        };

        let outcome = match SYSTEM_AUTHORITY
            .scope(authority.clone(), host.invoke_json(invocation))
            .await
        {
            Ok(result) => {
                RuntimeCapabilityOutcome::Completed(Box::new(RuntimeCapabilityCompleted {
                    capability_id: capability_id.clone(),
                    output: result.dispatch.output,
                    usage: result.dispatch.usage,
                }))
            }
            Err(CapabilityInvocationError::AuthorizationRequiresApproval { .. }) => {
                RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                    capability_id: capability_id.clone(),
                    kind: RuntimeFailureKind::Authorization,
                    message: Some(
                        "system invocation cannot request interactive approval".to_string(),
                    ),
                })
            }
            Err(error) => {
                RuntimeCapabilityOutcome::Failed(system_failure_from(error, capability_id.clone()))
            }
        };

        let (success, usage, status) = match &outcome {
            RuntimeCapabilityOutcome::Completed(completed) => {
                (true, Some(&completed.usage), Some("completed"))
            }
            RuntimeCapabilityOutcome::Failed(failure) => (false, None, Some(failure.kind.as_str())),
            RuntimeCapabilityOutcome::ApprovalRequired(_) => {
                (false, None, Some("approval_required"))
            }
            RuntimeCapabilityOutcome::AuthRequired(_) => (false, None, Some("auth_required")),
            RuntimeCapabilityOutcome::ResourceBlocked(_) => (false, None, Some("resource_blocked")),
            RuntimeCapabilityOutcome::SpawnedProcess(_) => (false, None, Some("spawned_process")),
        };
        self.emit_system_audit(&authority, &scope, &capability_id, success, usage, status)
            .await?;
        Ok(outcome)
    }

    fn capability_host(&self) -> CapabilityHost<'_, dyn CapabilityDispatcher> {
        let mut host = CapabilityHost::new(
            self.registry.as_ref(),
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
        )
        .with_obligation_handler(self.obligation_handler.as_ref());
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        host
    }

    async fn emit_system_audit(
        &self,
        authority: &SystemInvocationAuthority,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        success: bool,
        usage: Option<&ResourceUsage>,
        status: Option<&str>,
    ) -> Result<(), HostRuntimeError> {
        let record = AuditEnvelope {
            event_id: AuditEventId::new(),
            correlation_id: ironclaw_host_api::CorrelationId::new(),
            stage: AuditStage::After,
            timestamp: Utc::now(),
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            invocation_id: scope.invocation_id,
            process_id: None,
            approval_request_id: None,
            extension_id: None,
            action: ActionSummary {
                kind: "system_capability".to_string(),
                target: Some(format!(
                    "{}:{}",
                    capability_id.as_str(),
                    authority.operation_id()
                )),
                effects: vec![EffectKind::DispatchCapability],
            },
            decision: DecisionSummary {
                kind: format!("system_authorized:{}", authority.reason()),
                reason: None,
                actor: Some(Principal::System(authority.issuer().clone())),
            },
            result: Some(ActionResultSummary {
                success,
                status: status.map(str::to_string),
                output_bytes: usage.map(|usage| usage.output_bytes),
            }),
        };
        self.audit_sink
            .emit_audit(record)
            .await
            .map_err(|_| HostRuntimeError::unavailable("system audit sink failed"))
    }
}

struct NoApprovalAuthorizer {
    inner: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
}

impl NoApprovalAuthorizer {
    fn new(inner: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for NoApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &ironclaw_host_api::CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        match self
            .inner
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await
        {
            Decision::RequireApproval { .. } => Decision::Deny {
                reason: DenyReason::ApprovalDenied,
            },
            decision => decision,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &ironclaw_host_api::CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        match self
            .inner
            .authorize_spawn_with_trust(context, descriptor, estimate, trust_decision)
            .await
        {
            Decision::RequireApproval { .. } => Decision::Deny {
                reason: DenyReason::ApprovalDenied,
            },
            decision => decision,
        }
    }
}

pub(crate) fn no_approval_authorizer(
    inner: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    Arc::new(NoApprovalAuthorizer::new(inner))
}

#[derive(Clone)]
pub(crate) struct SystemRuntimeAdapter {
    registry: Arc<SystemCapabilityRegistry>,
}

impl SystemRuntimeAdapter {
    pub(crate) fn from_registry(registry: Arc<SystemCapabilityRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl<F, G> ironclaw_dispatcher::RuntimeAdapter<F, G> for SystemRuntimeAdapter
where
    F: ironclaw_filesystem::RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: ironclaw_dispatcher::RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<ironclaw_dispatcher::RuntimeAdapterResult, DispatchError> {
        let Some(handler) = self.registry.get(request.capability_id) else {
            if let Some(reservation) = request.resource_reservation {
                let _ = request.governor.release(reservation.id);
            }
            return Err(DispatchError::System {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
            });
        };

        let authority = match current_system_authority() {
            Ok(authority) => authority,
            Err(error) => {
                if let Some(reservation) = &request.resource_reservation {
                    release_system_reservation(request.governor, reservation.id);
                }
                return Err(error);
            }
        };

        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope.clone(), request.estimate.clone())
                .map_err(|_| DispatchError::System {
                    kind: RuntimeDispatchErrorKind::Resource,
                })?,
        };
        let result = match AssertUnwindSafe(handler.dispatch(SystemCapabilityRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            estimate: request.estimate,
            mounts: request.mounts,
            input: request.input,
            authority,
        }))
        .catch_unwind()
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                account_or_release_failed_system_execution(
                    request.governor,
                    reservation.id,
                    error.usage(),
                )?;
                return Err(DispatchError::System { kind: error.kind() });
            }
            Err(_) => {
                release_system_reservation(request.governor, reservation.id);
                return Err(DispatchError::System {
                    kind: RuntimeDispatchErrorKind::Backend,
                });
            }
        };

        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| DispatchError::System {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            })?;
        let mut usage = result.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        let receipt = match request.governor.reconcile(reservation.id, usage.clone()) {
            Ok(receipt) => receipt,
            Err(_) => {
                release_system_reservation(request.governor, reservation.id);
                return Err(DispatchError::System {
                    kind: RuntimeDispatchErrorKind::Resource,
                });
            }
        };
        Ok(ironclaw_dispatcher::RuntimeAdapterResult {
            output: result.output,
            usage,
            receipt,
            output_bytes,
        })
    }
}

fn system_dispatch_grant(
    capability_id: &CapabilityId,
    context: &ExecutionContext,
    effects: &[EffectKind],
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id.clone(),
        grantee: Principal::Extension(context.extension_id.clone()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: effects.to_vec(),
            mounts: context.mounts.clone(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn evaluate_system_trust(
    registry: &ExtensionRegistry,
    trust_policy: &dyn TrustPolicy,
    capability_id: &CapabilityId,
) -> Result<TrustDecision, HostRuntimeError> {
    let descriptor = registry
        .get_capability(capability_id)
        .ok_or_else(|| HostRuntimeError::invalid_request("unknown system capability"))?;
    let package = registry
        .get_extension(&descriptor.provider)
        .ok_or_else(|| HostRuntimeError::invalid_request("missing system package"))?;
    let input = trust_policy_input_for_local_manifest(package)?;
    trust_policy.evaluate(&input).map_err(|error| match error {
        TrustError::InvariantViolation { .. } => {
            HostRuntimeError::unavailable("system trust policy evaluation failed")
        }
    })
}

fn trust_policy_input_for_local_manifest(
    package: &ExtensionPackage,
) -> Result<ironclaw_trust::TrustPolicyInput, HostRuntimeError> {
    package
        .trust_policy_input(local_manifest_source(package), None, None)
        .map_err(|_| HostRuntimeError::invalid_request("invalid system trust metadata"))
}

fn local_manifest_source(package: &ExtensionPackage) -> PackageSource {
    PackageSource::LocalManifest {
        path: format!(
            "{}/manifest.toml",
            package.root.as_str().trim_end_matches('/')
        ),
    }
}

fn account_or_release_failed_system_execution<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    usage: Option<&ResourceUsage>,
) -> Result<(), DispatchError>
where
    G: ResourceGovernor + ?Sized,
{
    let Some(usage) = usage else {
        release_system_reservation(governor, reservation_id);
        return Ok(());
    };
    if !has_accountable_effects(usage) {
        release_system_reservation(governor, reservation_id);
        return Ok(());
    }
    if governor.reconcile(reservation_id, usage.clone()).is_err() {
        release_system_reservation(governor, reservation_id);
        return Err(DispatchError::System {
            kind: RuntimeDispatchErrorKind::Resource,
        });
    }
    Ok(())
}

fn release_system_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
}

fn current_system_authority() -> Result<SystemInvocationAuthority, DispatchError> {
    SYSTEM_AUTHORITY
        .try_with(Clone::clone)
        .map_err(|_| DispatchError::System {
            kind: RuntimeDispatchErrorKind::Backend,
        })
}

fn has_accountable_effects(usage: &ResourceUsage) -> bool {
    usage.usd != Default::default()
        || usage.input_tokens > 0
        || usage.output_tokens > 0
        || usage.wall_clock_ms > 0
        || usage.output_bytes > 0
        || usage.network_egress_bytes > 0
        || usage.process_count > 0
}

fn system_failure_from(
    error: CapabilityInvocationError,
    capability_id: CapabilityId,
) -> RuntimeCapabilityFailure {
    let kind = match &error {
        CapabilityInvocationError::UnknownCapability { .. } => RuntimeFailureKind::MissingRuntime,
        CapabilityInvocationError::AuthorizationDenied { .. } => RuntimeFailureKind::Authorization,
        CapabilityInvocationError::Dispatch { kind } => dispatch_kind_to_failure(kind),
        CapabilityInvocationError::InvocationFingerprint { .. } => RuntimeFailureKind::InvalidInput,
        CapabilityInvocationError::Lease(_)
        | CapabilityInvocationError::RunState(_)
        | CapabilityInvocationError::Process(_)
        | CapabilityInvocationError::ApprovalStoreMissing { .. }
        | CapabilityInvocationError::ResumeStoreMissing { .. }
        | CapabilityInvocationError::ProcessManagerMissing { .. } => RuntimeFailureKind::Backend,
        _ => RuntimeFailureKind::Authorization,
    };
    RuntimeCapabilityFailure {
        capability_id,
        kind,
        message: Some(error.to_string()),
    }
}

fn dispatch_kind_to_failure(kind: &str) -> RuntimeFailureKind {
    match kind {
        "UndeclaredCapability" | "UnsupportedRunner" | "Backend" => RuntimeFailureKind::Backend,
        "Resource" | "Memory" => RuntimeFailureKind::Resource,
        "OutputDecode" | "InputEncode" | "InvalidResult" => RuntimeFailureKind::InvalidInput,
        "OutputTooLarge" => RuntimeFailureKind::OutputTooLarge,
        "NetworkDenied" => RuntimeFailureKind::Network,
        "UnknownCapability"
        | "UnknownProvider"
        | "MissingRuntimeBackend"
        | "UnsupportedRuntime" => RuntimeFailureKind::MissingRuntime,
        _ => RuntimeFailureKind::Unknown,
    }
}

fn validate_system_token(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, HostRuntimeError> {
    if value.is_empty() || value.len() > max_bytes {
        return Err(HostRuntimeError::invalid_request(format!(
            "{label} must be non-empty and at most {max_bytes} bytes"
        )));
    }
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
    {
        return Err(HostRuntimeError::invalid_request(format!(
            "{label} must use only ascii letters, digits, dash, underscore, or dot"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_dispatcher::{RuntimeAdapter, RuntimeAdapterRequest};
    use ironclaw_extensions::{CapabilityManifest, ExtensionManifest, ExtensionRuntime};
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{
        CapabilitySet, ExtensionId, PermissionMode, RequestedTrustClass, UserId, VirtualPath,
    };
    use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
    use serde_json::json;

    struct ShouldNotRunSystemHandler;

    #[async_trait]
    impl SystemCapabilityHandler for ShouldNotRunSystemHandler {
        async fn dispatch(
            &self,
            _request: SystemCapabilityRequest,
        ) -> Result<SystemCapabilityResult, SystemCapabilityError> {
            panic!("direct adapter path must fail before handler dispatch")
        }
    }

    #[tokio::test]
    async fn direct_adapter_without_system_authority_releases_prepared_reservation() {
        let extension_registry = system_extension_registry();
        let capability_id = system_capability_id();
        let package = extension_registry
            .get_extension(&system_provider_id())
            .unwrap();
        let descriptor = extension_registry.get_capability(&capability_id).unwrap();
        let adapter = SystemRuntimeAdapter::from_registry(Arc::new(
            SystemCapabilityRegistry::new()
                .with_handler(capability_id.clone(), Arc::new(ShouldNotRunSystemHandler)),
        ));
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let context = ExecutionContext::local_default(
            UserId::new("user").unwrap(),
            system_provider_id(),
            RuntimeKind::System,
            TrustClass::System,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap();
        let scope = context.resource_scope;
        let account = ResourceAccount::tenant(scope.tenant_id.clone());
        let estimate = ResourceEstimate {
            network_egress_bytes: Some(100),
            ..ResourceEstimate::default()
        };
        let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
        assert_ne!(governor.reserved_for(&account), Default::default());

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package,
                descriptor,
                filesystem: &filesystem,
                governor: &governor,
                capability_id: &capability_id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: Some(reservation),
                input: json!({"repair":"state"}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::System {
                kind: RuntimeDispatchErrorKind::Backend
            })
        ));
        assert_eq!(governor.reserved_for(&account), Default::default());
    }

    fn system_extension_registry() -> ExtensionRegistry {
        let package = ExtensionPackage::from_manifest(
            ExtensionManifest {
                id: system_provider_id(),
                name: "System".to_string(),
                version: "0.1.0".to_string(),
                description: "Host-owned system capabilities".to_string(),
                requested_trust: RequestedTrustClass::SystemRequested,
                trust: TrustClass::Sandbox,
                runtime: ExtensionRuntime::System {
                    service: "kernel".to_string(),
                },
                capabilities: vec![CapabilityManifest {
                    id: system_capability_id(),
                    description: "Repairs host-owned state".to_string(),
                    effects: vec![EffectKind::DispatchCapability],
                    default_permission: PermissionMode::Allow,
                    parameters_schema: json!({"type":"object"}),
                    resource_profile: None,
                }],
            },
            VirtualPath::new("/system/extensions/system").unwrap(),
        )
        .unwrap();
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).unwrap();
        registry
    }

    fn system_provider_id() -> ExtensionId {
        ExtensionId::new("system").unwrap()
    }

    fn system_capability_id() -> CapabilityId {
        CapabilityId::new("system.repair").unwrap()
    }
}
