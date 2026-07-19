// arch-exempt: large_file, mechanical LocalFilesystem->DiskFilesystem Bucket-2 rename (arch-simplification §4.4), no logic change, plan #6168
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
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntimeServices, ProductionWiringComponent,
    ProductionWiringIssueKind,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_runner::turn_scheduler::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator,
    GetRunStateRequest, IdempotencyKey, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    NoopTurnRunWakeNotifier, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileRequest, SanitizedCancelReason, SourceBindingRef, SpawnTreeReservation,
    SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnRunRecord, TurnRunState, TurnRunWake, TurnRunWakeNotifier,
    TurnRunWakeNotifyError, TurnRunnerId, TurnScope, TurnSpawnTreeStateStore, TurnStateStore,
    TurnStatus,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
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

    async fn record_model_route_snapshot(
        &self,
        _request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not record model route snapshots")
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

    async fn record_runner_failure(
        &self,
        _request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("failing claim transitions should not record terminal failure")
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

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        self.inner.retry_turn(request).await
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
impl TurnSpawnTreeStateStore for DurableLikeTurnStore {
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner
            .submit_child_turn(request, admission_policy, run_profile_resolver)
            .await
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        self.inner.children_of(scope, run_id).await
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        self.inner.get_run_record(scope, run_id).await
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        self.inner
            .reserve_tree_descendants(scope, root_run_id, delta, cap)
            .await
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        self.inner
            .release_tree_descendants(scope, root_run_id, delta, idempotency_key)
            .await
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.inner
            .prune_released_child(scope, root_run_id, child_run_id)
            .await
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

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_model_route_snapshot(request).await
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

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_runner_failure(request).await
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

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        // WS-3 implements this.
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
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

    async fn record_model_route_snapshot(
        &self,
        _request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not record model route snapshots")
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

    async fn record_runner_failure(
        &self,
        _request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("transition stub should not record terminal failure")
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
    heartbeat_attempts: AtomicUsize,
    heartbeats: AtomicUsize,
    notify_heartbeat_attempt: Notify,
    notify_heartbeat: Notify,
    heartbeat_delay: Mutex<Option<Duration>>,
    one_shot_heartbeat_delay: Mutex<Option<Duration>>,
}

struct ClaimRecordingTransitions {
    store: Arc<InMemoryTurnStateStore>,
    claim_runner_ids: Mutex<Vec<TurnRunnerId>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChaosOperation {
    ClaimNextRun,
    Heartbeat,
    RecoverExpiredLeases,
    RecordModelRouteSnapshot,
    BlockRun,
    CompleteRun,
    CancelRun,
    FailRun,
    RecordRunnerFailure,
    RelinquishRun,
    ApplyValidatedLoopExit,
}

struct ChaosRule {
    operation: ChaosOperation,
    remaining_drops: usize,
}

struct ChaosTransitionPort {
    store: Arc<InMemoryTurnStateStore>,
    rules: Mutex<Vec<ChaosRule>>,
    calls: Mutex<Vec<ChaosOperation>>,
    hold_after_relinquish: Mutex<Option<Arc<Notify>>>,
}

impl HeartbeatTrackingTransitions {
    fn new(store: Arc<InMemoryTurnStateStore>) -> Self {
        Self {
            store,
            heartbeat_attempts: AtomicUsize::new(0),
            heartbeats: AtomicUsize::new(0),
            notify_heartbeat_attempt: Notify::new(),
            notify_heartbeat: Notify::new(),
            heartbeat_delay: Mutex::new(None),
            one_shot_heartbeat_delay: Mutex::new(None),
        }
    }

    fn with_heartbeat_delay(self, delay: Duration) -> Self {
        *self.heartbeat_delay.lock().unwrap() = Some(delay);
        self
    }

    fn with_one_shot_heartbeat_delay(self, delay: Duration) -> Self {
        *self.one_shot_heartbeat_delay.lock().unwrap() = Some(delay);
        self
    }

    async fn wait_for_heartbeat_attempts(&self, expected: usize) {
        timeout(Duration::from_secs(2), async {
            loop {
                let notified = self.notify_heartbeat_attempt.notified();
                if self.heartbeat_attempts.load(Ordering::SeqCst) >= expected {
                    return;
                }
                notified.await;
            }
        })
        .await
        .expect("scheduler did not attempt expected heartbeats");
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

impl ChaosTransitionPort {
    fn new(store: Arc<InMemoryTurnStateStore>) -> Self {
        Self {
            store,
            rules: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            hold_after_relinquish: Mutex::new(None),
        }
    }

    fn drop_next(self, operation: ChaosOperation) -> Self {
        self.drop_next_n(operation, 1)
    }

    fn drop_next_n(self, operation: ChaosOperation, count: usize) -> Self {
        self.rules.lock().unwrap().push(ChaosRule {
            operation,
            remaining_drops: count,
        });
        self
    }

    fn call_count(&self, operation: ChaosOperation) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|recorded| **recorded == operation)
            .count()
    }

    fn hold_after_relinquish(self, gate: Arc<Notify>) -> Self {
        *self.hold_after_relinquish.lock().unwrap() = Some(gate);
        self
    }

    async fn wait_for_call_count(&self, operation: ChaosOperation, expected: usize) {
        timeout(Duration::from_secs(2), async {
            loop {
                if self.call_count(operation) >= expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "chaos transition port did not observe {expected} calls for {operation:?}; saw {}",
                self.call_count(operation)
            )
        });
    }

    fn maybe_drop(&self, operation: ChaosOperation) -> Result<(), TurnError> {
        self.calls.lock().unwrap().push(operation);
        let mut rules = self.rules.lock().unwrap();
        if let Some(rule) = rules
            .iter_mut()
            .find(|rule| rule.operation == operation && rule.remaining_drops > 0)
        {
            rule.remaining_drops -= 1;
            return Err(TurnError::Unavailable {
                reason: format!("chaos monkey dropped {operation:?}"),
            });
        }
        Ok(())
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
        self.heartbeat_attempts.fetch_add(1, Ordering::SeqCst);
        self.notify_heartbeat_attempt.notify_waiters();
        let delay = self
            .one_shot_heartbeat_delay
            .lock()
            .unwrap()
            .take()
            .or_else(|| *self.heartbeat_delay.lock().unwrap());
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
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

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_model_route_snapshot(request).await
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

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_runner_failure(request).await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.apply_validated_loop_exit(request).await
    }
}

#[async_trait]
impl TurnRunTransitionPort for ChaosTransitionPort {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.maybe_drop(ChaosOperation::ClaimNextRun)?;
        self.store.claim_next_run(request).await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        self.maybe_drop(ChaosOperation::Heartbeat)?;
        self.store.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.maybe_drop(ChaosOperation::RecoverExpiredLeases)?;
        self.store.recover_expired_leases(request).await
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::RecordModelRouteSnapshot)?;
        self.store.record_model_route_snapshot(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::BlockRun)?;
        self.store.block_run(request).await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::CompleteRun)?;
        self.store.complete_run(request).await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::CancelRun)?;
        self.store.cancel_run(request).await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::FailRun)?;
        self.store.fail_run(request).await
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::RecordRunnerFailure)?;
        self.store.record_runner_failure(request).await
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::RelinquishRun)?;
        let result = self.store.relinquish_run(request).await;
        let gate = self.hold_after_relinquish.lock().unwrap().clone();
        if result.is_ok()
            && let Some(gate) = gate
        {
            gate.notified().await;
        }
        result
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.maybe_drop(ChaosOperation::ApplyValidatedLoopExit)?;
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

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_model_route_snapshot(request).await
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

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.store.record_runner_failure(request).await
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
fn production_services_expose_verified_transition_port_without_notifier() {
    let store = Arc::new(DurableTurnStoreStub);
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state_and_transition_port(store);
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(CompletingExecutor::default());

    let transition_port = services
        .turn_run_transition_port_for_production()
        .expect("production transition port should be verified");
    let _scheduler = TurnRunScheduler::new(transition_port, executor, fast_config());
}

