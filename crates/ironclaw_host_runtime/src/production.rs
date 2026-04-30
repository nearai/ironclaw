//! Production composition of the [`HostRuntime`] contract.
//!
//! [`DefaultHostRuntime`] is the contract-level facade that upper turn/loop
//! services should depend on. Internally it composes
//! [`ironclaw_capabilities::CapabilityHost`] with neutral kernel services —
//! extension registry, capability dispatcher, trust-aware authorizer,
//! run-state and approval stores, capability-lease store, and process
//! manager.
//!
//! Trust is not evaluated here: callers must supply a host-validated
//! [`TrustDecision`](ironclaw_trust::TrustDecision) on each invocation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::{CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityInvocationError, CapabilityInvocationRequest,
    CapabilityInvocationResult,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatcher, CapabilityId, InvocationId, ResourceScope,
};
use ironclaw_processes::ProcessManager;
use ironclaw_run_state::{ApprovalRequestStore, RunStateError, RunStateStore, RunStatus};

use crate::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate,
    RuntimeBlockedReason, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeStatusRequest,
    RuntimeWorkId, RuntimeWorkSummary, VisibleCapabilityRequest, VisibleCapabilitySurface,
};

/// Default production wiring for [`HostRuntime`].
pub struct DefaultHostRuntime {
    registry: Arc<ExtensionRegistry>,
    dispatcher: Arc<dyn CapabilityDispatcher>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    process_manager: Option<Arc<dyn ProcessManager>>,
    surface_version: CapabilitySurfaceVersion,
}

impl DefaultHostRuntime {
    /// Constructs a default host runtime over the supplied kernel services.
    ///
    /// Callers must additionally attach a run-state store and approval-
    /// request store via [`with_run_state`](Self::with_run_state) and
    /// [`with_approval_requests`](Self::with_approval_requests) before
    /// invoking any capability whose authorizer may return
    /// `RequireApproval`. Without those stores the capability host fails
    /// closed with `ApprovalStoreMissing`, which surfaces here as a
    /// [`RuntimeCapabilityOutcome::Failed`] rather than blocking for human
    /// review.
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        dispatcher: Arc<dyn CapabilityDispatcher>,
        authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
        surface_version: CapabilitySurfaceVersion,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            process_manager: None,
            surface_version,
        }
    }

    /// Attaches the run-state store used to record invocation lifecycle.
    pub fn with_run_state(mut self, run_state: Arc<dyn RunStateStore>) -> Self {
        self.run_state = Some(run_state);
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    pub fn with_approval_requests(
        mut self,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self
    }

    /// Attaches the capability-lease store used by approval resume paths.
    pub fn with_capability_leases(
        mut self,
        capability_leases: Arc<dyn CapabilityLeaseStore>,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    /// Attaches the process manager used by future spawn paths.
    pub fn with_process_manager(mut self, process_manager: Arc<dyn ProcessManager>) -> Self {
        self.process_manager = Some(process_manager);
        self
    }
}

#[async_trait]
impl HostRuntime for DefaultHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        // Forward the (currently advisory) idempotency key into spans for
        // audit/tracing only — dedupe enforcement is not yet implemented at
        // this layer (see `RuntimeCapabilityRequest::idempotency_key`).
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.as_str().to_string());
        if let Some(key) = idempotency_key.as_deref() {
            tracing::debug!(
                capability_id = %capability_id,
                idempotency_key = %key,
                "capability invocation accepted advisory idempotency key (not yet enforced)"
            );
        }

        let mut host = CapabilityHost::new(
            self.registry.as_ref(),
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
        );
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(process_manager) = &self.process_manager {
            host = host.with_process_manager(process_manager.as_ref());
        }

        let invocation = CapabilityInvocationRequest {
            context: request.context,
            capability_id: capability_id.clone(),
            estimate: request.estimate,
            input: request.input,
            trust_decision: request.trust_decision,
        };

        match host.invoke_json(invocation).await {
            Ok(result) => Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                completed_outcome_from(result, capability_id),
            ))),
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    idempotency_key = idempotency_key.as_deref().unwrap_or(""),
                    "capability invocation failed"
                );
                self.translate_invocation_error(error, capability_id, scope, invocation_id)
                    .await
            }
        }
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        let _ = request;
        let descriptors = self.registry.capabilities().cloned().collect();
        Ok(VisibleCapabilitySurface {
            version: self.surface_version.clone(),
            descriptors,
        })
    }

    /// Best-effort cancellation fanout.
    ///
    /// This implementation is a deliberate stub at this layer: the underlying
    /// [`CapabilityHost`] does not yet expose a cancellation port, so we always
    /// return an empty outcome. Upper services must not treat an empty result
    /// as confirmation that work was cancelled — it is `unknown`/`no-op` until
    /// a richer cancellation contract lands.
    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        let _ = request;
        Ok(CancelRuntimeWorkOutcome::default())
    }

    /// Snapshot of active host runtime work for one scope.
    ///
    /// `correlation_id` is carried for tracing/audit only — at this layer we
    /// surface every running invocation in scope rather than narrowing to the
    /// caller's correlation. Upper turn/loop services that need per-correlation
    /// fan-in are expected to filter the returned summaries themselves.
    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        let Some(run_state) = &self.run_state else {
            return Ok(HostRuntimeStatus::default());
        };

        let records = run_state
            .records_for_scope(&request.scope)
            .await
            .map_err(unavailable_from_run_state)?;

        let active_work = records
            .into_iter()
            .filter(|record| record.status == RunStatus::Running)
            .map(|record| {
                let runtime = self
                    .registry
                    .get_capability(&record.capability_id)
                    .map(|descriptor| descriptor.runtime);
                RuntimeWorkSummary {
                    work_id: RuntimeWorkId::Invocation(record.invocation_id),
                    capability_id: Some(record.capability_id),
                    runtime,
                }
            })
            .collect();

        Ok(HostRuntimeStatus { active_work })
    }

    /// Returns a coarse readiness signal for the composed host runtime.
    ///
    /// This is a deliberate stub at this layer: the underlying capability
    /// host does not yet expose backend-level health probes, so we always
    /// report `ready = true` with no missing backends. Upper services must not
    /// rely on this for liveness — richer health surfaces will land alongside
    /// the runtime registry.
    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        Ok(HostRuntimeHealth {
            ready: true,
            missing_runtime_backends: Vec::new(),
        })
    }
}

