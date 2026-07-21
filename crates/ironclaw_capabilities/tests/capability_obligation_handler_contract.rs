mod support;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_trust::TrustDecision;
use serde_json::json;

use support::*;

#[tokio::test]
async fn capability_host_uses_obligation_handler_before_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::AuditBefore]);
    let handler = RecordingObligationHandler::default();
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "handled"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.call_count() > 0);
    assert_eq!(
        handler.records(),
        vec![ObligationRecord {
            phase: CapabilityObligationPhase::Invoke,
            obligations: vec![Obligation::AuditBefore],
        }]
    );
}

#[tokio::test]
async fn capability_host_still_fails_closed_when_handler_rejects_obligations() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let handler = RecordingObligationHandler::default();
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not dispatch"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(dispatcher.call_count() == 0);
    assert_eq!(handler.records().len(), 1);
}

#[tokio::test]
async fn capability_host_passes_prepared_effects_to_dispatch() {
    let registry = registry_with_echo_capability();
    let reservation_id = ResourceReservationId::new();
    let narrowed_mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_only(),
    );
    let dispatcher = recording_dispatcher();
    let mut context = execution_context(CapabilitySet::default());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    let scope = context.resource_scope.clone();
    let authorizer = ObligatingAuthorizer::new(vec![
        Obligation::UseScopedMounts {
            mounts: narrowed_mounts.clone(),
        },
        Obligation::ReserveResources { reservation_id },
    ]);
    let handler = EffectObligationHandler {
        mounts: Some(narrowed_mounts.clone()),
        reservation: Some(ResourceReservation {
            id: reservation_id,
            scope: scope.clone(),
            estimate: estimate.clone(),
        }),
        aborted: Arc::new(AtomicBool::new(false)),
    };
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    host.invoke_json(CapabilityInvocationRequest {
        context,
        capability_id: capability_id(),
        estimate: estimate.clone(),
        input: json!({"message": "prepared effects"}),
    })
    .await
    .unwrap();

    let request = dispatcher.last_request().unwrap();
    assert_eq!(request.invocation.scope, scope);
    assert_eq!(request.invocation.estimate, estimate);
    assert_eq!(request.mounts, Some(narrowed_mounts));
    assert_eq!(
        request
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id),
        Some(reservation_id)
    );
}

#[tokio::test]
async fn capability_host_completes_post_dispatch_obligations_before_returning() {
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::responding(|request, _| {
        Ok(dispatch_result_with_output(
            request,
            json!({"token": "secret-token"}),
        ))
    });
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let handler = RedactingObligationHandler;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "post dispatch"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"token": "[REDACTED]"}));
}

#[tokio::test]
async fn capability_host_aborts_staged_obligations_when_completion_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::responding(|request, _| {
        Ok(dispatch_result_with_output(
            request,
            json!({"oversized": true}),
        ))
    });
    let aborted_outcome = Arc::new(Mutex::new(None));
    let reservation_id = ResourceReservationId::new();
    let context = execution_context(CapabilitySet::default());
    let estimate = ResourceEstimate::default();
    let handler = FailingCompletionObligationHandler {
        reservation: ResourceReservation {
            id: reservation_id,
            scope: context.resource_scope.clone(),
            estimate: estimate.clone(),
        },
        aborted_outcome: Arc::clone(&aborted_outcome),
    };
    let authorizer = ObligatingAuthorizer::new(vec![
        Obligation::ReserveResources { reservation_id },
        Obligation::RedactOutput,
    ]);
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "completion fails"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ObligationFailed { .. }
    ));
    let aborted = aborted_outcome.lock().unwrap().clone().unwrap();
    assert!(aborted.resource_reservation.is_none());
}

#[tokio::test]
async fn capability_host_passes_prepared_mounts_to_process_start() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let narrowed_mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_only(),
    );
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::UseScopedMounts {
        mounts: narrowed_mounts.clone(),
    }]);
    let handler = EffectObligationHandler {
        mounts: Some(narrowed_mounts.clone()),
        reservation: None,
        aborted: Arc::new(AtomicBool::new(false)),
    };
    let process_manager = MountRecordingProcessManager::default();
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);
    let mut context = execution_context(CapabilitySet::default());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );

    host.spawn_json(CapabilitySpawnRequest {
        context,
        capability_id: capability_id(),
        estimate: ResourceEstimate::default(),
        input: json!({"message": "prepared mount"}),
    })
    .await
    .unwrap();

    assert_eq!(process_manager.mounts(), Some(narrowed_mounts));
}