#[test]
fn production_services_reject_unverified_scheduler_transition_port() {
    let turn_state = Arc::new(DurableTurnStoreStub);
    let transition_port = Arc::new(DurableTurnStoreStub);
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state(turn_state)
    .with_turn_run_transition_port(transition_port);
    let result = services.turn_run_transition_port_for_production();
    let Err(report) = result else {
        panic!("production wiring should reject unverified transition port");
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_state_and_transition_port(Arc::clone(&store));
    let executor = Arc::new(CompletingExecutor::default());
    let executor_port: Arc<dyn TurnRunExecutor> = executor.clone();
    let transition_port = services
        .turn_run_transition_port_for_production()
        .expect("production transition port should be verified");
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor_port,
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
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

/// Verifies that `TurnRunScheduler` emits a "turn run started" debug event with
/// `thread_id` and `run_id` correlation fields so the operator Logs panel can
/// scope entries to a specific run.
///
/// # Why `#[traced_test]` instead of `set_default`
///
/// `tracing::dispatcher::set_default` is thread-local and subject to a global
/// `SCOPED_COUNT` fast-path race in `tracing-core`: when parallel tests
/// decrement the count to 0, spawned async tasks silently fall back to the
/// no-op global dispatcher. `#[traced_test]` registers a global subscriber
/// instead, which correctly captures events from spawned tasks regardless of
/// parallelism. The `no-env-filter` feature is required because the event
/// originates in the runner crate's `turn_scheduler` module.
#[tokio::test]
#[tracing_test::traced_test]
async fn scheduler_executor_emits_thread_run_correlated_operator_log() {
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

    let request = submit_turn_request("thread-scheduler-operator-log", "idem-scheduler-log");
    let thread_id = request.scope.thread_id.to_string();
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started(1).await;
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;

    let run_id_str = run_id.to_string();
    // `logs_assert` gives access to all captured log lines from this test.
    // We verify that at least one line contains the "turn run started" message
    // together with the expected thread_id and run_id correlation fields.
    logs_assert(|lines: &[&str]| {
        let found = lines.iter().any(|line| {
            line.contains("turn run started")
                && line.contains(&format!("thread_id={thread_id}"))
                && line.contains(&format!("run_id={run_id_str}"))
        });
        if found {
            Ok(())
        } else {
            let matching: Vec<_> = lines
                .iter()
                .filter(|l| l.contains("turn run started"))
                .collect();
            Err(format!(
                "no log line found with 'turn run started', thread_id={thread_id}, run_id={run_id_str}; \
                 lines_with_message={matching:?}"
            ))
        }
    });
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

/// Verifies that graceful scheduler shutdown relinquishes in-flight runs back
/// to Queued so a restart can retry them instead of letting lease expiry fail
/// them.
#[tokio::test]
async fn shutdown_relinquishes_in_flight_runs_to_queued() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    // HangingExecutor blocks indefinitely so the run stays Running through shutdown.
    let executor = Arc::new(HangingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-relinquish-shutdown", "idem-relinquish-shutdown");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    // Wait until the executor has actually claimed and started the run (Running).
    // Bounded so a regression in claim/start logic cannot hang the test suite.
    timeout(Duration::from_secs(3), executor.wait_for_started())
        .await
        .expect("executor did not start the run within 3s; scheduler may not have claimed the queued run");

    // Shutdown aborts the in-flight task and must relinquish the run to Queued.
    timeout(Duration::from_secs(2), handle.shutdown())
        .await
        .expect("scheduler shutdown should not hang when relinquishing in-flight runs");

    let state = store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap();
    assert_eq!(
        state.status,
        TurnStatus::Queued,
        "in-flight run must be relinquished back to Queued on shutdown, not left Running or Failed"
    );
}

/// Regression test for `Drop for TurnRunSchedulerHandle`.
///
/// When a handle is dropped WITHOUT calling `shutdown()` — for example, when a
/// build function starts the scheduler and then fails on a later fallible step —
/// the `Drop` impl must still drive `shutdown_scheduler`, which aborts every
/// in-flight executor task and relinquishes each active run back to `Queued`.
///
/// This complements `shutdown_relinquishes_in_flight_runs_to_queued` (which
/// covers the explicit `shutdown()` path) by proving the same drain happens on
/// an implicit, synchronous drop.
#[tokio::test]
async fn drop_without_shutdown_relinquishes_in_flight_run_to_queued() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    // HangingExecutor parks in `pending::<()>()` so the run stays Running
    // until the task is aborted, giving the shutdown drain a live in-flight
    // run to relinquish.
    let executor = Arc::new(HangingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-drop-relinquish", "idem-drop-relinquish");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    // Wait until the executor has actually claimed and started the run so it
    // is genuinely in-flight (Running, lease held, tracked in active_runs).
    timeout(Duration::from_secs(3), executor.wait_for_started())
        .await
        .expect("executor did not start the run within 3 s; scheduler may not have claimed it");

    // Drop the handle WITHOUT calling shutdown().
    // The Drop impl cancels the token, the background loop's `select!` picks
    // it up, calls `shutdown_scheduler`, aborts executor tasks, and relinquishes
    // each active run.
    drop(handle);

    // Give the background loop time to run the shutdown_scheduler drain.
    // Bounded so a regression cannot hang CI.
    timeout(Duration::from_secs(3), async {
        loop {
            let state = store
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::Queued {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect(
        "in-flight run must be relinquished back to Queued when handle is dropped without shutdown()",
    );
}

/// Verifies that when a coordinator mints the notifier and channel before
/// starting the scheduler via `start_with_channel`, the notifier held by the
/// coordinator and the scheduler's consuming loop share the SAME underlying
/// mpsc channel. A mismatch would leave the notified run unclaimed.
#[tokio::test]
async fn start_with_channel_shares_notifier_with_scheduler_loop() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let transitions: Arc<dyn TurnRunTransitionPort> = store.clone();
    let executor = Arc::new(CompletingExecutor::default());

    // Mint the notifier and channel BEFORE building the scheduler — this is
    // the cycle-breaking entry point the coordinator uses so it can hold the
    // notifier first.
    let cap = fast_config().wake_channel_capacity();
    let (notifier, channel) = SchedulerTurnRunWakeNotifier::channel(cap);

    let scheduler = TurnRunScheduler::new(
        Arc::clone(&transitions),
        executor.clone(),
        fast_config().with_poll_interval(Duration::from_secs(60)),
    );
    // Pass the pre-minted notifier + channel; do NOT call start() which would
    // create a fresh channel and break the identity contract under test.
    let handle = scheduler.start_with_channel(notifier.clone(), channel);

    // Submit a turn through the store directly (no coordinator) so we control
    // exactly which notifier fires the wake.
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(notifier.clone());

    let request = submit_turn_request("thread-start-with-channel", "idem-start-with-channel");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    // Fire the pre-minted notifier — the one the coordinator holds, not the
    // one inside the handle.  If the scheduler loop is consuming a different
    // channel this wake is lost and the run stays Queued indefinitely.
    timeout(Duration::from_secs(3), executor.wait_for_started(1))
        .await
        .expect("executor did not start the run within 3s; notifier and scheduler channel are mismatched");

    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;
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
async fn executor_error_fails_run_instead_of_retrying() {
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
            if state.status == TurnStatus::Failed {
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
    .expect("run did not move to failed");
    handle.shutdown().await;
}

#[tokio::test]
async fn executor_panic_fails_run() {
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
            if state.status == TurnStatus::Failed {
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
    .expect("run did not move to failed after executor panic");
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
async fn chaos_scheduler_recovers_after_transient_claim_and_heartbeat_drops() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(500),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let chaos = Arc::new(
        ChaosTransitionPort::new(Arc::clone(&store))
            .drop_next(ChaosOperation::ClaimNextRun)
            .drop_next(ChaosOperation::Heartbeat),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = chaos.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_claim_error_backoff(Duration::from_millis(1))
            .with_runner_heartbeat_interval(Duration::from_millis(5))
            .with_max_consecutive_heartbeat_failures(3),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-chaos-claim-heartbeat", "idem-chaos-claim-heartbeat");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    chaos
        .wait_for_call_count(ChaosOperation::ClaimNextRun, 2)
        .await;
    executor.wait_for_started(1).await;
    chaos
        .wait_for_call_count(ChaosOperation::Heartbeat, 2)
        .await;
    gate.notify_waiters();
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;

    assert!(chaos.call_count(ChaosOperation::ClaimNextRun) >= 2);
    assert!(chaos.call_count(ChaosOperation::Heartbeat) >= 2);
}

#[tokio::test]
async fn scheduler_tolerates_transient_heartbeat_timeout_until_executor_completes() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(500),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let transitions = Arc::new(
        HeartbeatTrackingTransitions::new(Arc::clone(&store))
            .with_one_shot_heartbeat_delay(Duration::from_secs(60)),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_runner_heartbeat_interval(Duration::from_millis(5))
            .with_max_consecutive_heartbeat_failures(2),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request(
        "thread-heartbeat-transient-timeout",
        "idem-heartbeat-transient-timeout",
    );
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());
    executor.wait_for_started(1).await;

    transitions.wait_for_heartbeats(1).await;
    gate.notify_waiters();
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn chaos_scheduler_retries_terminal_failure_recording_after_transient_store_drop() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let chaos = Arc::new(
        ChaosTransitionPort::new(Arc::clone(&store)).drop_next(ChaosOperation::RecordRunnerFailure),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = chaos.clone();
    let executor = Arc::new(FailingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_terminal_failure_record_attempts(3)
            .with_terminal_failure_record_backoff(Duration::from_millis(1)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request(
        "thread-chaos-terminal-recording",
        "idem-chaos-terminal-recording",
    );
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started().await;
    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::Failed {
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
    .expect("run did not persist failed state after transient terminal-recording drop");
    handle.shutdown().await;

    assert_eq!(chaos.call_count(ChaosOperation::RecordRunnerFailure), 2);
}

#[tokio::test]
async fn chaos_scheduler_relinquishes_run_when_terminal_failure_recording_is_exhausted() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let relinquish_gate = Arc::new(Notify::new());
    let chaos = Arc::new(
        ChaosTransitionPort::new(Arc::clone(&store))
            .drop_next_n(ChaosOperation::RecordRunnerFailure, 3)
            .hold_after_relinquish(Arc::clone(&relinquish_gate)),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = chaos.clone();
    let executor = Arc::new(FailingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_terminal_failure_record_attempts(3)
            .with_terminal_failure_record_backoff(Duration::from_millis(1)),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request(
        "thread-chaos-terminal-recording-exhausted",
        "idem-chaos-terminal-recording-exhausted",
    );
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());

    executor.wait_for_started().await;
    wait_for_status(&*store, scope, run_id, TurnStatus::Queued).await;
    assert_eq!(chaos.call_count(ChaosOperation::RecordRunnerFailure), 3);
    assert_eq!(chaos.call_count(ChaosOperation::RelinquishRun), 1);

    relinquish_gate.notify_one();
    handle.shutdown().await;
}

#[tokio::test]
async fn scheduler_keeps_executor_alive_during_sustained_heartbeat_timeouts() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(500),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let transitions = Arc::new(
        HeartbeatTrackingTransitions::new(Arc::clone(&store))
            .with_heartbeat_delay(Duration::from_secs(60)),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions.clone();
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(CompletingExecutor::with_gate(Arc::clone(&gate)));
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_runner_heartbeat_interval(Duration::from_millis(10))
            .with_max_consecutive_heartbeat_failures(2),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-heartbeat-timeout", "idem-heartbeat-timeout");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());
    executor.wait_for_started(1).await;
    transitions.wait_for_heartbeat_attempts(3).await;

    let state = store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Running);
    gate.notify_waiters();
    wait_for_status(&*store, scope, run_id, TurnStatus::Completed).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn scheduler_records_failure_after_repeated_heartbeat_errors() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            runner_lease_ttl: ChronoDuration::milliseconds(500),
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let chaos = Arc::new(
        ChaosTransitionPort::new(Arc::clone(&store)).drop_next_n(ChaosOperation::Heartbeat, 2),
    );
    let transition_port: Arc<dyn TurnRunTransitionPort> = chaos.clone();
    let executor = Arc::new(HangingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor.clone(),
        fast_config()
            .with_poll_interval(Duration::from_secs(60))
            .with_runner_heartbeat_interval(Duration::from_millis(10))
            .with_max_consecutive_heartbeat_failures(2),
    );
    let handle = scheduler.start();
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(handle.wake_notifier());

    let request = submit_turn_request("thread-heartbeat-error", "idem-heartbeat-error");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(coordinator.submit_turn(request).await.unwrap());
    executor.wait_for_started().await;

    timeout(Duration::from_secs(2), async {
        loop {
            let state = store
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .unwrap();
            if state.status == TurnStatus::Failed {
                assert_eq!(
                    state.failure.as_ref().map(|failure| failure.category()),
                    Some("scheduler_heartbeat_failed")
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not move to failed after heartbeat errors");
    handle.shutdown().await;
}

#[tokio::test]
async fn canceled_hanging_executor_lease_expires_to_cancelled() {
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

    let request = submit_turn_request("thread-cancel-terminal", "idem-cancel-terminal");
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

    wait_for_status(&*store, scope, run_id, TurnStatus::Cancelled).await;
    handle.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn expired_lease_reconciler_fails_running_run() {
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
    let thread_id = request.scope.thread_id.to_string();
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

    let failed_state = wait_for_status(&*store, scope, run_id, TurnStatus::Failed).await;
    assert_eq!(
        failed_state
            .failure
            .as_ref()
            .map(|failure| failure.category()),
        Some("lease_expired")
    );
    handle.shutdown().await;

    let run_id_str = run_id.to_string();
    logs_assert(|lines: &[&str]| {
        let found = lines.iter().any(|line| {
            line.contains("turn run scheduler recovered expired lease")
                && line.contains(&format!("thread_id={thread_id}"))
                && line.contains(&format!("run_id={run_id_str}"))
                && line.contains("failure_category=lease_expired")
        });
        if found {
            Ok(())
        } else {
            let matching: Vec<_> = lines
                .iter()
                .filter(|line| line.contains("turn run scheduler recovered expired lease"))
                .collect();
            Err(format!(
                "no expired lease recovery log found with thread_id={thread_id}, run_id={run_id_str}; \
                 lines_with_message={matching:?}"
            ))
        }
    });
}

/// A `TurnRunTransitionPort` whose `claim_next_run` blocks until a `Notify` is
/// signalled, then returns `Ok(None)`.  Used to simulate a slow store claim so
/// that we can verify the cancellation / drain-exit behaviour.
struct BarrierClaimTransitions {
    /// Fired by the test when `claim_next_run` should return.
    release: Arc<tokio::sync::Notify>,
    /// Fired by `claim_next_run` once it has entered its blocking await so the
    /// test knows the drain loop is actually parked inside a claim.
    entered: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl TurnRunTransitionPort for BarrierClaimTransitions {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.entered.notify_waiters();
        self.release.notified().await;
        Ok(None)
    }

    async fn heartbeat(
        &self,
        _request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        panic!("barrier claim transitions should not heartbeat")
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        Ok(RecoverExpiredLeasesResponse { recovered: vec![] })
    }

    async fn record_model_route_snapshot(
        &self,
        _request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not record model route snapshots")
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not block runs")
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not complete runs")
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not cancel runs")
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not fail runs")
    }

    async fn record_runner_failure(
        &self,
        _request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not record terminal failure")
    }

    async fn apply_validated_loop_exit(
        &self,
        _request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("barrier claim transitions should not apply loop exits")
    }
}

/// Verifies that scheduler shutdown does not start new `claim_next_run` calls
/// after cancellation fires, and exits promptly once the in-flight claim returns.
///
/// Shape chosen (FIX 1): cancellation is checked at the TOP of each drain
/// iteration, BEFORE starting a new `claim_next_run`.  This means:
///  - Any claim already in progress runs to completion so its result is always
///    tracked in `active_runs` (no leaked claimed-but-untracked run).
///  - Once that claim returns, the top-of-loop check fires → drain exits without
///    issuing any further claim calls.
///  - The outer scheduler select re-enters and observes `cancelled()`, calling
///    `shutdown_scheduler`.
///
/// The test drives this by:
///  1. Sending a Wake to start a drain → `claim_next_run` blocks on a barrier.
///  2. Waiting for the drain to be inside the claim await (`entered` notify).
///  3. Calling `shutdown()` to cancel the token.
///  4. Releasing the barrier → claim returns `Ok(None)`.
///  5. Asserting `shutdown()` completes within a short bound after the release.
#[tokio::test]
async fn shutdown_completes_promptly_after_stalled_claim_unblocks() {
    let release = Arc::new(tokio::sync::Notify::new());
    let entered = Arc::new(tokio::sync::Notify::new());
    let transitions = Arc::new(BarrierClaimTransitions {
        release: Arc::clone(&release),
        entered: Arc::clone(&entered),
    });
    let transition_port: Arc<dyn TurnRunTransitionPort> = transitions;
    let executor = Arc::new(CompletingExecutor::default());
    let scheduler = TurnRunScheduler::new(
        transition_port,
        executor,
        fast_config()
            // Long poll interval so only the explicit Wake triggers a drain.
            .with_poll_interval(Duration::from_secs(3600))
            .with_lease_recovery_interval(Duration::from_secs(3600)),
    );
    let handle = scheduler.start();

    // Trigger a drain by sending a Wake; the barrier claim will block.
    handle
        .wake_notifier()
        .notify_queued_run(fake_wake("thread-stalled-claim"))
        .unwrap();

    // Wait until the scheduler is actually parked inside claim_next_run.
    timeout(Duration::from_secs(2), entered.notified())
        .await
        .expect("scheduler did not enter claim_next_run within 2 s");

    // Initiate shutdown in the background — the token is cancelled immediately,
    // but the loop is stuck awaiting the in-flight claim.
    let shutdown_future = tokio::spawn(async move { handle.shutdown().await });

    // Release the barrier — claim_next_run returns Ok(None).
    // The top-of-loop cancellation check fires on the next iteration and the
    // drain returns; the outer loop observes cancelled() and calls shutdown_scheduler.
    release.notify_waiters();

    // Shutdown must complete promptly (well within 2 s) after we release the barrier.
    timeout(Duration::from_secs(2), shutdown_future)
        .await
        .expect("scheduler shutdown must complete promptly after stalled claim unblocks")
        .expect("shutdown task must not panic");
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
) -> TurnRunState
where
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
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not reach expected status")
}

fn submit_turn_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        requested_model: None,
        scope: scope(thread),
        actor: TurnActor::new(UserId::new("user1").unwrap()),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
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