impl DefaultHostRuntime {
    async fn translate_invocation_error(
        &self,
        error: CapabilityInvocationError,
        capability_id: CapabilityId,
        scope: ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        match error {
            CapabilityInvocationError::AuthorizationRequiresApproval { capability } => {
                match self.lookup_approval_request_id(&scope, invocation_id).await {
                    Ok(Some(approval_request_id)) => Ok(
                        RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate {
                            approval_request_id,
                            capability_id: capability,
                            reason: RuntimeBlockedReason::ApprovalRequired,
                        }),
                    ),
                    Ok(None) => Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                        capability_id: capability,
                        kind: RuntimeFailureKind::Authorization,
                        message: Some(
                            "approval required but no approval request was persisted".to_string(),
                        ),
                    })),
                    Err(host_error) => {
                        // Surface persistence outages as Unavailable rather than
                        // pretending the approval was never persisted; otherwise a
                        // transient run-state failure looks indistinguishable from
                        // the (separately bug-prone) cap-host-skipped-persist path.
                        tracing::warn!(
                            capability_id = %capability,
                            error = %host_error,
                            "approval request lookup failed; surfacing as host runtime unavailability"
                        );
                        Err(host_error)
                    }
                }
            }
            other => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                other,
                capability_id,
            ))),
        }
    }

    async fn lookup_approval_request_id(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ApprovalRequestId>, HostRuntimeError> {
        let Some(run_state) = self.run_state.as_ref() else {
            return Ok(None);
        };
        let record = run_state
            .get(scope, invocation_id)
            .await
            .map_err(unavailable_from_run_state)?;
        Ok(record.and_then(|record| record.approval_request_id))
    }
}

