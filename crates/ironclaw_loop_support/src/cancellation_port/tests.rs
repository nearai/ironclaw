use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::run_profile::{LoopCancelReasonKind, LoopCancellationPort};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest,
    ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileResolver,
    RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnAdmissionPolicy, TurnError, TurnId, TurnRunId, TurnRunState, TurnRunWake,
    TurnRunWakeNotifier, TurnScope, TurnStateStore, TurnStatus, run_profile::AgentLoopHostError,
};

use super::{
    AlwaysAliveLoopCancellationPort, AlwaysAliveRunCancellationFactory, RunCancellationFactory,
    RunCancellationHandle, RunCancellationObservationKind, RunStateLoopCancellationPort,
    TurnStateRunCancellationFactory,
};

struct TestLiveCancellationFactory;

#[async_trait]
impl RunCancellationFactory for TestLiveCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        Ok(RunCancellationHandle::default())
    }
}

struct StaticTurnStateStore {
    state: TurnRunState,
}

impl StaticTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self { state }
    }
}

struct CountingTurnStateStore {
    state: TurnRunState,
    get_run_state_calls: AtomicUsize,
}

impl CountingTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self {
            state,
            get_run_state_calls: AtomicUsize::new(0),
        }
    }

    fn get_run_state_calls(&self) -> usize {
        self.get_run_state_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TurnStateStore for CountingTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by cancellation factory tests")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by cancellation factory tests")
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by cancellation factory tests")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        assert_eq!(request.scope, self.state.scope);
        assert_eq!(request.run_id, self.state.run_id);
        self.get_run_state_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.state.clone())
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by cancellation factory tests")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by cancellation factory tests")
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
        panic!("request_cancel should not be called by cancellation factory tests")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        assert_eq!(request.scope, self.state.scope);
        assert_eq!(request.run_id, self.state.run_id);
        Ok(self.state.clone())
    }
}

fn test_run_state(status: TurnStatus) -> TurnRunState {
    let tenant_id = TenantId::new("tenant-cancel-factory").unwrap();
    let agent_id = AgentId::new("agent-cancel-factory").unwrap();
    let project_id = ProjectId::new("project-cancel-factory").unwrap();
    let thread_id = ThreadId::new("thread-cancel-factory").unwrap();
    TurnRunState {
        scope: TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id),
        actor: Some(TurnActor::new(UserId::new("user-cancel-factory").unwrap())),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status,
        accepted_message_ref: AcceptedMessageRef::new("accepted-cancel-factory").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-cancel-factory").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-cancel-factory").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: None,
        resume_disposition: None,
    }
}

#[test]
fn observe_returns_none_when_not_requested() {
    let port = RunStateLoopCancellationPort::new(RunCancellationHandle::default());

    assert_eq!(port.observe_cancellation(), None);
}

#[test]
fn observe_returns_signal_after_flip() {
    let handle = RunCancellationHandle::default();
    let port = RunStateLoopCancellationPort::new(handle.clone());

    handle.request(LoopCancelReasonKind::UserRequested);

    let signal = port.observe_cancellation().expect("signal");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
    assert!(handle.is_requested());
}

#[test]
fn observe_idempotent_after_first_read() {
    let handle = RunCancellationHandle::default();
    let port = RunStateLoopCancellationPort::new(handle.clone());

    handle.request(LoopCancelReasonKind::Superseded);

    let first = port.observe_cancellation();
    assert_eq!(port.observe_cancellation(), first);
    assert_eq!(port.observe_cancellation(), first);
}

#[test]
fn duplicate_request_preserves_first_signal() {
    let handle = RunCancellationHandle::default();
    let port = RunStateLoopCancellationPort::new(handle.clone());

    handle.request(LoopCancelReasonKind::UserRequested);
    let first = port.observe_cancellation().expect("first signal");

    handle.request(LoopCancelReasonKind::Policy);

    let second = port.observe_cancellation().expect("second signal");
    assert_eq!(second, first);
    assert_eq!(second.reason_kind, LoopCancelReasonKind::UserRequested);
}

#[tokio::test]
async fn cancellation_requested_returns_immediately_after_flip() {
    let handle = RunCancellationHandle::default();
    let port = RunStateLoopCancellationPort::new(handle.clone());
    handle.request(LoopCancelReasonKind::UserRequested);

    let signal = tokio::time::timeout(Duration::from_millis(50), port.cancellation_requested())
        .await
        .expect("already-requested cancellation should not wait");

    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
}

