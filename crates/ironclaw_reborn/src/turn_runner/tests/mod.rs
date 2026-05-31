use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use ironclaw_host_api::{TenantId, ThreadId};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, EventCursor, LoopCheckpointKind,
    LoopCompleted, LoopCompletionKind, LoopExit, LoopExitId, LoopExitMapping, LoopMessageRef,
    ReplyTargetBindingRef, RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnError,
    TurnId, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, CheckpointSchemaId, LoopDriverId,
        LoopModelRouteSnapshot,
    },
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        TurnRunnerOutcome,
    },
};

use crate::{
    driver_registry::{DriverKind, DriverRegistry, DriverRequirements},
    loop_exit_applier::{InMemoryLoopExitEvidencePort, LoopExitApplier},
};

use super::*;

// ─── Test helpers ───────────────────────────────────────────────────────────

fn test_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("test-tenant").expect("valid"),
        None,
        None,
        ThreadId::new("test-thread").expect("valid"),
    )
}

fn test_descriptor() -> AgentLoopDriverDescriptor {
    AgentLoopDriverDescriptor {
        id: LoopDriverId::new("test_loop").expect("valid"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(CheckpointSchemaId::new("test_checkpoint").expect("valid")),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    }
}

fn test_resolved_profile_with_driver(
    desc: &AgentLoopDriverDescriptor,
) -> ironclaw_turns::ResolvedRunProfile {
    use ironclaw_turns::run_profile::*;
    use ironclaw_turns::*;

    ResolvedRunProfile {
        run_class_id: RunClassId::new("test_class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: desc.clone(),
        checkpoint_schema_id: desc
            .checkpoint_schema_id
            .clone()
            .unwrap_or_else(|| CheckpointSchemaId::new("fallback_checkpoint").expect("valid")),
        checkpoint_schema_version: desc
            .checkpoint_schema_version
            .unwrap_or_else(|| RunProfileVersion::new(1)),
        model_profile_id: ModelProfileId::new("test_model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new("test_capabilities")
            .expect("valid"),
        context_profile_id: ContextProfileId::new("test_context").expect("valid"),
        steering_policy: SteeringPolicy {
            allow_steering: false,
            allow_interrupt: true,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: true,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: false,
            require_before_block: true,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("test_tier").expect("valid"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
        personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive").expect("valid"),
        concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
        resolution_fingerprint: RunProfileFingerprint::new("test-fingerprint-v1").expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    }
}

fn test_run_state(scope: TurnScope, status: TurnStatus) -> TurnRunState {
    TurnRunState {
        scope,
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status,
        accepted_message_ref: AcceptedMessageRef::new("test-msg").expect("valid"),
        source_binding_ref: SourceBindingRef::new("test-source").expect("valid"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("test-reply").expect("valid"),
        resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        failure: None,
        event_cursor: EventCursor(0),
    }
}

fn test_completed_exit() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![LoopMessageRef::new("msg:test-1").expect("valid")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: LoopExitId::new("exit:test-1").expect("valid"),
    })
}

// ─── Mock transition port ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum TransitionCall {
    Claim,
    Heartbeat,
    RecoverExpiredLeases,
    ApplyValidatedLoopExit,
    RecordModelRouteSnapshot,
    RecordRunnerFailure,
    Relinquish,
}

struct MockTransitionPort {
    claim_results: Mutex<Vec<Result<Option<ClaimedTurnRun>, TurnError>>>,
    heartbeat_result: Mutex<Result<EventCursor, TurnError>>,
    recover_result: Mutex<Result<RecoverExpiredLeasesResponse, TurnError>>,
    apply_exit_result: Mutex<Result<TurnRunState, TurnError>>,
    runner_failure_result: Mutex<Result<TurnRunState, TurnError>>,
    applied_mappings: Mutex<Vec<LoopExitMapping>>,
    calls: Mutex<Vec<TransitionCall>>,
    claim_requests: Mutex<Vec<ClaimRunRequest>>,
    heartbeat_requests: Mutex<Vec<HeartbeatRequest>>,
    recover_requests: Mutex<Vec<RecoverExpiredLeasesRequest>>,
    apply_exit_requests: Mutex<Vec<ApplyValidatedLoopExitRequest>>,
    route_snapshot_requests: Mutex<Vec<RecordModelRouteSnapshotRequest>>,
    runner_failure_requests: Mutex<Vec<RecordRunnerFailureRequest>>,
    relinquish_result: Mutex<Result<TurnRunState, TurnError>>,
    relinquish_requests: Mutex<Vec<RelinquishRunRequest>>,
}

impl MockTransitionPort {
    fn new() -> Self {
        Self {
            claim_results: Mutex::new(Vec::new()),
            heartbeat_result: Mutex::new(Ok(EventCursor(1))),
            recover_result: Mutex::new(Ok(RecoverExpiredLeasesResponse {
                recovered: Vec::new(),
            })),
            apply_exit_result: Mutex::new(Ok(test_run_state(test_scope(), TurnStatus::Completed))),
            runner_failure_result: Mutex::new(Ok(test_run_state(test_scope(), TurnStatus::Failed))),
            applied_mappings: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            claim_requests: Mutex::new(Vec::new()),
            heartbeat_requests: Mutex::new(Vec::new()),
            recover_requests: Mutex::new(Vec::new()),
            apply_exit_requests: Mutex::new(Vec::new()),
            route_snapshot_requests: Mutex::new(Vec::new()),
            runner_failure_requests: Mutex::new(Vec::new()),
            relinquish_result: Mutex::new(Ok(test_run_state(test_scope(), TurnStatus::Queued))),
            relinquish_requests: Mutex::new(Vec::new()),
        }
    }

    fn with_claim_result(self, result: Result<Option<ClaimedTurnRun>, TurnError>) -> Self {
        self.claim_results.lock().expect("lock").push(result);
        self
    }

    fn with_heartbeat_result(self, result: Result<EventCursor, TurnError>) -> Self {
        *self.heartbeat_result.lock().expect("lock") = result;
        self
    }

    fn calls(&self) -> Vec<TransitionCall> {
        self.calls.lock().expect("lock").clone()
    }

    fn applied_mappings(&self) -> Vec<LoopExitMapping> {
        self.applied_mappings.lock().expect("lock").clone()
    }

    fn route_snapshot_requests(&self) -> Vec<RecordModelRouteSnapshotRequest> {
        self.route_snapshot_requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl TurnRunTransitionPort for MockTransitionPort {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.calls.lock().expect("lock").push(TransitionCall::Claim);
        self.claim_requests
            .lock()
            .expect("lock")
            .push(request.clone());
        let mut results = self.claim_results.lock().expect("lock");
        if results.is_empty() {
            Ok(None)
        } else {
            results.remove(0).map(|maybe_claimed| {
                maybe_claimed.map(|mut claimed| {
                    claimed.runner_id = request.runner_id;
                    claimed.lease_token = request.lease_token;
                    claimed
                })
            })
        }
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::Heartbeat);
        self.heartbeat_requests.lock().expect("lock").push(request);
        self.heartbeat_result.lock().expect("lock").clone()
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::RecoverExpiredLeases);
        self.recover_requests.lock().expect("lock").push(request);
        self.recover_result.lock().expect("lock").clone()
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::RecordModelRouteSnapshot);
        request
            .snapshot
            .validate()
            .map_err(|reason| TurnError::InvalidRequest { reason })?;
        self.route_snapshot_requests
            .lock()
            .expect("lock")
            .push(request.clone());
        let mut state = test_run_state(test_scope(), TurnStatus::Running);
        state.resolved_model_route = Some(request.snapshot);
        Ok(state)
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(test_scope(), TurnStatus::BlockedApproval))
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(test_scope(), TurnStatus::Completed))
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(test_scope(), TurnStatus::Cancelled))
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(test_scope(), TurnStatus::Failed))
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::RecordRunnerFailure);
        self.runner_failure_requests
            .lock()
            .expect("lock")
            .push(request);
        self.runner_failure_result.lock().expect("lock").clone()
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::Relinquish);
        self.relinquish_requests.lock().expect("lock").push(request);
        self.relinquish_result.lock().expect("lock").clone()
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::ApplyValidatedLoopExit);
        self.applied_mappings
            .lock()
            .expect("lock")
            .push(request.mapping.clone());
        self.apply_exit_requests.lock().expect("lock").push(request);
        self.apply_exit_result.lock().expect("lock").clone()
    }
}

