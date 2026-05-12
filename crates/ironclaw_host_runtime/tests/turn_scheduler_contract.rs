use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntimeServices, ProductionWiringComponent,
    ProductionWiringIssueKind, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator,
    GetRunStateRequest, IdempotencyKey, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    NoopTurnRunWakeNotifier, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileRequest, SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunState, TurnRunWake,
    TurnRunWakeNotifier, TurnRunWakeNotifyError, TurnRunnerId, TurnScope, TurnStateStore,
    TurnStatus,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordRecoveryRequiredRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort,
    },
};
use tokio::{sync::Notify, time::timeout};

#[derive(Default)]
struct CompletingExecutor {
    started: AtomicUsize,
    notify_started: Notify,
    gate: Mutex<Option<Arc<Notify>>>,
}

impl CompletingExecutor {
    fn with_gate(gate: Arc<Notify>) -> Self {
        Self {
            gate: Mutex::new(Some(gate)),
            ..Self::default()
        }
    }

    async fn wait_for_started(&self, expected: usize) {
        timeout(Duration::from_secs(2), async {
            loop {
                if self.started.load(Ordering::SeqCst) >= expected {
                    return;
                }
                self.notify_started.notified().await;
            }
        })
        .await
        .expect("executor did not start expected runs");
    }

    fn started_count(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TurnRunExecutor for CompletingExecutor {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        let started = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        self.notify_started.notify_waiters();
        let gate = self.gate.lock().unwrap().clone();
        if started == 1
            && let Some(gate) = gate
        {
            gate.notified().await;
        }
        transitions
            .complete_run(CompleteRunRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
            })
            .await
            .unwrap();
        Ok(())
    }
}

#[derive(Default)]
struct FailingExecutor {
    started: AtomicUsize,
    notify_started: Notify,
}

#[derive(Default)]
struct PanickingExecutor {
    started: AtomicUsize,
    notify_started: Notify,
}

#[derive(Default)]
struct FailingClaimTransitions {
    claim_attempts: AtomicUsize,
    notify_claim: Notify,
}

#[derive(Default)]
struct DurableLikeTurnStore {
    inner: InMemoryTurnStateStore,
}

#[derive(Debug)]
struct DurableTurnStoreStub;

#[derive(Default)]
struct HangingExecutor {
    started: AtomicUsize,
    notify_started: Notify,
}

impl FailingExecutor {
    async fn wait_for_started(&self) {
        timeout(Duration::from_secs(2), async {
            while self.started.load(Ordering::SeqCst) == 0 {
                self.notify_started.notified().await;
            }
        })
        .await
        .expect("executor did not start");
    }
}

#[async_trait]
impl TurnRunExecutor for FailingExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.notify_started.notify_waiters();
        Err(TurnRunExecutorError::new("scheduler_test_error").unwrap())
    }
}

impl PanickingExecutor {
    async fn wait_for_started(&self) {
        timeout(Duration::from_secs(2), async {
            while self.started.load(Ordering::SeqCst) == 0 {
                self.notify_started.notified().await;
            }
        })
        .await
        .expect("executor did not start");
    }
}

#[async_trait]
impl TurnRunExecutor for PanickingExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.notify_started.notify_waiters();
        panic!("scheduler test panic");
    }
}

impl FailingClaimTransitions {
    async fn wait_for_claim_attempts(&self, expected: usize) {
        assert!(
            self.wait_for_claim_attempts_for(expected, Duration::from_secs(2))
                .await,
            "scheduler did not reach expected claim attempts"
        );
    }

    async fn wait_for_claim_attempts_for(&self, expected: usize, duration: Duration) -> bool {
        timeout(duration, async {
            while self.claim_attempts.load(Ordering::SeqCst) < expected {
                self.notify_claim.notified().await;
            }
        })
        .await
        .is_ok()
    }