#[tokio::test]
async fn cancellation_requested_wakes_waiter_after_request() {
    let handle = RunCancellationHandle::default();
    let port = Arc::new(RunStateLoopCancellationPort::new(handle.clone()));
    let waiter = Arc::clone(&port);
    let join = tokio::spawn(async move { waiter.cancellation_requested().await });

    tokio::time::sleep(Duration::from_millis(5)).await;
    handle.request(LoopCancelReasonKind::Superseded);

    let signal = tokio::time::timeout(Duration::from_millis(100), join)
        .await
        .expect("waiter should be notified")
        .expect("waiter task should complete");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::Superseded);
}

#[test]
fn observe_payload_includes_requested_at() {
    let handle = RunCancellationHandle::default();
    let port = RunStateLoopCancellationPort::new(handle.clone());
    let before = Utc::now();

    handle.request(LoopCancelReasonKind::Policy);

    let after = Utc::now();
    let signal = port.observe_cancellation().expect("signal");
    assert!(signal.requested_at >= before);
    assert!(signal.requested_at <= after + chrono::Duration::seconds(5));
}

#[test]
fn handle_signal_visible_after_atomic_load() {
    let handle = RunCancellationHandle::default();
    let port = Arc::new(RunStateLoopCancellationPort::new(handle.clone()));
    let observer = Arc::clone(&port);

    let join = std::thread::spawn(move || {
        for _ in 0..10_000 {
            if let Some(signal) = observer.observe_cancellation() {
                return signal;
            }
            std::thread::sleep(Duration::from_micros(50));
        }
        panic!("observer did not see cancellation signal");
    });

    handle.request(LoopCancelReasonKind::UserRequested);

    let signal = join.join().expect("observer thread");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
}

#[test]
fn always_alive_port_returns_none() {
    let port = AlwaysAliveLoopCancellationPort;

    assert_eq!(port.observe_cancellation(), None);
}

#[tokio::test]
async fn always_alive_cancellation_requested_never_resolves() {
    let port = AlwaysAliveLoopCancellationPort;

    let result =
        tokio::time::timeout(Duration::from_millis(10), port.cancellation_requested()).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn always_alive_factory_is_identified_as_inert_fallback() {
    let factory = AlwaysAliveRunCancellationFactory;

    assert_eq!(
        factory.observation_kind(),
        RunCancellationObservationKind::InertFallback
    );
    assert!(!factory.observation_kind().is_live_capable());

    let state = test_run_state(TurnStatus::Running);
    let handle = factory
        .handle_for_run(&state.scope, TurnRunId::new())
        .await
        .unwrap();
    assert!(!handle.is_requested());
}

#[tokio::test]
async fn turn_state_factory_seeds_already_cancel_requested_run() {
    let state = test_run_state(TurnStatus::CancelRequested);
    let factory =
        TurnStateRunCancellationFactory::new(Arc::new(StaticTurnStateStore::new(state.clone())));

    let handle = factory
        .handle_for_run(&state.scope, state.run_id)
        .await
        .unwrap();

    assert!(handle.is_requested());
    let port = RunStateLoopCancellationPort::new(handle);
    let signal = port.observe_cancellation().expect("cancel signal");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
}

#[tokio::test]
async fn turn_state_factory_seeds_claimed_cancel_requested_run_without_store_read() {
    let state = test_run_state(TurnStatus::CancelRequested);
    let store = Arc::new(CountingTurnStateStore::new(state.clone()));
    let factory = TurnStateRunCancellationFactory::new(store.clone());

    let handle = factory.handle_for_claimed_run(&state).await.unwrap();

    assert!(handle.is_requested());
    assert_eq!(store.get_run_state_calls(), 0);
    assert_eq!(factory.registered_run_count(), 0);
    let port = RunStateLoopCancellationPort::new(handle);
    let signal = port.observe_cancellation().expect("cancel signal");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
}

#[tokio::test]
async fn turn_state_factory_claimed_running_run_skips_store_read_and_registers() {
    let state = test_run_state(TurnStatus::Running);
    let store = Arc::new(CountingTurnStateStore::new(state.clone()));
    let factory = TurnStateRunCancellationFactory::new(store.clone())
        .with_poll_interval(Duration::from_secs(60));

    let handle = factory.handle_for_claimed_run(&state).await.unwrap();

    assert!(!handle.is_requested());
    assert_eq!(store.get_run_state_calls(), 0);
    assert_eq!(factory.registered_run_count(), 1);
}

#[tokio::test]
async fn turn_state_factory_flips_registered_handle_from_cancel_wake() {
    let state = test_run_state(TurnStatus::Running);
    let factory =
        TurnStateRunCancellationFactory::new(Arc::new(StaticTurnStateStore::new(state.clone())))
            .with_poll_interval(Duration::from_secs(60));
    let handle = factory
        .handle_for_run(&state.scope, state.run_id)
        .await
        .unwrap();
    assert!(!handle.is_requested());

    factory
        .notify_queued_run(TurnRunWake {
            scope: state.scope,
            run_id: state.run_id,
            status: TurnStatus::CancelRequested,
            event_cursor: EventCursor(2),
        })
        .unwrap();

    assert!(handle.is_requested());
}

#[tokio::test]
async fn turn_state_factory_reads_run_state_once_when_registering_running_run() {
    let state = test_run_state(TurnStatus::Running);
    let store = Arc::new(CountingTurnStateStore::new(state.clone()));
    let factory = TurnStateRunCancellationFactory::new(store.clone())
        .with_poll_interval(Duration::from_secs(60));

    let handle = factory
        .handle_for_run(&state.scope, state.run_id)
        .await
        .unwrap();

    assert!(!handle.is_requested());
    assert_eq!(store.get_run_state_calls(), 1);
    assert_eq!(factory.registered_run_count(), 1);
}

#[tokio::test]
async fn turn_state_factory_prunes_run_after_cancel_wake() {
    let state = test_run_state(TurnStatus::Running);
    let factory =
        TurnStateRunCancellationFactory::new(Arc::new(StaticTurnStateStore::new(state.clone())))
            .with_poll_interval(Duration::from_secs(60));
    let _handle = factory
        .handle_for_run(&state.scope, state.run_id)
        .await
        .unwrap();
    assert_eq!(factory.registered_run_count(), 1);

    factory
        .notify_queued_run(TurnRunWake {
            scope: state.scope,
            run_id: state.run_id,
            status: TurnStatus::CancelRequested,
            event_cursor: EventCursor(2),
        })
        .unwrap();

    assert_eq!(factory.registered_run_count(), 0);
}

struct MutableTurnStateStore {
    state: std::sync::Mutex<TurnRunState>,
}

impl MutableTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self {
            state: std::sync::Mutex::new(state),
        }
    }

    fn set_status(&self, status: TurnStatus) {
        self.state.lock().unwrap().status = status;
    }
}