// ─── Mock driver ────────────────────────────────────────────────────────────

struct MockDriver {
    descriptor: AgentLoopDriverDescriptor,
    run_result: Mutex<Result<LoopExit, AgentLoopDriverError>>,
    run_delay: Duration,
    run_requests: Mutex<Vec<AgentLoopDriverRunRequest>>,
    resume_requests: Mutex<Vec<AgentLoopDriverResumeRequest>>,
}

impl MockDriver {
    fn completing(descriptor: AgentLoopDriverDescriptor) -> Self {
        Self {
            descriptor,
            run_result: Mutex::new(Ok(test_completed_exit())),
            run_delay: Duration::ZERO,
            run_requests: Mutex::new(Vec::new()),
            resume_requests: Mutex::new(Vec::new()),
        }
    }

    fn failing(descriptor: AgentLoopDriverDescriptor, error: AgentLoopDriverError) -> Self {
        Self {
            descriptor,
            run_result: Mutex::new(Err(error)),
            run_delay: Duration::ZERO,
            run_requests: Mutex::new(Vec::new()),
            resume_requests: Mutex::new(Vec::new()),
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.run_delay = delay;
        self
    }

    fn run_requests(&self) -> Vec<AgentLoopDriverRunRequest> {
        self.run_requests.lock().expect("lock").clone()
    }

    fn resume_requests(&self) -> Vec<AgentLoopDriverResumeRequest> {
        self.resume_requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AgentLoopDriver for MockDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run_requests.lock().expect("lock").push(request);
        if !self.run_delay.is_zero() {
            tokio::time::sleep(self.run_delay).await;
        }
        self.run_result.lock().expect("lock").clone()
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.resume_requests.lock().expect("lock").push(request);
        if !self.run_delay.is_zero() {
            tokio::time::sleep(self.run_delay).await;
        }
        self.run_result.lock().expect("lock").clone()
    }
}

struct PanickingDriver {
    descriptor: AgentLoopDriverDescriptor,
}

#[async_trait]
impl AgentLoopDriver for PanickingDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        panic!("simulated driver panic")
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        panic!("simulated driver resume panic")
    }
}

