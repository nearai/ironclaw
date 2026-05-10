use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use ironclaw_host_api::{TenantId, ThreadId};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, EventCursor, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopMessageRef, ReplyTargetBindingRef,
    RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnError, TurnId, TurnLeaseToken,
    TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{AgentLoopDriverHost, AgentLoopHostError, CheckpointSchemaId, LoopDriverId},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordRecoveryRequiredRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort,
    },
};

use crate::driver_registry::{DriverKind, DriverRegistry, DriverRequirements};
use crate::loop_exit_applier::{InMemoryLoopExitEvidencePort, LoopExitApplier};

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
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("test_tier").expect("valid"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
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
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status,
        accepted_message_ref: AcceptedMessageRef::new("test-msg").expect("valid"),
        source_binding_ref: SourceBindingRef::new("test-source").expect("valid"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("test-reply").expect("valid"),
        resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
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
    ApplyValidatedLoopExit,
    RecordRecoveryRequired,
}

struct MockTransitionPort {
    claim_results: Mutex<Vec<Result<Option<ClaimedTurnRun>, TurnError>>>,
    heartbeat_result: Mutex<Result<EventCursor, TurnError>>,
    apply_exit_result: Mutex<Result<TurnRunState, TurnError>>,
    recovery_result: Mutex<Result<TurnRunState, TurnError>>,
    calls: Mutex<Vec<TransitionCall>>,
}

impl MockTransitionPort {
    fn new() -> Self {
        Self {
            claim_results: Mutex::new(Vec::new()),
            heartbeat_result: Mutex::new(Ok(EventCursor(1))),
            apply_exit_result: Mutex::new(Ok(test_run_state(test_scope(), TurnStatus::Completed))),
            recovery_result: Mutex::new(Ok(test_run_state(
                test_scope(),
                TurnStatus::RecoveryRequired,
            ))),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn with_claim_result(self, result: Result<Option<ClaimedTurnRun>, TurnError>) -> Self {
        self.claim_results.lock().expect("lock").push(result);
        self
    }

    fn calls(&self) -> Vec<TransitionCall> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl TurnRunTransitionPort for MockTransitionPort {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.calls.lock().expect("lock").push(TransitionCall::Claim);
        let mut results = self.claim_results.lock().expect("lock");
        if results.is_empty() {
            Ok(None)
        } else {
            results.remove(0)
        }
    }

    async fn heartbeat(&self, _request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::Heartbeat);
        self.heartbeat_result.lock().expect("lock").clone()
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        Ok(RecoverExpiredLeasesResponse {
            recovered: Vec::new(),
        })
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

    async fn record_recovery_required(
        &self,
        _request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::RecordRecoveryRequired);
        self.recovery_result.lock().expect("lock").clone()
    }

    async fn apply_validated_loop_exit(
        &self,
        _request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.calls
            .lock()
            .expect("lock")
            .push(TransitionCall::ApplyValidatedLoopExit);
        self.apply_exit_result.lock().expect("lock").clone()
    }
}

// ─── Mock driver ────────────────────────────────────────────────────────────

struct MockDriver {
    descriptor: AgentLoopDriverDescriptor,
    run_result: Mutex<Result<LoopExit, AgentLoopDriverError>>,
    run_delay: Duration,
}

impl MockDriver {
    fn completing(descriptor: AgentLoopDriverDescriptor) -> Self {
        Self {
            descriptor,
            run_result: Mutex::new(Ok(test_completed_exit())),
            run_delay: Duration::ZERO,
        }
    }

    fn failing(descriptor: AgentLoopDriverDescriptor, error: AgentLoopDriverError) -> Self {
        Self {
            descriptor,
            run_result: Mutex::new(Err(error)),
            run_delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.run_delay = delay;
        self
    }
}

#[async_trait]
impl AgentLoopDriver for MockDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        if !self.run_delay.is_zero() {
            tokio::time::sleep(self.run_delay).await;
        }
        self.run_result.lock().expect("lock").clone()
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        if !self.run_delay.is_zero() {
            tokio::time::sleep(self.run_delay).await;
        }
        self.run_result.lock().expect("lock").clone()
    }
}

struct BlockingDriver {
    descriptor: AgentLoopDriverDescriptor,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl BlockingDriver {
    fn new(
        descriptor: AgentLoopDriverDescriptor,
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            descriptor,
            started,
            release,
        }
    }
}

#[async_trait]
impl AgentLoopDriver for BlockingDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(test_completed_exit())
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(test_completed_exit())
    }
}

// ─── Stub host (mock driver never calls host methods) ───────────────────────

struct StubHost;