#[tokio::test]
async fn capability_host_aborts_prepared_obligations_when_process_start_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let context = execution_context(CapabilitySet::default());
    let estimate = ResourceEstimate::default();
    let aborted = Arc::new(AtomicBool::new(false));
    let reservation_id = ResourceReservationId::new();
    let handler = EffectObligationHandler {
        mounts: None,
        reservation: Some(ResourceReservation {
            id: reservation_id,
            scope: context.resource_scope.clone(),
            estimate: estimate.clone(),
        }),
        aborted: Arc::clone(&aborted),
    };
    let authorizer =
        ObligatingAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    let process_manager = FailingProcessManager;
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "spawn fails"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CapabilityInvocationError::Process { .. }));
    assert!(aborted.load(Ordering::SeqCst));
    assert!(dispatcher.call_count() == 0);
}

#[tokio::test]
async fn capability_host_rejects_post_output_obligations_for_spawn_before_handler_or_process() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let observed = Arc::new(AtomicBool::new(false));
    let handler = FlaggingObligationHandler {
        observed: Arc::clone(&observed),
    };
    let process_manager = PanicProcessManager;
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not spawn"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!observed.load(Ordering::SeqCst));
}

// S6 (§5.3.2/§9): dispatch routes through the sealed `Authorized` witness. The
// witness carries the fold's `Option<MountView>` verbatim, so a capability with
// NO scoped-mount obligation dispatches with `mounts: None` — never a collapsed
// empty `MountView`. Proves subtlety (a): `None` is preserved byte-for-byte, so
// the filesystem resolver's `None`-vs-empty distinction is not silently erased.
#[tokio::test]
async fn invoke_dispatch_carries_witness_none_mounts_verbatim_not_a_default() {
    let registry = registry_with_echo_capability();
    let reservation_id = ResourceReservationId::new();
    let dispatcher = RecordingDispatcher::default();
    let context = dispatchable_context();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let authorizer =
        ObligatingAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    let handler = EffectObligationHandler {
        mounts: None,
        reservation: Some(ResourceReservation {
            id: reservation_id,
            scope: scope.clone(),
            estimate: estimate.clone(),
        }),
        aborted: Arc::new(AtomicBool::new(false)),
    };
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    host.invoke_json(CapabilityInvocationRequest {
        context,
        capability_id: capability_id(),
        estimate: estimate.clone(),
        input: json!({"message": "none mounts"}),
    })
    .await
    .unwrap();

    let request = dispatcher.take_request();
    assert_eq!(
        request.mounts, None,
        "a fold with no scoped-mount obligation must dispatch mounts as None, not Some(default)"
    );
    assert_eq!(
        request
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id),
        Some(reservation_id),
        "the sealed reservation drives dispatch"
    );
}

// Spawn neutrality for the `None`-mounts case: when the fold produced no
// scoped-mount obligation the witness carries `None`, and `spawn_json` applies
// the same `unwrap_or_else(context.mounts)` fallback it always did — so the
// process start still receives the context's mounts, byte-for-byte as before.
#[tokio::test]
async fn spawn_process_start_uses_context_mounts_when_witness_mounts_are_none() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let context_mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );
    // Empty obligations → the fold seals a witness with `mounts: None`.
    let authorizer = ObligatingAuthorizer::new(vec![]);
    let process_manager = MountRecordingProcessManager::default();
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_process_manager(&process_manager);
    let mut context = dispatchable_context();
    context.mounts = context_mounts.clone();

    host.spawn_json(CapabilitySpawnRequest {
        context,
        capability_id: capability_id(),
        estimate: ResourceEstimate::default(),
        input: json!({"message": "context mounts fallback"}),
    })
    .await
    .unwrap();

    assert_eq!(
        process_manager.mounts(),
        Some(context_mounts),
        "a None witness mount must fall back to the context mounts, not an empty default"
    );
}