    fn claim_attempts(&self) -> usize {
        self.claim_attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TurnRunTransitionPort for FailingClaimTransitions {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.claim_attempts.fetch_add(1, Ordering::SeqCst);
        self.notify_claim.notify_waiters();
        Err(TurnError::Unavailable {
            reason: "claim store unavailable".to_string(),
        })
    }

    async fn heartbeat(
        &self,
        _request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        panic!("failing claim transitions should not heartbeat")
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        Ok(RecoverExpiredLeasesResponse { recovered: vec![] })
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not block runs")
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not complete runs")
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not cancel runs")
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not fail runs")
    }

    async fn record_recovery_required(
        &self,
        _request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not record recovery")
    }

    async fn apply_validated_loop_exit(
        &self,
        _request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not apply loop exits")
    }
}

#[async_trait]
impl TurnStateStore for DurableLikeTurnStore {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner
            .submit_turn(request, admission_policy, run_profile_resolver)
            .await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.inner.resume_turn(request).await
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        self.inner.request_cancel(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

#[async_trait]
impl TurnRunTransitionPort for DurableLikeTurnStore {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.inner.claim_next_run(request).await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        self.inner.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.inner.recover_expired_leases(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.inner.block_run(request).await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.inner.complete_run(request).await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.cancel_run(request).await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.inner.fail_run(request).await
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_recovery_required(request).await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.apply_validated_loop_exit(request).await
    }
}

#[async_trait]
impl TurnStateStore for DurableTurnStoreStub {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("store stub should not submit turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("store stub should not resume turns")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("store stub should not cancel turns")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("store stub should not read turns")
    }
}

#[async_trait]
impl TurnRunTransitionPort for DurableTurnStoreStub {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        panic!("transition stub should not claim turns")
    }

    async fn heartbeat(
        &self,
        _request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        panic!("transition stub should not heartbeat")
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        panic!("transition stub should not recover leases")
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not block runs")
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not complete runs")
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not cancel runs")
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not fail runs")
    }

    async fn record_recovery_required(
        &self,
        _request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not record recovery")
    }

    async fn apply_validated_loop_exit(
        &self,
        _request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not apply loop exits")
    }
}

#[async_trait]
impl TurnRunExecutor for HangingExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.notify_started.notify_waiters();
        std::future::pending::<()>().await;
        Ok(())
    }
}

impl HangingExecutor {
    async fn wait_for_started(&self) {
        timeout(Duration::from_secs(2), async {
            while self.started.load(Ordering::SeqCst) == 0 {
                self.notify_started.notified().await;
            }
        })
        .await
        .expect("hanging executor did not start");
    }
}

struct HeartbeatTrackingTransitions {
    store: Arc<InMemoryTurnStateStore>,
    heartbeats: AtomicUsize,
    notify_heartbeat: Notify,
}

struct ClaimRecordingTransitions {
    store: Arc<InMemoryTurnStateStore>,
    claim_runner_ids: Mutex<Vec<TurnRunnerId>>,
}

impl HeartbeatTrackingTransitions {
    fn new(store: Arc<InMemoryTurnStateStore>) -> Self {
        Self {
            store,
            heartbeats: AtomicUsize::new(0),
            notify_heartbeat: Notify::new(),
        }
    }

    async fn wait_for_heartbeats(&self, expected: usize) {
        timeout(Duration::from_secs(2), async {
            while self.heartbeats.load(Ordering::SeqCst) < expected {
                self.notify_heartbeat.notified().await;
            }
        })
        .await
        .expect("scheduler did not heartbeat claimed run");
    }
}

impl ClaimRecordingTransitions {
    fn new(store: Arc<InMemoryTurnStateStore>) -> Self {
        Self {
            store,
            claim_runner_ids: Mutex::new(Vec::new()),
        }
    }