// ─── Stub host (mock driver never calls host methods) ───────────────────────

struct StubHost {
    context: ironclaw_turns::run_profile::LoopRunContext,
}

impl ironclaw_turns::run_profile::LoopRunInfoPort for StubHost {
    fn run_context(&self) -> &ironclaw_turns::run_profile::LoopRunContext {
        &self.context
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopContextPort for StubHost {
    async fn load_loop_context(
        &self,
        _request: ironclaw_turns::run_profile::LoopContextRequest,
    ) -> Result<ironclaw_turns::run_profile::LoopContextBundle, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopPromptPort for StubHost {
    async fn build_prompt_bundle(
        &self,
        _request: ironclaw_turns::run_profile::LoopPromptBundleRequest,
    ) -> Result<ironclaw_turns::run_profile::LoopPromptBundle, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopInputPort for StubHost {
    async fn poll_inputs(
        &self,
        _after: ironclaw_turns::run_profile::LoopInputCursor,
        _limit: usize,
    ) -> Result<ironclaw_turns::run_profile::LoopInputBatch, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }

    async fn ack_inputs(
        &self,
        _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
    ) -> Result<(), AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopModelPort for StubHost {
    async fn stream_model(
        &self,
        _request: ironclaw_turns::run_profile::LoopModelRequest,
    ) -> Result<ironclaw_turns::run_profile::LoopModelResponse, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCompactionPort for StubHost {
    async fn compact_loop_context(
        &self,
        _request: ironclaw_turns::run_profile::LoopCompactionRequest,
    ) -> Result<
        ironclaw_turns::run_profile::LoopCompactionResponse,
        ironclaw_turns::run_profile::LoopCompactionError,
    > {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCapabilityPort for StubHost {
    async fn visible_capabilities(
        &self,
        _request: ironclaw_turns::run_profile::VisibleCapabilityRequest,
    ) -> Result<ironclaw_turns::run_profile::VisibleCapabilitySurface, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }

    async fn invoke_capability(
        &self,
        _request: ironclaw_turns::run_profile::CapabilityInvocation,
    ) -> Result<ironclaw_turns::run_profile::CapabilityOutcome, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }

    async fn invoke_capability_batch(
        &self,
        _request: ironclaw_turns::run_profile::CapabilityBatchInvocation,
    ) -> Result<ironclaw_turns::run_profile::CapabilityBatchOutcome, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopTranscriptPort for StubHost {
    async fn finalize_assistant_message(
        &self,
        _request: ironclaw_turns::run_profile::FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCheckpointPort for StubHost {
    async fn checkpoint(
        &self,
        _request: ironclaw_turns::run_profile::LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopProgressPort for StubHost {
    async fn emit_loop_progress(
        &self,
        _event: ironclaw_turns::run_profile::LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        unimplemented!("stub host: never called by mock driver")
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCancellationPort for StubHost {
    fn observe_cancellation(&self) -> Option<ironclaw_turns::run_profile::LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> ironclaw_turns::run_profile::LoopCancellationSignal {
        std::future::pending().await
    }
}

// ─── Mock host factory ──────────────────────────────────────────────────────

struct MockHostFactory;

impl MockHostFactory {
    fn host_for_claim(claimed: &ClaimedTurnRun) -> StubHost {
        let mut context = ironclaw_turns::run_profile::LoopRunContext::new(
            claimed.state.scope.clone(),
            claimed.state.turn_id,
            claimed.state.run_id,
            claimed.resolved_run_profile.clone(),
        );
        if let Some(snapshot) = claimed.state.resolved_model_route.clone() {
            context = context.with_resolved_model_route(snapshot);
        }
        StubHost { context }
    }
}

#[async_trait]
impl HostFactory for MockHostFactory {
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        Ok(Box::new(Self::host_for_claim(claimed)))
    }
}

struct RouteSnapshotHostFactory {
    snapshot: LoopModelRouteSnapshot,
}

#[async_trait]
impl HostFactory for RouteSnapshotHostFactory {
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        let mut host = MockHostFactory::host_for_claim(claimed);
        host.context = host
            .context
            .with_resolved_model_route(self.snapshot.clone());
        Ok(Box::new(host))
    }
}

struct FailingHostFactory {
    reason: String,
}

#[async_trait]
impl HostFactory for FailingHostFactory {
    async fn create_host(
        &self,
        _claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        Err(HostFactoryError::new(self.reason.clone()))
    }
}

// ─── Test setup ─────────────────────────────────────────────────────────────

fn make_claimed_run(
    descriptor: &AgentLoopDriverDescriptor,
    scope: TurnScope,
    status: TurnStatus,
) -> ClaimedTurnRun {
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let profile = test_resolved_profile_with_driver(descriptor);
    let mut state = test_run_state(scope, status);
    state.resolved_run_profile_id = profile.profile_id.clone();
    state.resolved_run_profile_version = profile.profile_version;
    ClaimedTurnRun {
        state,
        resolved_run_profile: profile,
        runner_id,
        lease_token,
    }
}

fn assert_first_terminal_failure_matches_first_claim(port: &MockTransitionPort, run_id: TurnRunId) {
    let claim_requests = port.claim_requests.lock().expect("lock");
    let runner_failure_requests = port.runner_failure_requests.lock().expect("lock");
    let claim = claim_requests
        .first()
        .expect("worker should issue a claim request before recovery");
    let runner_failure = runner_failure_requests
        .first()
        .expect("worker should record terminal failure");
    assert_eq!(runner_failure.run_id, run_id);
    assert_eq!(runner_failure.runner_id, claim.runner_id);
    assert_eq!(runner_failure.lease_token, claim.lease_token);
}

fn setup_registry(driver: Arc<dyn AgentLoopDriver>) -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            driver,
            DriverRequirements::all_optional(),
            DriverKind::Production,
        )
        .expect("registration should succeed");
    registry
}

fn make_applier(port: Arc<MockTransitionPort>) -> Arc<LoopExitApplier> {
    Arc::new(LoopExitApplier::new(
        port,
        Arc::new(InMemoryLoopExitEvidencePort::all_verified()),
    ))
}

fn make_fail_closed_recovery_applier(port: Arc<MockTransitionPort>) -> Arc<LoopExitApplier> {
    Arc::new(LoopExitApplier::new(
        port,
        Arc::new(
            InMemoryLoopExitEvidencePort::new()
                .with_latest_checkpoint_kind(Some(LoopCheckpointKind::BeforeSideEffect)),
        ),
    ))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn worker_recovers_expired_leases_before_claiming() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    wake_sender.wake();
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let calls = port.calls();
    let recover_pos = calls
        .iter()
        .position(|call| *call == TransitionCall::RecoverExpiredLeases)
        .expect("worker should recover expired leases before claiming");
    let claim_pos = calls
        .iter()
        .position(|call| *call == TransitionCall::Claim)
        .expect("worker should claim after recovery sweep");
    assert!(recover_pos < claim_pos);
    assert!(!port.recover_requests.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn worker_reuses_claim_runner_and_lease_for_heartbeat_and_exit() {
    let desc = test_descriptor();
    let driver =
        Arc::new(MockDriver::completing(desc.clone()).with_delay(Duration::from_millis(150)));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_secs(60),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );
    let worker_runner_id = worker.runner_id();

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    wake_sender.wake();
    tokio::time::sleep(Duration::from_millis(250)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let claim_requests = port.claim_requests.lock().expect("lock").clone();
    let first_claim = claim_requests
        .first()
        .expect("worker should issue a claim request");
    assert_eq!(first_claim.runner_id, worker_runner_id);

    let heartbeat_requests = port.heartbeat_requests.lock().expect("lock").clone();
    assert!(
        heartbeat_requests
            .iter()
            .any(|request| request.run_id == run_id
                && request.runner_id == first_claim.runner_id
                && request.lease_token == first_claim.lease_token),
        "heartbeat must use the same runner and lease token as the claim"
    );

    let apply_requests = port.apply_exit_requests.lock().expect("lock").clone();
    assert!(
        apply_requests.iter().any(|request| request.run_id == run_id
            && request.runner_id == first_claim.runner_id
            && request.lease_token == first_claim.lease_token),
        "exit application must use the same runner and lease token as the claim"
    );
}

#[tokio::test]
async fn worker_resumes_claimed_run_with_checkpoint() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver.clone()));
    let checkpoint_id = TurnCheckpointId::new();
    let mut claimed = make_claimed_run(&desc, test_scope(), TurnStatus::RecoveryRequired);
    claimed.state.checkpoint_id = Some(checkpoint_id);
    let turn_id = claimed.state.turn_id;
    let run_id = claimed.state.run_id;
    let profile = claimed.resolved_run_profile.clone();
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: Duration::from_secs(60),
            poll_interval: Duration::from_secs(60),
            scope_filter: None,
        },
        port.clone(),
        make_applier(port),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    assert!(
        worker
            .try_claim_and_run(&CancellationToken::new())
            .await
            .unwrap()
    );

    assert!(
        driver.run_requests().is_empty(),
        "checkpointed recovery run must not start from scratch"
    );
    let resume_requests = driver.resume_requests();
    assert_eq!(resume_requests.len(), 1);
    assert_eq!(resume_requests[0].turn_id, turn_id);
    assert_eq!(resume_requests[0].run_id, run_id);
    assert_eq!(resume_requests[0].checkpoint_id, checkpoint_id);
    assert_eq!(resume_requests[0].resolved_run_profile, profile);
}

#[tokio::test]
async fn worker_resumes_requeued_claimed_run_with_checkpoint() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver.clone()));
    let checkpoint_id = TurnCheckpointId::new();
    let mut claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    claimed.state.checkpoint_id = Some(checkpoint_id);
    let turn_id = claimed.state.turn_id;
    let run_id = claimed.state.run_id;
    let profile = claimed.resolved_run_profile.clone();
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: Duration::from_secs(60),
            poll_interval: Duration::from_secs(60),
            scope_filter: None,
        },
        port.clone(),
        make_applier(port),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    assert!(
        worker
            .try_claim_and_run(&CancellationToken::new())
            .await
            .unwrap()
    );

    assert!(
        driver.run_requests().is_empty(),
        "requeued checkpointed run must not start from scratch"
    );
    let resume_requests = driver.resume_requests();
    assert_eq!(resume_requests.len(), 1);
    assert_eq!(resume_requests[0].turn_id, turn_id);
    assert_eq!(resume_requests[0].run_id, run_id);
    assert_eq!(resume_requests[0].checkpoint_id, checkpoint_id);
    assert_eq!(resume_requests[0].resolved_run_profile, profile);
}

#[tokio::test]
async fn worker_persists_host_model_route_snapshot_before_driver_exit() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    let snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: Duration::from_secs(60),
            poll_interval: Duration::from_secs(60),
            scope_filter: None,
        },
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(RouteSnapshotHostFactory {
            snapshot: snapshot.clone(),
        }),
        wake_receiver,
    );

    assert!(
        worker
            .try_claim_and_run(&CancellationToken::new())
            .await
            .unwrap()
    );

    let requests = port.route_snapshot_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].run_id, run_id);
    assert_eq!(requests[0].snapshot, snapshot);
    let calls = port.calls();
    let route_pos = calls
        .iter()
        .position(|call| *call == TransitionCall::RecordModelRouteSnapshot)
        .expect("route snapshot should be recorded");
    let exit_pos = calls
        .iter()
        .position(|call| *call == TransitionCall::ApplyValidatedLoopExit)
        .expect("exit should be applied");
    assert!(route_pos < exit_pos);
}

