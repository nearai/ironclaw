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
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::AuditBefore]);
    let handler = RecordingObligationHandler::default();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "handled"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.has_request());
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
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let handler = RecordingObligationHandler::default();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not dispatch"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!dispatcher.has_request());
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
    let dispatcher = RecordingDispatcher::default();
    let mut context = execution_context(CapabilitySet::default());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
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
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    host.invoke_json(CapabilityInvocationRequest {
        context,
        capability_id: capability_id(),
        estimate: estimate.clone(),
        input: json!({"message": "prepared effects"}),
        trust_decision: trust_decision(),
    })
    .await
    .unwrap();

    let request = dispatcher.take_request();
    assert_eq!(request.scope, scope);
    assert_eq!(request.estimate, estimate);
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
    let dispatcher = OutputDispatcher::new(json!({"token": "secret-token"}));
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let handler = RedactingObligationHandler;
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "post dispatch"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"token": "[REDACTED]"}));
}

#[tokio::test]
async fn capability_host_aborts_staged_obligations_when_completion_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = OutputDispatcher::new(json!({"oversized": true}));
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
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "completion fails"}),
            trust_decision: trust_decision(),
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
    let dispatcher = RecordingDispatcher::default();
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
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
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
        trust_decision: trust_decision(),
    })
    .await
    .unwrap();

    assert_eq!(process_manager.mounts(), Some(narrowed_mounts));
}

#[tokio::test]
async fn capability_host_aborts_prepared_obligations_when_process_start_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
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
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "spawn fails"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CapabilityInvocationError::Process { .. }));
    assert!(aborted.load(Ordering::SeqCst));
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_rejects_post_output_obligations_for_spawn_before_handler_or_process() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer::new(vec![Obligation::RedactOutput]);
    let observed = Arc::new(AtomicBool::new(false));
    let handler = FlaggingObligationHandler {
        observed: Arc::clone(&observed),
    };
    let process_manager = PanicProcessManager;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_process_manager(&process_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context: execution_context(CapabilitySet::default()),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not spawn"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!observed.load(Ordering::SeqCst));
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

struct OutputDispatcher {
    output: serde_json::Value,
}

impl OutputDispatcher {
    fn new(output: serde_json::Value) -> Self {
        Self { output }
    }
}

#[async_trait]
impl CapabilityDispatcher for OutputDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: self.output.clone(),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

fn process_record_from_start(start: ProcessStart, status: ProcessStatus) -> ProcessRecord {
    ProcessRecord {
        process_id: start.process_id,
        parent_process_id: start.parent_process_id,
        invocation_id: start.invocation_id,
        scope: start.scope,
        extension_id: start.extension_id,
        capability_id: start.capability_id,
        runtime: start.runtime,
        status,
        grants: start.grants,
        mounts: start.mounts,
        estimated_resources: start.estimated_resources,
        resource_reservation_id: start.resource_reservation_id,
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