    fn claim_runner_ids(&self) -> Vec<TurnRunnerId> {
        self.claim_runner_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnRunTransitionPort for HeartbeatTrackingTransitions {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.store.claim_next_run(request).await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        let result = self.store.heartbeat(request).await;
        if result.is_ok() {
            self.heartbeats.fetch_add(1, Ordering::SeqCst);
            self.notify_heartbeat.notify_waiters();
        }
        result
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.store.recover_expired_leases(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.block_run(request).await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.complete_run(request).await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.cancel_run(request).await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.fail_run(request).await
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_recovery_required(request).await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.apply_validated_loop_exit(request).await
    }
}

#[async_trait]
impl TurnRunTransitionPort for ClaimRecordingTransitions {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.claim_runner_ids
            .lock()
            .unwrap()
            .push(request.runner_id);
        self.store.claim_next_run(request).await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        self.store.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.store.recover_expired_leases(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.block_run(request).await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.complete_run(request).await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.cancel_run(request).await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.store.fail_run(request).await
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_recovery_required(request).await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.apply_validated_loop_exit(request).await
    }
}

#[test]
fn executor_error_exposes_typed_sanitized_failure() {
    let error = TurnRunExecutorError::new("scheduler_test_error").unwrap();

    assert_eq!(error.failure().category(), "scheduler_test_error");
    assert_eq!(error.failure_category(), "scheduler_test_error");
}

#[test]
fn production_services_build_scheduler_from_configured_transition_port_without_notifier() {
    let store = Arc::new(DurableTurnStoreStub);
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state_and_transition_port(store);
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(CompletingExecutor::default());

    let _scheduler = services
        .turn_scheduler_for_production(executor, fast_config())
        .expect("production turn scheduler should build from configured transition port");
}

#[test]
fn production_services_reject_unverified_scheduler_transition_port() {
    let turn_state = Arc::new(DurableTurnStoreStub);
    let transition_port = Arc::new(DurableTurnStoreStub);
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state(turn_state)
    .with_turn_run_transition_port(transition_port);
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(CompletingExecutor::default());

    let result = services.turn_scheduler_for_production(executor, fast_config());
    let Err(report) = result else {
        panic!("production scheduler should reject unverified transition port");
    };
    assert!(report.contains(
        ProductionWiringComponent::TurnState,
        ProductionWiringIssueKind::UnverifiedProductionImplementation
    ));
}

#[tokio::test]
async fn scheduler_uses_stable_runner_id_across_claims() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions = Arc::new(ClaimRecordingTransitions::new(Arc::clone(&store)));
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_max_concurrent_runs(2)
            .with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let first = submit_turn_request("thread-runner-id-a", "idem-runner-id-a");
    let second = submit_turn_request("thread-runner-id-b", "idem-runner-id-b");
    coordinator.submit_turn(first).await.unwrap();
    coordinator.submit_turn(second).await.unwrap();

    executor.wait_for_started(2).await;
    let runner_ids = transitions.claim_runner_ids();
    assert!(
        runner_ids.len() >= 2,
        "scheduler should record at least two claim attempts"
    );
    assert!(
        runner_ids
            .iter()
            .all(|runner_id| *runner_id == runner_ids[0]),
        "one scheduler instance should use one stable TurnRunnerId across claims"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn production_services_scheduler_and_coordinator_execute_turn_end_to_end() {
    let store = Arc::new(DurableLikeTurnStore::default());
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state_and_transition_port(Arc::clone(&store));
    let executor = Arc::new(CompletingExecutor::default());
    let executor_port: Arc<dyn TurnRunExecutor> = executor.clone();
    let scheduler = services
        .turn_scheduler_for_production(
            executor_port,
            fast_config().with_poll_interval(Duration::from_secs(60)),
        )
        .expect("production scheduler should build from verified turn store");
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-production-e2e", "idem-production-e2e");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started(1).await;
    wait_for_status(store.as_ref(), scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn scheduler_completes_multiple_submitted_threads_end_to_end() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config()
            .with_max_concurrent_runs(2)
            .with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let first = submit_turn_request("thread-multi-a", "idem-multi-a");
    let first_scope = first.scope.clone();
    let first_run = accepted_run_id(coordinator.submit_turn(first).await.unwrap());
    let second = submit_turn_request("thread-multi-b", "idem-multi-b");
    let second_scope = second.scope.clone();
    let second_run = accepted_run_id(coordinator.submit_turn(second).await.unwrap());

    executor.wait_for_started(2).await;
    wait_for_status(&*store, first_scope, first_run, TurnStatus::Completed).await;
    wait_for_status(&*store, second_scope, second_run, TurnStatus::Completed).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn fake_wake_without_queued_run_does_not_execute() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();

    handle
        .wake_notifier()
        .notify_queued_run(fake_wake("thread-fake-wake"))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        executor.started_count(),
        0,
        "scheduler must not execute directly from wake payload without a claimed queued run"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn stale_wake_after_completion_does_not_reexecute_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-stale-wake", "idem-stale-wake");
    let scope = request.scope.clone();
    let response = coordinator.submit_turn(request).await.unwrap();
    let run_id = accepted_run_id(response.clone());
    let stale_wake = wake_from_response(scope.clone(), &response);

    executor.wait_for_started(1).await;
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle
        .wake_notifier()
        .notify_queued_run(stale_wake)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        executor.started_count(),
        1,
        "stale wake for completed run must not re-execute work"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn wake_notifier_reports_delivery_unavailable_after_scheduler_shutdown() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(Arc::clone(&transitions), executor, fast_config());
    let handle = scheduler.start();
    let notifier = handle.wake_notifier();

    handle.shutdown().await;

    assert_eq!(
        notifier.notify_queued_run(fake_wake("thread-after-shutdown")),
        Err(TurnRunWakeNotifyError::DeliveryUnavailable)
    );
}

#[tokio::test]
async fn claim_errors_coalesce_wakes_during_backoff() {
    let transitions = Arc::new(FailingClaimTransitions::default());
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor,
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_lease_recovery_interval(Duration::from_secs(60))
            .with_claim_error_backoff(Duration::from_millis(200)),
    );
    let handle = scheduler.start();

    transitions.wait_for_claim_attempts(1).await;
    for _ in 0..8 {
        handle
            .wake_notifier()
            .notify_queued_run(TurnRunWake {
                scope: scope("thread-claim-backoff"),
                run_id: ironclaw_turns::TurnRunId::new(),
                status: TurnStatus::Queued,
                event_cursor: ironclaw_turns::EventCursor::default(),
            })
            .unwrap();
    }

    assert!(
        !transitions
            .wait_for_claim_attempts_for(2, Duration::from_millis(50))
            .await,
        "wake storm retried claims before claim_error_backoff elapsed"
    );
    transitions.wait_for_claim_attempts(2).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        transitions.claim_attempts(),
        2,
        "claim retries should be coalesced while one backoff retry is pending"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn wake_notifier_drains_submitted_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-wake", "idem-wake");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started(1).await;
    assert_eq!(
        store
            .get_run_state(ironclaw_turns::GetRunStateRequest { scope, run_id })
            .await
            .unwrap()
            .status,
        TurnStatus::Completed
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn duplicate_wakes_claim_run_once() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler =
        TurnRunScheduler::new(Arc::clone(&transitions), executor.clone(), fast_config());
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-duplicate", "idem-duplicate");
    let scope = request.scope.clone();
    let response = coordinator.submit_turn(request).await.unwrap();
    let wake = wake_from_response(scope, &response);
    handle.wake_notifier().notify_queued_run(wake).unwrap();

    executor.wait_for_started(1).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(executor.started_count(), 1);
    gate.notify_waiters();
    executor.wait_for_started(1).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn shutdown_aborts_in_flight_executor_tasks() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(HangingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    coordinator
        .submit_turn(submit_turn_request("thread-shutdown", "idem-shutdown"))
        .await
        .unwrap();
    executor.wait_for_started().await;

    timeout(Duration::from_secs(2), handle.shutdown())
        .await
        .expect("scheduler shutdown should not detach hanging executor tasks");
}

#[tokio::test]
async fn poller_recovers_queued_run_after_missed_wake() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler =
        TurnRunScheduler::new(Arc::clone(&transitions), executor.clone(), fast_config());
    let handle = scheduler.start();
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_wake_notifier(Arc::new(NoopTurnRunWakeNotifier));

    coordinator
        .submit_turn(submit_turn_request("thread-poll", "idem-poll"))
        .await
        .unwrap();

    executor.wait_for_started(1).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn executor_completion_rearms_drain_without_waiting_for_poll() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config()
            .with_max_concurrent_runs(1)
            .with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_wake_notifier(Arc::new(NoopTurnRunWakeNotifier));

    let first = coordinator
        .submit_turn(submit_turn_request("thread-rearm-a", "idem-rearm-a"))
        .await
        .unwrap();
    coordinator
        .submit_turn(submit_turn_request("thread-rearm-b", "idem-rearm-b"))
        .await
        .unwrap();
    handle
        .wake_notifier()
        .notify_queued_run(wake_from_response(scope("thread-rearm-a"), &first))
        .unwrap();

    executor.wait_for_started(1).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(executor.started_count(), 1);
    gate.notify_waiters();
    executor.wait_for_started(2).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn executor_error_records_recovery_required_instead_of_retrying() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(FailingExecutor::default());
    let scheduler =
        TurnRunScheduler::new(Arc::clone(&transitions), executor.clone(), fast_config());
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-error", "idem-error");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started().await;
    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(ironclaw_turns::GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::RecoveryRequired {
                assert_eq!(
                    state.failure.as_ref().map(|failure| failure.category()),
                    Some("scheduler_test_error")
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not move to recovery_required");
    handle.shutdown().await;
}

#[tokio::test]
async fn executor_panic_records_recovery_required() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(PanickingExecutor::default());
    let scheduler =
        TurnRunScheduler::new(Arc::clone(&transitions), executor.clone(), fast_config());
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-panic", "idem-panic");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started().await;
    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(ironclaw_turns::GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::RecoveryRequired {
                assert_eq!(
                    state.failure.as_ref().map(|failure| failure.category()),
                    Some("scheduler_executor_panic")
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not move to recovery_required after executor panic");
    handle.shutdown().await;
}

#[tokio::test]
async fn scheduler_heartbeats_long_running_executor_until_completion() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(40),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let transitions = Arc::new(HeartbeatTrackingTransitions::new(Arc::clone(&store)));
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_runner_heartbeat_interval(Duration::from_millis(5)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-heartbeat", "idem-heartbeat");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started(1).await;
    transitions.wait_for_heartbeats(2).await;
    gate.notify_waiters();
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn canceled_hanging_executor_lease_expires_to_recovery_required() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(40),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(HangingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_runner_heartbeat_interval(Duration::from_millis(5)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-cancel-recovery", "idem-cancel-recovery");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());
    executor.wait_for_started().await;
    coordinator
        .cancel_run(CancelRunRequest {
            scope: scope.clone(),
            actor: TurnActor::new(UserId::new("user1").unwrap()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-request").unwrap(),
        })
        .await
        .unwrap();

    wait_for_status(&*store, scope, run_id, TurnStatus::RecoveryRequired).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn expired_lease_reconciler_marks_running_run_recovery_required() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(-1),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor,
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_wake_notifier(Arc::new(NoopTurnRunWakeNotifier));

    let request = submit_turn_request("thread-expired", "idem-expired");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: ironclaw_turns::TurnRunnerId::new(),
            lease_token: ironclaw_turns::TurnLeaseToken::new(),
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();

    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(ironclaw_turns::GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::RecoveryRequired {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expired lease was not reconciled");
    handle.shutdown().await;
}

fn fast_config() -> TurnRunSchedulerConfig {
    TurnRunSchedulerConfig::default()
        .with_poll_interval(Duration::from_millis(10))
        .with_lease_recovery_interval(Duration::from_millis(10))
        .with_claim_error_backoff(Duration::from_millis(10))
        .with_wake_channel_capacity(16)
}

async fn wait_for_status<S>(
    store: &S,
    scope: TurnScope,
    run_id: ironclaw_turns::TurnRunId,
    expected: TurnStatus,
) where
    S: TurnStateStore + ?Sized,
{
    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(ironclaw_turns::GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not reach expected status");
}

fn submit_turn_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: scope(thread),
        actor: TurnActor::new(UserId::new("user1").unwrap()),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc::now(),
    }
}

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn accepted_run_id(response: SubmitTurnResponse) -> ironclaw_turns::TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    run_id
}

fn fake_wake(thread: &str) -> TurnRunWake {
    TurnRunWake {
        scope: scope(thread),
        run_id: ironclaw_turns::TurnRunId::new(),
        status: TurnStatus::Queued,
        event_cursor: ironclaw_turns::EventCursor::default(),
    }
}

fn wake_from_response(scope: TurnScope, response: &SubmitTurnResponse) -> TurnRunWake {
    let SubmitTurnResponse::Accepted {
        run_id,
        status,
        event_cursor,
        ..
    } = response;
    TurnRunWake {
        scope,
        run_id: *run_id,
        status: *status,
        event_cursor: *event_cursor,
    }
}