#[tokio::test]
async fn worker_rejects_unvalidated_host_route_snapshot_before_persist() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: Duration::from_secs(60),
            poll_interval: Duration::from_secs(60),
            scope_filter: None,
        },
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(RouteSnapshotHostFactory {
            snapshot: LoopModelRouteSnapshot::new(
                "openrouter",
                "anthropic/secret-model",
                "config:v1",
                "auth:v1",
            ),
        }),
        wake_receiver,
    );

    assert!(
        worker
            .try_claim_and_run(&CancellationToken::new())
            .await
            .unwrap()
    );

    assert!(port.route_snapshot_requests().is_empty());
    let calls = port.calls();
    assert!(calls.contains(&TransitionCall::RecordModelRouteSnapshot));
    assert!(calls.contains(&TransitionCall::RecordRunnerFailure));
    assert!(!calls.contains(&TransitionCall::ApplyValidatedLoopExit));
}

#[tokio::test]
async fn default_worker_policy_rejects_fabricated_completion_refs() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig::default(),
        port.clone(),
        make_fail_closed_recovery_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    worker.try_claim_and_run(&cancel).await.unwrap();

    assert!(
        port.calls()
            .contains(&TransitionCall::ApplyValidatedLoopExit)
    );
    assert!(matches!(
        port.applied_mappings().as_slice(),
        [LoopExitMapping::RunnerOutcome(
            TurnRunnerOutcome::Failed { .. }
        )]
    ));
}