#[async_trait]
impl TurnStateStore for MutableTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by cancellation factory tests")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by cancellation factory tests")
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
        panic!("request_cancel should not be called by cancellation factory tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(self.state.lock().unwrap().clone())
    }
}

#[tokio::test]
async fn turn_state_factory_polling_fallback_fires_without_wake() {
    let initial = test_run_state(TurnStatus::Running);
    let store = Arc::new(MutableTurnStateStore::new(initial.clone()));
    let factory = TurnStateRunCancellationFactory::new(store.clone())
        .with_poll_interval(Duration::from_millis(5));
    let handle = factory
        .handle_for_run(&initial.scope, initial.run_id)
        .await
        .unwrap();
    assert!(!handle.is_requested());

    // Transition durable state without dispatching a wake — only the
    // polling-fallback task can discover the flip.
    store.set_status(TurnStatus::CancelRequested);

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !handle.is_requested() {
        if std::time::Instant::now() > deadline {
            panic!("polling fallback never observed cancel-requested transition");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn turn_state_factory_prunes_run_after_terminal_wake() {
    let state = test_run_state(TurnStatus::Running);
    let factory =
        TurnStateRunCancellationFactory::new(Arc::new(StaticTurnStateStore::new(state.clone())))
            .with_poll_interval(Duration::from_secs(60));
    let handle = factory
        .handle_for_run(&state.scope, state.run_id)
        .await
        .unwrap();
    assert_eq!(factory.registered_run_count(), 1);

    factory
        .notify_queued_run(TurnRunWake {
            scope: state.scope,
            run_id: state.run_id,
            status: TurnStatus::Completed,
            event_cursor: EventCursor(2),
        })
        .unwrap();

    assert!(!handle.is_requested());
    assert_eq!(factory.registered_run_count(), 0);
}

#[test]
fn custom_run_cancellation_factory_defaults_to_live_capable() {
    let factory = TestLiveCancellationFactory;

    assert_eq!(
        factory.observation_kind(),
        RunCancellationObservationKind::LiveCapable
    );
    assert!(factory.observation_kind().is_live_capable());
}