// Expired-witness invoke fails closed and releases its reservation (issue test
// (2)). A persistent grant that already expired freezes the witness deadline in
// the past, so `into_parts(now)` at dispatch time returns the expired witness —
// the only way to exercise the expiry arm, which cannot occur in a synchronous
// authorize→dispatch otherwise. The run fails closed (terminal
// `InternalInvariantViolation`), dispatch never runs, and the prepared
// reservation is released through the obligation lifecycle (`abort`).
#[tokio::test]
async fn invoke_fails_closed_and_releases_reservation_when_witness_expired() {
    let registry = registry_with_echo_capability();
    let reservation_id = ResourceReservationId::new();
    let dispatcher = RecordingDispatcher::default();
    let context = dispatchable_context();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let aborted = Arc::new(AtomicBool::new(false));
    let handler = EffectObligationHandler {
        mounts: None,
        reservation: Some(ResourceReservation {
            id: reservation_id,
            scope: scope.clone(),
            estimate: estimate.clone(),
        }),
        aborted: Arc::clone(&aborted),
    };
    let authorizer =
        ObligatingAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    let policy_facts = PersistentGrantPolicyFacts::new(expired_grant(past_timestamp()));
    let host =
        capability_host_with_policy_facts(&registry, &dispatcher, &authorizer, &policy_facts)
            .with_obligation_handler(&handler);

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "expired witness"}),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::Dispatch {
                kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied),
                ..
            }
        ),
        "an expired witness must fail closed as an expired sealed dispatch authorization, got {err:?}"
    );
    assert!(
        !dispatcher.has_request(),
        "an expired witness must not reach dispatch"
    );
    assert!(
        aborted.load(Ordering::SeqCst),
        "the prepared reservation must be released via the obligation abort path"
    );
}

// Expired-witness spawn fails closed and releases its reservation, mirroring the
// invoke arm (the spawn path has its own, distinct expiry branch).
#[tokio::test]
async fn spawn_fails_closed_and_releases_reservation_when_witness_expired() {
    let registry = registry_with_echo_capability();
    let reservation_id = ResourceReservationId::new();
    let dispatcher = RecordingDispatcher::default();
    let context = dispatchable_context();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let aborted = Arc::new(AtomicBool::new(false));
    let handler = EffectObligationHandler {
        mounts: None,
        reservation: Some(ResourceReservation {
            id: reservation_id,
            scope: scope.clone(),
            estimate: estimate.clone(),
        }),
        aborted: Arc::clone(&aborted),
    };
    let authorizer =
        ObligatingAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    let policy_facts = PersistentGrantPolicyFacts::new(expired_grant(past_timestamp()));
    let process_manager = PanicProcessManager;
    let host =
        capability_host_with_policy_facts(&registry, &dispatcher, &authorizer, &policy_facts)
            .with_obligation_handler(&handler)
            .with_process_manager(&process_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "expired spawn witness"}),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::Dispatch {
                kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied),
                ..
            }
        ),
        "an expired spawn witness must fail closed as an expired sealed dispatch authorization, got {err:?}"
    );
    assert!(
        aborted.load(Ordering::SeqCst),
        "the prepared reservation must be released via the obligation abort path"
    );
}

fn past_timestamp() -> Timestamp {
    chrono::DateTime::from_timestamp(1, 0).unwrap()
}

/// An execution context that seals a witness. S6 routes dispatch through the
/// sealed `Authorized`; the seal mints one only when the context carries a real
/// ingress `origin` (production always stamps one — the loop stamps `LoopRun`),
/// so a context lacking both `origin` and `run_id` takes the neutral
/// obligation-derived fallback with no witness to consume. These S6 tests want
/// the witness path, so they stamp a `Product` origin.
fn dispatchable_context() -> ExecutionContext {
    let mut context = execution_context(CapabilitySet::default());
    context.origin = Some(InvocationOrigin::Product(
        ProductKind::new("settings").unwrap(),
    ));
    context
}

fn expired_grant(expires_at: Timestamp) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: Some(expires_at),
            max_invocations: None,
        },
    }
}

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