#[tokio::test]
async fn worker_claims_and_completes_run() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let calls = port.calls();
    assert!(calls.contains(&TransitionCall::Claim));
    assert!(calls.contains(&TransitionCall::ApplyValidatedLoopExit));
    assert!(matches!(
        port.applied_mappings().as_slice(),
        [LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)]
    ));
}

#[tokio::test]
async fn worker_records_terminal_failure_when_heartbeat_fails() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()).with_delay(Duration::from_secs(60)));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(
        MockTransitionPort::new()
            .with_claim_result(Ok(Some(claimed)))
            .with_heartbeat_result(Err(TurnError::LeaseMismatch)),
    );

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_millis(10),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let result = tokio::time::timeout(
        Duration::from_millis(300),
        worker.try_claim_and_run(&cancel),
    )
    .await;
    assert!(
        result.is_ok(),
        "heartbeat failure should stop active driver promptly"
    );
    result.unwrap().unwrap();
    assert!(port.calls().contains(&TransitionCall::Heartbeat));
    assert!(port.calls().contains(&TransitionCall::RecordRunnerFailure));
    assert_first_terminal_failure_matches_first_claim(&port, run_id);
}

#[tokio::test]
async fn worker_cancellation_relinquishes_run() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()).with_delay(Duration::from_secs(60)));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let mut handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_millis(300), &mut handle).await;
    if result.is_err() {
        handle.abort();
        panic!("worker cancellation should stop active driver promptly");
    }
    result.unwrap().expect("worker task should complete");
    // WorkerCancelled routes through relinquish_run (re-queue) rather than
    // record_runner_failure (terminal), so the run stays available for retry.
    assert!(
        port.calls().contains(&TransitionCall::Relinquish),
        "WorkerCancelled should relinquish the run, not terminate it"
    );
    assert!(!port.calls().contains(&TransitionCall::RecordRunnerFailure));
    let claim_requests = port.claim_requests.lock().expect("lock");
    let relinquish_requests = port.relinquish_requests.lock().expect("lock");
    let claim = claim_requests.first().expect("claim should be issued");
    let relinquish = relinquish_requests
        .first()
        .expect("relinquish should be issued");
    assert_eq!(relinquish.run_id, run_id);
    assert_eq!(relinquish.runner_id, claim.runner_id);
    assert_eq!(relinquish.lease_token, claim.lease_token);
}