impl ironclaw_turns::run_profile::LoopRunInfoPort for StubHost {
    fn run_context(&self) -> &ironclaw_turns::run_profile::LoopRunContext {
        unimplemented!("stub host: never called by mock driver")
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
        _cursor: ironclaw_turns::run_profile::LoopInputCursor,
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

// ─── Mock host factory ──────────────────────────────────────────────────────

struct MockHostFactory;

#[async_trait]
impl HostFactory for MockHostFactory {
    async fn create_host(
        &self,
        _claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        Ok(Box::new(StubHost))
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

fn make_applier(port: Arc<dyn TurnRunTransitionPort>) -> Arc<LoopExitApplier> {
    let evidence = Arc::new(InMemoryLoopExitEvidencePort::all_verified());
    Arc::new(LoopExitApplier::new(port, evidence))
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

// ─── Tests ──────────────────────────────────────────────────────────────────

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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
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
}

#[tokio::test]
async fn worker_records_recovery_when_exit_application_fails() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    {
        let mut apply_exit_result = port.apply_exit_result.lock().expect("lock");
        *apply_exit_result = Err(TurnError::Unavailable {
            reason: "apply failed".to_string(),
        });
    }

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let calls = port.calls();
    assert!(calls.contains(&TransitionCall::ApplyValidatedLoopExit));
    assert!(calls.contains(&TransitionCall::RecordRecoveryRequired));
}

#[tokio::test]
async fn worker_cancellation_preempts_inflight_driver_and_records_recovery() {
    let desc = test_descriptor();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let driver = Arc::new(BlockingDriver::new(
        desc.clone(),
        started.clone(),
        release.clone(),
    ));
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    wake_sender.wake();
    started.notified().await;
    cancel.cancel();

    let finished = tokio::time::timeout(Duration::from_millis(200), async {
        while !handle.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    if !finished {
        release.notify_one();
    }
    assert!(
        finished,
        "worker should stop active driver after cancellation"
    );

    handle.await.expect("worker task should complete");

    let calls = port.calls();
    assert!(calls.contains(&TransitionCall::RecordRecoveryRequired));
    assert!(!calls.contains(&TransitionCall::ApplyValidatedLoopExit));
    let claim_count = calls
        .iter()
        .filter(|call| **call == TransitionCall::Claim)
        .count();
    assert_eq!(claim_count, 1, "cancellation should stop future claims");
}

#[tokio::test]
async fn worker_records_recovery_on_driver_error() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::failing(
        desc.clone(),
        AgentLoopDriverError::Failed {
            reason_kind: "test_failure".to_string(),
        },
    ));
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(
        port.calls()
            .contains(&TransitionCall::RecordRecoveryRequired)
    );
}

#[tokio::test]
async fn worker_records_recovery_on_host_factory_error() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
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
        registry,
        host_factory,
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(
        port.calls()
            .contains(&TransitionCall::RecordRecoveryRequired)
    );
}

#[tokio::test]
async fn worker_records_recovery_when_driver_not_found() {
    let registered_desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(registered_desc.clone()));
    let registry = Arc::new(setup_registry(driver));

    let mut claimed_desc = test_descriptor();
    claimed_desc.id = LoopDriverId::new("missing_driver").expect("valid");
    let claimed = make_claimed_run(&claimed_desc, test_scope(), TurnStatus::Queued);
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    assert!(
        port.calls()
            .contains(&TransitionCall::RecordRecoveryRequired)
    );
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
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
    assert_eq!(claim_count, 3, "drain should stop after empty claim");
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
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
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
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
async fn heartbeat_failure_records_recovery_without_applying_exit() {
    let desc = test_descriptor();
    let driver =
        Arc::new(MockDriver::completing(desc.clone()).with_delay(Duration::from_millis(300)));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));
    {
        let mut heartbeat_result = port.heartbeat_result.lock().expect("lock");
        *heartbeat_result = Err(TurnError::LeaseMismatch);
    }

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::from_millis(50),
        poll_interval: Duration::from_millis(50),
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    handle.await.expect("worker task should complete");

    let calls = port.calls();
    assert!(calls.contains(&TransitionCall::Heartbeat));
    assert!(calls.contains(&TransitionCall::RecordRecoveryRequired));
    assert!(!calls.contains(&TransitionCall::ApplyValidatedLoopExit));
}

#[tokio::test]
async fn zero_duration_config_does_not_panic() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let claimed = make_claimed_run(&desc, test_scope(), TurnStatus::Queued);
    let port = Arc::new(MockTransitionPort::new().with_claim_result(Ok(Some(claimed))));

    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();
    let config = TurnRunnerWorkerConfig {
        heartbeat_interval: Duration::ZERO,
        poll_interval: Duration::ZERO,
        scope_filter: None,
    };

    let worker = TurnRunnerWorker::new(
        config,
        port.clone(),
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        make_applier(port.clone()),
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should not hang");
    join.expect("worker should not panic");
}

#[tokio::test]
async fn worker_generates_stable_runner_id() {
    let desc = test_descriptor();
    let driver = Arc::new(MockDriver::completing(desc.clone()));
    let registry = Arc::new(setup_registry(driver));
    let port = Arc::new(MockTransitionPort::new());
    let (_ws, wake_receiver) = TurnRunnerWakeReceiver::new();

    let applier = make_applier(port.clone());
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig::default(),
        port,
        registry,
        Arc::new(MockHostFactory),
        wake_receiver,
        applier,
    );

    let id1 = worker.runner_id();
    let id2 = worker.runner_id();
    assert_eq!(id1, id2, "runner_id should be stable across calls");
}