/// Maps a [`RunStateError`] to a sanitized [`HostRuntimeError::Unavailable`].
///
/// `RunStateError::InvalidPath` and `Filesystem` carry raw filesystem
/// strings; `Serialization`/`Deserialization` carry serde internals. Forward
/// the redacted variant discriminator instead of `error.to_string()` so the
/// boundary stays infrastructure-opaque to upper services.
fn unavailable_from_run_state(error: RunStateError) -> HostRuntimeError {
    let reason = match error {
        RunStateError::UnknownInvocation { .. } => "run-state record not found",
        RunStateError::InvocationAlreadyExists { .. } => "run-state record already exists",
        RunStateError::UnknownApprovalRequest { .. } => "approval request not found",
        RunStateError::ApprovalRequestAlreadyExists { .. } => "approval request already exists",
        RunStateError::ApprovalNotPending { .. } => "approval request not pending",
        RunStateError::InvalidPath(_) => "run-state storage path invalid",
        RunStateError::Filesystem(_) => "run-state filesystem unavailable",
        RunStateError::Serialization(_) => "run-state serialization failed",
        RunStateError::Deserialization(_) => "run-state deserialization failed",
    };
    HostRuntimeError::unavailable(reason)
}

fn completed_outcome_from(
    result: CapabilityInvocationResult,
    capability_id: CapabilityId,
) -> RuntimeCapabilityCompleted {
    RuntimeCapabilityCompleted {
        capability_id,
        output: result.dispatch.output,
        usage: result.dispatch.usage,
    }
}

fn failure_from(
    error: CapabilityInvocationError,
    capability_id: CapabilityId,
) -> RuntimeCapabilityFailure {
    let kind = failure_kind_from(&error);
    let message = sanitized_failure_message(&error);
    RuntimeCapabilityFailure {
        capability_id,
        kind,
        message,
    }
}

/// Returns a stable, redacted summary message for a capability invocation
/// failure.
///
/// Variants that wrap inner errors (`Lease`, `RunState`, `Process`,
/// `InvocationFingerprint`) or that surface free-form storage/runtime
/// strings are mapped to fixed, infrastructure-opaque labels. Variants whose
/// `Display` impl is itself stable (capability id + enum discriminator) flow
/// through unchanged.
fn sanitized_failure_message(error: &CapabilityInvocationError) -> Option<String> {
    use CapabilityInvocationError::*;
    match error {
        UnknownCapability { .. }
        | AuthorizationDenied { .. }
        | UnsupportedObligations { .. }
        | AuthorizationRequiresApproval { .. }
        | ApprovalRequestMismatch { .. }
        | ApprovalFingerprintMismatch { .. }
        | ApprovalNotApproved { .. }
        | ApprovalLeaseMissing { .. }
        | ApprovalStoreMissing { .. }
        | ResumeStoreMissing { .. }
        | ProcessManagerMissing { .. }
        | ResumeNotBlocked { .. }
        | ResumeContextMismatch { .. }
        | Dispatch { .. } => Some(error.to_string()),
        InvocationFingerprint { .. } => Some("invocation fingerprint failed".to_string()),
        Lease(_) => Some("capability lease store unavailable".to_string()),
        RunState(_) => Some("run-state store unavailable".to_string()),
        Process(_) => Some("process manager unavailable".to_string()),
    }
}