#[tokio::test]
async fn worker_records_terminal_failure_on_driver_error() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::failing(
        desc.clone(),
        AgentLoopDriverError::Failed {
            reason_kind: "test_failure".to_string(),
        },
    ));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(port.calls().contains(&TransitionCall::RecordRunnerFailure));
    assert_first_terminal_failure_matches_first_claim(&port, run_id);
}

#[tokio::test]
async fn worker_records_terminal_failure_on_driver_panic() {
    let desc = test_descriptor();
    let driver = Arc::new(PanickingDriver {
        descriptor: desc.clone(),
    });
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(port.calls().contains(&TransitionCall::RecordRunnerFailure));
}

#[tokio::test]
async fn worker_records_terminal_failure_on_host_factory_error() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let host_factory = Arc::new(FailingHostFactory {
        reason: "test host creation failure".to_string(),
    });

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        host_factory,
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(port.calls().contains(&TransitionCall::RecordRunnerFailure));
    assert_first_terminal_failure_matches_first_claim(&port, run_id);
}

#[tokio::test]
async fn worker_records_terminal_failure_when_driver_not_found() {
    let registered_desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(registered_desc.clone()));
    let registry = Arc::new(setup_registry(driver));

    let mut claimed_desc = test_descriptor();
    claimed_desc.id = LoopDriverId::new("missing_driver").expect("valid");
    let claimed = make_claimed_run(&claimed_desc, test_scope(), TurnStatus::Queued);
    let run_id = claimed.state.run_id;
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(port.calls().contains(&TransitionCall::RecordRunnerFailure));
    assert_first_terminal_failure_matches_first_claim(&port, run_id);
}