#[derive(Default)]
struct RecordingObligationHandler {
    records: Mutex<Vec<ObligationRecord>>,
}

impl RecordingObligationHandler {
    fn records(&self) -> Vec<ObligationRecord> {
        self.records.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObligationRecord {
    phase: CapabilityObligationPhase,
    obligations: Vec<Obligation>,
}

#[async_trait]
impl CapabilityObligationHandler for RecordingObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.records.lock().unwrap().push(ObligationRecord {
            phase: request.phase,
            obligations: request.obligations.to_vec(),
        });
        if request
            .obligations
            .iter()
            .all(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            Ok(())
        } else {
            Err(CapabilityObligationError::Unsupported {
                obligations: request.obligations.to_vec(),
            })
        }
    }
}

struct EffectObligationHandler {
    mounts: Option<MountView>,
    reservation: Option<ResourceReservation>,
    aborted: Arc<AtomicBool>,
}

#[async_trait]
impl CapabilityObligationHandler for EffectObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        Ok(CapabilityObligationOutcome {
            mounts: self.mounts.clone(),
            resource_reservation: self.reservation.clone(),
        })
    }

    async fn abort(
        &self,
        _request: CapabilityObligationAbortRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.aborted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct FailingCompletionObligationHandler {
    reservation: ResourceReservation,
    aborted_outcome: Arc<Mutex<Option<CapabilityObligationOutcome>>>,
}

#[async_trait]
impl CapabilityObligationHandler for FailingCompletionObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        Ok(CapabilityObligationOutcome {
            mounts: None,
            resource_reservation: Some(self.reservation.clone()),
        })
    }

    async fn complete_dispatch(
        &self,
        _request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        Err(CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Output,
        })
    }

    async fn abort(
        &self,
        request: CapabilityObligationAbortRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        *self.aborted_outcome.lock().unwrap() = Some(request.outcome.clone());
        Ok(())
    }
}

struct RedactingObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for RedactingObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        assert_eq!(request.obligations, &[Obligation::RedactOutput]);
        Ok(())
    }

    async fn complete_dispatch(
        &self,
        request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        assert_eq!(request.obligations, &[Obligation::RedactOutput]);
        let mut dispatch = request.dispatch.clone();
        dispatch.output = json!({"token": "[REDACTED]"});
        Ok(dispatch)
    }
}

struct FlaggingObligationHandler {
    observed: Arc<AtomicBool>,
}

#[async_trait]
impl CapabilityObligationHandler for FlaggingObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.observed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct MountRecordingProcessManager {
    mounts: Mutex<Option<MountView>>,
}

impl MountRecordingProcessManager {
    fn mounts(&self) -> Option<MountView> {
        self.mounts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProcessManager for MountRecordingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        *self.mounts.lock().unwrap() = Some(start.mounts.clone());
        Ok(process_record_from_start(start, ProcessStatus::Running))
    }
}

struct FailingProcessManager;

#[async_trait]
impl ProcessManager for FailingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        Err(ProcessError::ProcessAlreadyExists {
            process_id: start.process_id,
        })
    }
}

struct PanicProcessManager;

#[async_trait]
impl ProcessManager for PanicProcessManager {
    async fn spawn(&self, _start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        panic!("process manager must not be called for unsupported post-output spawn obligations")
    }
}

fn process_record_from_start(start: ProcessStart, status: ProcessStatus) -> ProcessRecord {
    ProcessRecord {
        process_id: start.process_id,
        parent_process_id: start.parent_process_id,
        invocation_id: start.invocation_id,
        scope: start.scope,
        authenticated_actor_user_id: start.authenticated_actor_user_id,
        extension_id: start.extension_id,
        capability_id: start.capability_id,
        runtime: start.runtime,
        status,
        grants: start.grants,
        mounts: start.mounts,
        estimated_resources: start.estimated_resources,
        resource_reservation_id: start.resource_reservation_id,
        authorized_continuation: start.authorized_continuation,
        error_kind: None,
    }
}

fn mount_view(alias: &str, target: &str, permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).unwrap(),
        VirtualPath::new(target).unwrap(),
        permissions,
    )])
    .unwrap()
}