pub(crate) fn failure_kind_from(error: &CapabilityInvocationError) -> RuntimeFailureKind {
    match error {
        CapabilityInvocationError::UnknownCapability { .. } => RuntimeFailureKind::MissingRuntime,
        CapabilityInvocationError::AuthorizationDenied { .. }
        | CapabilityInvocationError::UnsupportedObligations { .. }
        | CapabilityInvocationError::AuthorizationRequiresApproval { .. }
        | CapabilityInvocationError::ApprovalRequestMismatch { .. }
        | CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
        | CapabilityInvocationError::ApprovalNotApproved { .. }
        | CapabilityInvocationError::ApprovalLeaseMissing { .. }
        | CapabilityInvocationError::ResumeNotBlocked { .. }
        | CapabilityInvocationError::ResumeContextMismatch { .. } => {
            RuntimeFailureKind::Authorization
        }
        CapabilityInvocationError::InvocationFingerprint { .. } => RuntimeFailureKind::InvalidInput,
        CapabilityInvocationError::ApprovalStoreMissing { .. }
        | CapabilityInvocationError::ResumeStoreMissing { .. }
        | CapabilityInvocationError::ProcessManagerMissing { .. } => RuntimeFailureKind::Backend,
        CapabilityInvocationError::Lease(_)
        | CapabilityInvocationError::RunState(_)
        | CapabilityInvocationError::Process(_) => RuntimeFailureKind::Backend,
        CapabilityInvocationError::Dispatch { kind } => dispatch_kind_to_failure(kind),
    }
}