#[tokio::test]
async fn worker_continues_when_no_runs_available() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let port = Arc::new(MockTransitionPort::new());

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let claim_count = port
        .calls()
        .iter()
        .filter(|c| **c == TransitionCall::Claim)
        .count();
    assert!(
        claim_count >= 1,
        "should have attempted claims, got {claim_count}"
    );
}

#[tokio::test]
async fn wake_signal_drains_available_runs_until_queue_empty() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let first = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let second = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(
        MockTransitionPort::new()
            .with_claim_result(Ok(Some(first)))
            .with_claim_result(Ok(Some(second))),
    );

    let (wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    wake_sender.wake();
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let calls = port.calls();
    let claim_count = calls
        .iter()
        .filter(|call| **call == TransitionCall::Claim)
        .count();
    let apply_count = calls
        .iter()
        .filter(|call| **call == TransitionCall::ApplyValidatedLoopExit)
        .count();
    assert!(
        claim_count >= 3,
        "drain should process queued runs and observe an empty claim"
    );
    assert_eq!(
        apply_count, 2,
        "single wake should process both queued runs"
    );
}

#[tokio::test]
async fn wake_signal_triggers_claim_attempt() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let port = Arc::new(MockTransitionPort::new());

    let (wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60), // very long so wake is the trigger
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    wake_sender.wake();
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(port.calls().contains(&TransitionCall::Claim));
}

#[tokio::test]
async fn heartbeat_runs_during_driver_execution() {
    let desc = test_descriptor();
    let driver =
        Arc::new(MockDriver::completing(desc.clone()).with_delay(Duration::from_millis(300)));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_millis(50), // fast heartbeats
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(600)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let heartbeat_count = port
        .calls()
        .iter()
        .filter(|c| **c == TransitionCall::Heartbeat)
        .count();
    assert!(
        heartbeat_count >= 2,
        "should have sent multiple heartbeats, got {heartbeat_count}"
    );
}

#[tokio::test]
async fn worker_generates_stable_runner_id() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let port = Arc::new(MockTransitionPort::new());
    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();

    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig::default(),
        port.clone(),
        make_applier(port.clone()),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
    );

    let id1 = worker.runner_id();
    let id2 = worker.runner_id();
    assert_eq!(id1, id2, "runner_id should be stable across calls");
}