fn dispatch_kind_to_failure(kind: &str) -> RuntimeFailureKind {
    match kind {
        "UnknownCapability"
        | "UnknownProvider"
        | "MissingRuntimeBackend"
        | "UnsupportedRuntime"
        | "ExtensionRuntimeMismatch" => RuntimeFailureKind::MissingRuntime,
        "RuntimeMismatch" => RuntimeFailureKind::Backend,
        "Memory" | "Resource" => RuntimeFailureKind::Resource,
        "NetworkDenied" => RuntimeFailureKind::Network,
        "OutputTooLarge" => RuntimeFailureKind::OutputTooLarge,
        "FilesystemDenied" => RuntimeFailureKind::Authorization,
        "ExitFailure" => RuntimeFailureKind::Process,
        "InputEncode" | "OutputDecode" | "InvalidResult" => RuntimeFailureKind::InvalidInput,
        "Backend"
        | "Client"
        | "Executor"
        | "Guest"
        | "Manifest"
        | "MethodMissing"
        | "UndeclaredCapability"
        | "UnsupportedRunner" => RuntimeFailureKind::Backend,
        _ => RuntimeFailureKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    //! Pinning tests for the host-runtime failure-kind and sanitized-message
    //! mappings.
    //!
    //! The dispatch-kind strings come from
    //! [`ironclaw_host_api::RuntimeDispatchErrorKind::as_str`] and from
    //! [`ironclaw_capabilities::error::dispatch_error_kind`]. Both are
    //! treated as part of the public contract surface; if an upstream rename
    //! drops a string, this module fails closed instead of silently
    //! degrading to [`RuntimeFailureKind::Unknown`].

    use super::*;
    use ironclaw_capabilities::CapabilityInvocationError;
    use ironclaw_host_api::{CapabilityId, RuntimeDispatchErrorKind};

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn dispatch(kind: &str) -> CapabilityInvocationError {
        CapabilityInvocationError::Dispatch {
            kind: kind.to_string(),
        }
    }

    #[test]
    fn dispatch_kind_to_failure_pins_every_runtime_dispatch_error_kind() {
        // Every RuntimeDispatchErrorKind variant must map to a non-Unknown
        // failure kind so upstream additions are surfaced explicitly.
        let cases: &[(RuntimeDispatchErrorKind, RuntimeFailureKind)] = &[
            (
                RuntimeDispatchErrorKind::Backend,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Client,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Executor,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::ExitFailure,
                RuntimeFailureKind::Process,
            ),
            (
                RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
                RuntimeFailureKind::MissingRuntime,
            ),
            (
                RuntimeDispatchErrorKind::FilesystemDenied,
                RuntimeFailureKind::Authorization,
            ),
            (RuntimeDispatchErrorKind::Guest, RuntimeFailureKind::Backend),
            (
                RuntimeDispatchErrorKind::InputEncode,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::InvalidResult,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::Manifest,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Memory,
                RuntimeFailureKind::Resource,
            ),
            (
                RuntimeDispatchErrorKind::MethodMissing,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::NetworkDenied,
                RuntimeFailureKind::Network,
            ),
            (
                RuntimeDispatchErrorKind::OutputDecode,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::OutputTooLarge,
                RuntimeFailureKind::OutputTooLarge,
            ),
            (
                RuntimeDispatchErrorKind::Resource,
                RuntimeFailureKind::Resource,
            ),
            (
                RuntimeDispatchErrorKind::UndeclaredCapability,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::UnsupportedRunner,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Unknown,
                RuntimeFailureKind::Unknown,
            ),
        ];
        for (variant, expected) in cases {
            let kind = variant.as_str();
            let actual = dispatch_kind_to_failure(kind);
            assert_eq!(
                actual, *expected,
                "dispatch kind {kind:?} should map to {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn dispatch_kind_to_failure_pins_dispatch_error_top_level_strings() {
        // These strings come from `dispatch_error_kind` for non-runtime
        // DispatchError variants (UnknownCapability, UnknownProvider, ...).
        assert_eq!(
            dispatch_kind_to_failure("UnknownCapability"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("UnknownProvider"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("MissingRuntimeBackend"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("UnsupportedRuntime"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("RuntimeMismatch"),
            RuntimeFailureKind::Backend
        );
    }

    #[test]
    fn dispatch_kind_to_failure_unknown_strings_fall_back_to_unknown() {
        assert_eq!(
            dispatch_kind_to_failure("some_future_kind_name"),
            RuntimeFailureKind::Unknown
        );
        assert_eq!(dispatch_kind_to_failure(""), RuntimeFailureKind::Unknown);
    }

    #[test]
    fn failure_kind_from_dispatch_unknown_capability_maps_to_missing_runtime() {
        let error = dispatch("UnknownCapability");
        assert_eq!(
            failure_kind_from(&error),
            RuntimeFailureKind::MissingRuntime
        );
    }

    #[test]
    fn failure_kind_from_unknown_capability_variant_maps_to_missing_runtime() {
        let error = CapabilityInvocationError::UnknownCapability { capability: cap() };
        assert_eq!(
            failure_kind_from(&error),
            RuntimeFailureKind::MissingRuntime
        );
    }

    #[test]
    fn sanitized_failure_message_redacts_dispatch_kind_to_stable_form() {
        let error = dispatch("NetworkDenied");
        let message = sanitized_failure_message(&error).expect("dispatch produces a message");
        // Stable form: relies only on the redacted kind token, never on raw
        // backend strings.
        assert!(
            message.contains("NetworkDenied"),
            "sanitized dispatch message should expose the redacted kind, got {message:?}"
        );
    }

    #[test]
    fn runtime_failure_kind_as_str_is_stable_snake_case() {
        // Pin the public metric/tracing tokens; renaming any of these is a
        // breaking observability contract change.
        assert_eq!(RuntimeFailureKind::Authorization.as_str(), "authorization");
        assert_eq!(RuntimeFailureKind::Backend.as_str(), "backend");
        assert_eq!(RuntimeFailureKind::Cancelled.as_str(), "cancelled");
        assert_eq!(RuntimeFailureKind::Dispatcher.as_str(), "dispatcher");
        assert_eq!(RuntimeFailureKind::InvalidInput.as_str(), "invalid_input");
        assert_eq!(
            RuntimeFailureKind::MissingRuntime.as_str(),
            "missing_runtime"
        );
        assert_eq!(RuntimeFailureKind::Network.as_str(), "network");
        assert_eq!(
            RuntimeFailureKind::OutputTooLarge.as_str(),
            "output_too_large"
        );
        assert_eq!(RuntimeFailureKind::Process.as_str(), "process");
        assert_eq!(RuntimeFailureKind::Resource.as_str(), "resource");
        assert_eq!(RuntimeFailureKind::Unknown.as_str(), "unknown");
    }

    #[test]
    fn unavailable_from_run_state_uses_redacted_reasons() {
        let error = RunStateError::InvalidPath("/private/users/secret/database.sqlite".to_string());
        let host_error = unavailable_from_run_state(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/private/"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "run-state storage path invalid");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }

        let error = RunStateError::Filesystem("connection refused at /tmp/runstate.db".to_string());
        let host_error = unavailable_from_run_state(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/tmp"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "run-state filesystem unavailable");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }
}
