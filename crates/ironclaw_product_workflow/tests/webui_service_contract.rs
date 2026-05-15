//! Contract tests for [`WebUiService`] (the WebChat v2 native facade for #3611).
//!
//! These cover the four browser-facing commands (create_thread, send_message,
//! cancel_run, resolve_gate). The facade is exercised through `DefaultWebUiService`
//! with a real `InMemorySessionThreadService` and a recording stub turn
//! coordinator — handlers will depend on this facade only, so contract
//! coverage here is the caller-level surface (per `.claude/rules/testing.md`).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot, ThreadTimeline, TimelineEntry, TimelineEntryKind,
};
use ironclaw_events::EventCursor;
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    DefaultWebUiService, FakeWebUiService, WebUiAuthenticatedCaller, WebUiCancelRunCommand,
    WebUiCreateThreadCommand, WebUiGateResolution, WebUiGateResolved, WebUiGetTimelineCommand,
    WebUiMessageRunOutcome, WebUiResolveGateCommand, WebUiSendMessageCommand, WebUiService,
    WebUiServiceError, WebUiThreadCreated, WebUiTimelineCursor,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GateRef, GetRunStateRequest, IdempotencyKey,
    ResumeTurnRequest, ResumeTurnResponse, SanitizedCancelReason, SubmitTurnRequest,
    SubmitTurnResponse, TurnCoordinator, TurnError, TurnErrorCategory, TurnRunId, TurnRunState,
    TurnScope, TurnStatus,
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Recording stub for `TurnCoordinator`.
//
// We use a real `InMemorySessionThreadService` for thread state but stub the
// turn coordinator so contract tests can deterministically assert which
// methods the facade called, without driving the full coordinator state
// machine through gate-park transitions.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RecordedCalls {
    submit: Vec<SubmitTurnRequest>,
    resume: Vec<ResumeTurnRequest>,
    cancel: Vec<CancelRunRequest>,
}

struct RecordingTurnCoordinator {
    calls: Mutex<RecordedCalls>,
    submit_response: Mutex<Option<Result<SubmitTurnResponse, TurnError>>>,
    resume_response: Mutex<Option<Result<ResumeTurnResponse, TurnError>>>,
    cancel_response: Mutex<Option<Result<CancelRunResponse, TurnError>>>,
}

impl RecordingTurnCoordinator {
    fn new() -> Self {
        Self {
            calls: Mutex::new(RecordedCalls::default()),
            submit_response: Mutex::new(None),
            resume_response: Mutex::new(None),
            cancel_response: Mutex::new(None),
        }
    }

    fn program_submit(&self, response: Result<SubmitTurnResponse, TurnError>) {
        *self.submit_response.lock().unwrap() = Some(response);
    }

    fn program_resume(&self, response: Result<ResumeTurnResponse, TurnError>) {
        *self.resume_response.lock().unwrap() = Some(response);
    }

    fn program_cancel(&self, response: Result<CancelRunResponse, TurnError>) {
        *self.cancel_response.lock().unwrap() = Some(response);
    }

    fn submits(&self) -> Vec<SubmitTurnRequest> {
        self.calls.lock().unwrap().submit.clone()
    }

    fn resumes(&self) -> Vec<ResumeTurnRequest> {
        self.calls.lock().unwrap().resume.clone()
    }

    fn cancels(&self) -> Vec<CancelRunRequest> {
        self.calls.lock().unwrap().cancel.clone()
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.calls.lock().unwrap().submit.push(request);
        self.submit_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| panic!("RecordingTurnCoordinator: submit response not programmed"))
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.calls.lock().unwrap().resume.push(request);
        self.resume_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| panic!("RecordingTurnCoordinator: resume response not programmed"))
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.calls.lock().unwrap().cancel.push(request);
        self.cancel_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| panic!("RecordingTurnCoordinator: cancel response not programmed"))
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Err(TurnError::ScopeNotFound)
    }
}

// ---------------------------------------------------------------------------
// Recording stub for `EventProjectionService`.
// ---------------------------------------------------------------------------

struct RecordingProjectionService {
    snapshot_calls: Mutex<Vec<ProjectionRequest>>,
    updates_calls: Mutex<Vec<ProjectionRequest>>,
    snapshot_response: Mutex<Option<Result<ProjectionSnapshot, ProjectionError>>>,
    updates_response: Mutex<Option<Result<ProjectionReplay, ProjectionError>>>,
}

impl RecordingProjectionService {
    fn new() -> Self {
        Self {
            snapshot_calls: Mutex::new(Vec::new()),
            updates_calls: Mutex::new(Vec::new()),
            snapshot_response: Mutex::new(None),
            updates_response: Mutex::new(None),
        }
    }

    fn program_snapshot(&self, response: Result<ProjectionSnapshot, ProjectionError>) {
        *self.snapshot_response.lock().unwrap() = Some(response);
    }

    #[allow(dead_code)] // helper kept symmetric with program_snapshot; reserved for future tests
    fn program_updates(&self, response: Result<ProjectionReplay, ProjectionError>) {
        *self.updates_response.lock().unwrap() = Some(response);
    }

    fn snapshot_requests(&self) -> Vec<ProjectionRequest> {
        self.snapshot_calls.lock().unwrap().clone()
    }

    fn updates_requests(&self) -> Vec<ProjectionRequest> {
        self.updates_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventProjectionService for RecordingProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.snapshot_calls.lock().unwrap().push(request.clone());
        self.snapshot_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(empty_snapshot(&request)))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        self.updates_calls.lock().unwrap().push(request.clone());
        self.updates_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(empty_replay(&request)))
    }
}

fn empty_snapshot(request: &ProjectionRequest) -> ProjectionSnapshot {
    ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: Vec::new(),
        },
        runs: Vec::new(),
        next_cursor: ProjectionCursor::origin_for_scope(request.scope.clone()),
        truncated: false,
    }
}

fn empty_replay(request: &ProjectionRequest) -> ProjectionReplay {
    ProjectionReplay {
        updates: Vec::new(),
        runs: Vec::new(),
        next_cursor: ProjectionCursor::origin_for_scope(request.scope.clone()),
        truncated: false,
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn caller_with_agent() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn caller_without_agent() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        None,
        None,
    )
}

fn idempotency(value: &str) -> IdempotencyKey {
    IdempotencyKey::new(value).expect("idempotency key")
}

fn thread_id(value: &str) -> ThreadId {
    ThreadId::new(value).expect("thread id")
}

fn turn_scope_for(caller: &WebUiAuthenticatedCaller, thread: &ThreadId) -> TurnScope {
    caller.turn_scope(thread.clone())
}

fn build_service() -> (
    Arc<InMemorySessionThreadService>,
    Arc<RecordingTurnCoordinator>,
    Arc<RecordingProjectionService>,
    DefaultWebUiService,
) {
    let threads = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(RecordingTurnCoordinator::new());
    let projections = Arc::new(RecordingProjectionService::new());
    let service =
        DefaultWebUiService::new(threads.clone(), coordinator.clone(), projections.clone());
    (threads, coordinator, projections, service)
}

fn submit_accepted(scope: &TurnScope) -> SubmitTurnResponse {
    SubmitTurnResponse::Accepted {
        turn_id: ironclaw_turns::TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Queued,
        resolved_run_profile_id: ironclaw_turns::RunProfileId::new("default").expect("profile id"),
        resolved_run_profile_version: ironclaw_turns::RunProfileVersion::new(1),
        event_cursor: Default::default(),
        accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:stub")
            .expect("message ref"),
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply:stub")
            .expect("reply target"),
    }
    .scope_aligned(scope)
}

// Helper so the returned response's scope isn't a smoke test concern.
trait ScopeAligned {
    fn scope_aligned(self, scope: &TurnScope) -> Self;
}

impl ScopeAligned for SubmitTurnResponse {
    fn scope_aligned(self, _scope: &TurnScope) -> Self {
        self
    }
}

// ---------------------------------------------------------------------------
// create_thread
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_thread_returns_caller_supplied_thread_id() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let outcome = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("create-1"),
            requested_thread_id: Some(thread_id("thread:webui:alpha")),
        })
        .await
        .expect("create thread");
    assert_eq!(outcome.thread_id, thread_id("thread:webui:alpha"));
}

#[tokio::test]
async fn create_thread_generates_thread_id_when_caller_omits_one() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let outcome = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("create-2"),
            requested_thread_id: None,
        })
        .await
        .expect("create thread");
    assert!(
        outcome.thread_id.as_str().starts_with("thread:webui:"),
        "expected generated thread id, got {}",
        outcome.thread_id.as_str()
    );
}

#[tokio::test]
async fn create_thread_requires_agent_context() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let err = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_without_agent(),
            client_action_id: idempotency("create-3"),
            requested_thread_id: None,
        })
        .await
        .expect_err("missing agent");
    assert_eq!(err, WebUiServiceError::MissingAgentContext);
    assert_eq!(err.status_code(), 400);
    assert!(!err.retryable());
}

// C1 regression: a browser retry (same client_action_id, no requested_thread_id)
// must NOT produce two distinct threads. Previously `Uuid::new_v4()` made each
// call yield a fresh thread; the fix derives the id from client_action_id, so
// the underlying ensure_thread call becomes idempotent on retry.
#[tokio::test]
async fn create_thread_is_idempotent_on_client_action_id() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let first = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("create-idem"),
            requested_thread_id: None,
        })
        .await
        .expect("first create");
    let second = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("create-idem"),
            requested_thread_id: None,
        })
        .await
        .expect("second create");
    assert_eq!(
        first.thread_id, second.thread_id,
        "same client_action_id must produce the same thread_id"
    );

    // And a different idempotency key should still produce a distinct thread.
    let other = service
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("create-idem-other"),
            requested_thread_id: None,
        })
        .await
        .expect("other create");
    assert_ne!(first.thread_id, other.thread_id);
}

// H1 regression: ThreadScopeMismatch — a caller asking for a `requested_thread_id`
// that already exists under a different `(tenant, agent)` scope must surface as
// Forbidden (403), not as ThreadServiceUnavailable (503). The previous catch-all
// collapsed every SessionThreadError to 503, which would cause browsers to
// retry forever against a thread they have no right to.
#[tokio::test]
async fn create_thread_scope_mismatch_maps_to_forbidden() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let shared_thread = thread_id("thread:webui:shared");

    let alice = caller_with_agent(); // agent:agent-alpha
    service
        .create_thread(WebUiCreateThreadCommand {
            caller: alice,
            client_action_id: idempotency("scope-1"),
            requested_thread_id: Some(shared_thread.clone()),
        })
        .await
        .expect("alice creates the thread under agent-alpha");

    // Bob is a different agent on the same tenant, asking for the SAME
    // thread_id. The thread service returns ThreadScopeMismatch.
    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        Some(AgentId::new("agent-bravo").expect("agent")),
        None,
    );
    let err = service
        .create_thread(WebUiCreateThreadCommand {
            caller: bob,
            client_action_id: idempotency("scope-2"),
            requested_thread_id: Some(shared_thread),
        })
        .await
        .expect_err("scope mismatch");

    assert_eq!(err, WebUiServiceError::Forbidden);
    assert_eq!(err.status_code(), 403);
    assert!(!err.retryable());
}

// ---------------------------------------------------------------------------
// send_message
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_message_happy_path_submits_turn_and_marks_message() {
    let (threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:beta");
    let scope = turn_scope_for(&caller, &thread);
    coordinator.program_submit(Ok(submit_accepted(&scope)));

    let outcome = service
        .send_message(WebUiSendMessageCommand {
            scope: scope.clone(),
            actor: caller.actor(),
            client_action_id: idempotency("send-1"),
            content: "hello".to_string(),
        })
        .await
        .expect("send message");

    assert_eq!(outcome.thread_id, thread);
    assert!(matches!(
        outcome.run,
        WebUiMessageRunOutcome::Submitted { .. }
    ));

    let submits = coordinator.submits();
    assert_eq!(submits.len(), 1, "submit_turn should be called once");
    assert_eq!(submits[0].scope, scope);
    assert_eq!(submits[0].idempotency_key.as_str(), "send-1");
    assert!(submits[0].source_binding_ref.as_str().starts_with("webui:"));

    // The accepted message must persist in the thread service.
    let history = threads
        .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
            scope: ironclaw_threads::ThreadScope {
                tenant_id: caller.tenant_id.clone(),
                agent_id: caller.agent_id.clone().expect("agent"),
                project_id: caller.project_id.clone(),
                owner_user_id: Some(caller.user_id.clone()),
                mission_id: None,
            },
            thread_id: thread.clone(),
        })
        .await
        .expect("list history");
    assert_eq!(history.messages.len(), 1);
}

#[tokio::test]
async fn send_message_thread_busy_marks_deferred() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:gamma");
    let scope = turn_scope_for(&caller, &thread);
    let busy_run = TurnRunId::new();
    coordinator.program_submit(Err(TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
        active_run_id: busy_run,
        status: TurnStatus::Running,
        event_cursor: Default::default(),
    })));

    let outcome = service
        .send_message(WebUiSendMessageCommand {
            scope,
            actor: caller.actor(),
            client_action_id: idempotency("send-2"),
            content: "hello".to_string(),
        })
        .await
        .expect("send message");

    match outcome.run {
        WebUiMessageRunOutcome::DeferredBusy { active_run_id } => {
            assert_eq!(active_run_id, busy_run)
        }
        other => panic!("expected DeferredBusy, got {other:?}"),
    }
}

#[tokio::test]
async fn send_message_maps_turn_error_to_typed_rejection() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:delta");
    let scope = turn_scope_for(&caller, &thread);
    coordinator.program_submit(Err(TurnError::Unauthorized));

    let err = service
        .send_message(WebUiSendMessageCommand {
            scope,
            actor: caller.actor(),
            client_action_id: idempotency("send-3"),
            content: "hello".to_string(),
        })
        .await
        .expect_err("expected unauthorized");

    match &err {
        WebUiServiceError::TurnRejected { category } => {
            assert_eq!(*category, TurnErrorCategory::Unauthorized);
        }
        other => panic!("expected TurnRejected, got {other:?}"),
    }
    assert_eq!(err.status_code(), 403);
}

#[tokio::test]
async fn send_message_requires_agent_context() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let caller = caller_without_agent();
    let thread = thread_id("thread:webui:epsilon");
    let scope = caller.turn_scope(thread); // agent_id is None inside this scope

    let err = service
        .send_message(WebUiSendMessageCommand {
            scope,
            actor: caller.actor(),
            client_action_id: idempotency("send-4"),
            content: "hello".to_string(),
        })
        .await
        .expect_err("missing agent");

    assert_eq!(err, WebUiServiceError::MissingAgentContext);
}

// ---------------------------------------------------------------------------
// cancel_run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_run_forwards_to_coordinator_and_maps_response() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:zeta"));
    let run_id = TurnRunId::new();
    coordinator.program_cancel(Ok(CancelRunResponse {
        run_id,
        status: TurnStatus::CancelRequested,
        event_cursor: Default::default(),
        already_terminal: false,
    }));

    let outcome = service
        .cancel_run(WebUiCancelRunCommand {
            scope: scope.clone(),
            actor: caller.actor(),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            client_action_id: idempotency("cancel-1"),
        })
        .await
        .expect("cancel run");

    assert_eq!(outcome.run_id, run_id);
    assert!(!outcome.already_terminal);

    let cancels = coordinator.cancels();
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0].scope, scope);
    assert_eq!(cancels[0].run_id, run_id);
    assert_eq!(cancels[0].reason, SanitizedCancelReason::UserRequested);
}

#[tokio::test]
async fn cancel_run_propagates_turn_error() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:eta"));
    coordinator.program_cancel(Err(TurnError::ScopeNotFound));

    let err = service
        .cancel_run(WebUiCancelRunCommand {
            scope,
            actor: caller.actor(),
            run_id: TurnRunId::new(),
            reason: SanitizedCancelReason::UserRequested,
            client_action_id: idempotency("cancel-2"),
        })
        .await
        .expect_err("expected scope-not-found");

    match err {
        WebUiServiceError::TurnRejected { category, .. } => {
            assert_eq!(category, TurnErrorCategory::ScopeNotFound)
        }
        other => panic!("expected TurnRejected, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// resolve_gate
// ---------------------------------------------------------------------------

fn resume_response(run_id: TurnRunId) -> ResumeTurnResponse {
    ResumeTurnResponse {
        run_id,
        status: TurnStatus::Running,
        event_cursor: Default::default(),
    }
}

fn cancel_response(run_id: TurnRunId) -> CancelRunResponse {
    CancelRunResponse {
        run_id,
        status: TurnStatus::Cancelled,
        event_cursor: Default::default(),
        already_terminal: false,
    }
}

#[tokio::test]
async fn resolve_gate_approved_routes_to_resume_turn() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:theta"));
    let run_id = TurnRunId::new();
    coordinator.program_resume(Ok(resume_response(run_id)));

    let outcome = service
        .resolve_gate(WebUiResolveGateCommand {
            scope,
            actor: caller.actor(),
            run_id,
            gate_ref: GateRef::new("gate:approval:42").expect("gate"),
            client_action_id: idempotency("gate-1"),
            resolution: WebUiGateResolution::Approved { always: false },
        })
        .await
        .expect("resolve gate");

    match outcome {
        WebUiGateResolved::Resumed { run_id: actual } => assert_eq!(actual, run_id),
        other => panic!("expected Resumed, got {other:?}"),
    }
    assert_eq!(coordinator.resumes().len(), 1);
    assert!(coordinator.cancels().is_empty());
}

// C2 regression: previously this arm fell through to resume_turn and silently
// dropped `credential_ref`. The fix is to fail loud with Unsupported until the
// credential-binding port lands. If/when that port exists, this test should be
// flipped back to "routes to resume_turn after binding credential".
#[tokio::test]
async fn resolve_gate_credential_provided_returns_unsupported_until_binding_port_exists() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:iota"));
    let run_id = TurnRunId::new();

    let err = service
        .resolve_gate(WebUiResolveGateCommand {
            scope,
            actor: caller.actor(),
            run_id,
            gate_ref: GateRef::new("gate:auth:gmail").expect("gate"),
            client_action_id: idempotency("gate-2"),
            resolution: WebUiGateResolution::CredentialProvided {
                credential_ref: "cred:abc".to_string(),
            },
        })
        .await
        .expect_err("CredentialProvided is currently unsupported");

    match &err {
        WebUiServiceError::Unsupported { what } => {
            assert_eq!(*what, "credential_provided_gate_resolution");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert_eq!(err.status_code(), 501);
    assert!(!err.retryable());
    // Crucially: the facade must NOT have called resume_turn or cancel_run
    // when it doesn't know how to honor the credential.
    assert!(coordinator.resumes().is_empty());
    assert!(coordinator.cancels().is_empty());
}

#[tokio::test]
async fn resolve_gate_denied_routes_to_cancel_run() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:kappa"));
    let run_id = TurnRunId::new();
    coordinator.program_cancel(Ok(cancel_response(run_id)));

    let outcome = service
        .resolve_gate(WebUiResolveGateCommand {
            scope,
            actor: caller.actor(),
            run_id,
            gate_ref: GateRef::new("gate:approval:43").expect("gate"),
            client_action_id: idempotency("gate-3"),
            resolution: WebUiGateResolution::Denied,
        })
        .await
        .expect("resolve gate");

    match outcome {
        WebUiGateResolved::Cancelled { run_id: actual, .. } => assert_eq!(actual, run_id),
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert_eq!(coordinator.cancels().len(), 1);
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn resolve_gate_cancelled_routes_to_cancel_run() {
    let (_threads, coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();
    let scope = turn_scope_for(&caller, &thread_id("thread:webui:lambda"));
    let run_id = TurnRunId::new();
    coordinator.program_cancel(Ok(cancel_response(run_id)));

    let outcome = service
        .resolve_gate(WebUiResolveGateCommand {
            scope,
            actor: caller.actor(),
            run_id,
            gate_ref: GateRef::new("gate:approval:44").expect("gate"),
            client_action_id: idempotency("gate-4"),
            resolution: WebUiGateResolution::Cancelled,
        })
        .await
        .expect("resolve gate");

    assert!(matches!(outcome, WebUiGateResolved::Cancelled { .. }));
    assert_eq!(coordinator.cancels().len(), 1);
}

// ---------------------------------------------------------------------------
// FakeWebUiService — sanity coverage so downstream gateway handler tests can
// rely on the recorded-calls fields used by Slice 2.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fake_webui_service_records_create_thread_calls() {
    let fake = FakeWebUiService::new();
    fake.program_create_thread(WebUiThreadCreated {
        thread_id: thread_id("thread:webui:fake"),
    });

    let outcome = fake
        .create_thread(WebUiCreateThreadCommand {
            caller: caller_with_agent(),
            client_action_id: idempotency("fake-create-1"),
            requested_thread_id: None,
        })
        .await
        .expect("fake create thread");

    assert_eq!(outcome.thread_id, thread_id("thread:webui:fake"));
    assert_eq!(fake.create_thread_calls().len(), 1);
}

#[tokio::test]
async fn fake_webui_service_propagates_programmed_error() {
    let fake = FakeWebUiService::new();
    fake.fail_send_message(WebUiServiceError::Transient);

    let err = fake
        .send_message(WebUiSendMessageCommand {
            scope: caller_with_agent().turn_scope(thread_id("thread:webui:fake")),
            actor: caller_with_agent().actor(),
            client_action_id: idempotency("fake-send-1"),
            content: "hello".to_string(),
        })
        .await
        .expect_err("forced failure");
    assert_eq!(err, WebUiServiceError::Transient);
    assert!(err.retryable());
}

// ---------------------------------------------------------------------------
// get_timeline_snapshot / get_timeline_updates
// ---------------------------------------------------------------------------

fn sample_timeline_entry() -> TimelineEntry {
    use ironclaw_host_api::CapabilityId;
    TimelineEntry {
        cursor: EventCursor::origin(),
        event_id: ironclaw_events::RuntimeEventId::default(),
        timestamp: chrono::Utc::now(),
        kind: TimelineEntryKind::AssistantReplyFinalized,
        invocation_id: InvocationId::new(),
        thread_id: Some(thread_id("thread:webui:mu")),
        capability_id: CapabilityId::new("model.reply.v1").expect("capability id"),
        provider: None,
        runtime: None,
        process_id: None,
        output_bytes: Some(42),
        error_kind: None,
    }
}

#[tokio::test]
async fn get_timeline_snapshot_routes_to_projection_service() {
    let (_threads, _coordinator, projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:mu");

    let entry = sample_timeline_entry();
    let scope = ProjectionScope {
        stream: ironclaw_events::EventStreamKey::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            caller.agent_id.clone(),
        ),
        read_scope: ironclaw_events::ReadScope {
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: Some(thread.clone()),
            process_id: None,
        },
    };
    projections.program_snapshot(Ok(ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: vec![entry.clone()],
        },
        runs: Vec::new(),
        next_cursor: ProjectionCursor::origin_for_scope(scope.clone()),
        truncated: false,
    }));

    let snapshot = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller.clone(),
            thread_id: thread.clone(),
            after: None,
            limit: 10,
        })
        .await
        .expect("snapshot");

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(
        snapshot.entries[0].kind,
        TimelineEntryKind::AssistantReplyFinalized
    );
    assert!(snapshot.runs.is_empty());
    assert!(!snapshot.truncated);

    let requests = projections.snapshot_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].limit, 10);
}

#[tokio::test]
async fn get_timeline_updates_passes_cursor_through() {
    let (_threads, _coordinator, projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:nu");

    // First call: snapshot to mint a cursor the browser would hold.
    let cursor_snapshot = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller.clone(),
            thread_id: thread.clone(),
            after: None,
            limit: 0, // exercise default-limit clamping
        })
        .await
        .expect("initial snapshot");

    let cursor = cursor_snapshot.next_cursor;

    // Second call: updates after the minted cursor.
    let replay = service
        .get_timeline_updates(WebUiGetTimelineCommand {
            caller: caller.clone(),
            thread_id: thread.clone(),
            after: Some(cursor.clone()),
            limit: 25,
        })
        .await
        .expect("updates");

    assert!(replay.entries.is_empty());
    let requests = projections.updates_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].limit, 25);
    assert!(requests[0].after.is_some());
}

#[tokio::test]
async fn get_timeline_clamps_oversize_limit() {
    let (_threads, _coordinator, projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:xi");

    let _ = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller,
            thread_id: thread,
            after: None,
            limit: 9_999_999,
        })
        .await
        .expect("snapshot");

    let requests = projections.snapshot_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].limit,
        ironclaw_event_projections::MAX_PROJECTION_PAGE_LIMIT
    );
}

#[tokio::test]
async fn get_timeline_uses_default_limit_when_zero() {
    let (_threads, _coordinator, projections, service) = build_service();
    let caller = caller_with_agent();

    let _ = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller,
            thread_id: thread_id("thread:webui:omicron"),
            after: None,
            limit: 0,
        })
        .await
        .expect("snapshot");

    assert_eq!(
        projections.snapshot_requests()[0].limit,
        ironclaw_product_workflow::WEBUI_TIMELINE_DEFAULT_LIMIT
    );
}

#[tokio::test]
async fn get_timeline_requires_agent_context() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let err = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller_without_agent(),
            thread_id: thread_id("thread:webui:pi"),
            after: None,
            limit: 50,
        })
        .await
        .expect_err("missing agent");
    assert_eq!(err, WebUiServiceError::MissingAgentContext);
}

#[tokio::test]
async fn get_timeline_rejects_cursor_for_different_thread() {
    let (_threads, _coordinator, _projections, service) = build_service();
    let caller = caller_with_agent();

    // Mint a cursor for thread A …
    let cursor_a = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller.clone(),
            thread_id: thread_id("thread:webui:rho-a"),
            after: None,
            limit: 0,
        })
        .await
        .expect("snapshot A")
        .next_cursor;

    // … then try to replay it under thread B. The scope mismatch must trip
    // the InvalidInput rail before reaching the projection service.
    let err = service
        .get_timeline_updates(WebUiGetTimelineCommand {
            caller,
            thread_id: thread_id("thread:webui:rho-b"),
            after: Some(cursor_a),
            limit: 25,
        })
        .await
        .expect_err("scope mismatch");
    assert_eq!(err, WebUiServiceError::InvalidInput);
}

#[tokio::test]
async fn get_timeline_maps_projection_rebase_required() {
    let (_threads, _coordinator, projections, service) = build_service();
    let caller = caller_with_agent();
    let thread = thread_id("thread:webui:sigma");

    let scope = ProjectionScope {
        stream: ironclaw_events::EventStreamKey::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            caller.agent_id.clone(),
        ),
        read_scope: ironclaw_events::ReadScope {
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: Some(thread.clone()),
            process_id: None,
        },
    };
    let earliest = ProjectionCursor::origin_for_scope(scope.clone());
    projections.program_snapshot(Err(ProjectionError::RebaseRequired {
        requested: Box::new(ProjectionCursor::for_scope(
            scope.clone(),
            EventCursor::origin(),
        )),
        earliest: Box::new(earliest.clone()),
    }));

    let err = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller,
            thread_id: thread,
            after: None,
            limit: 50,
        })
        .await
        .expect_err("expected rebase");

    match err {
        WebUiServiceError::TimelineRebaseRequired { earliest_cursor } => {
            assert_eq!((*earliest_cursor).serialized(), serialize_cursor(&earliest));
        }
        other => panic!("expected TimelineRebaseRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn get_timeline_maps_projection_source_to_transient() {
    let (_threads, _coordinator, projections, service) = build_service();
    projections.program_snapshot(Err(ProjectionError::Source {
        operation: "snapshot",
    }));
    let err = service
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller_with_agent(),
            thread_id: thread_id("thread:webui:tau"),
            after: None,
            limit: 50,
        })
        .await
        .expect_err("expected transient");
    assert_eq!(err, WebUiServiceError::Transient);
    assert!(err.retryable());
}

#[tokio::test]
async fn fake_webui_service_records_timeline_snapshot_calls() {
    let fake = FakeWebUiService::new();
    let outcome = fake
        .get_timeline_snapshot(WebUiGetTimelineCommand {
            caller: caller_with_agent(),
            thread_id: thread_id("thread:webui:fake-tl"),
            after: None,
            limit: 50,
        })
        .await
        .expect("fake snapshot");
    assert!(outcome.entries.is_empty());
    assert_eq!(fake.timeline_snapshot_calls().len(), 1);
}

// Tiny helper: re-serialize cursors to compare across `Box` and direct ownership.
fn serialize_cursor(cursor: &ProjectionCursor) -> String {
    serde_json::to_string(cursor).expect("cursor serializes")
}

trait CursorSerialize {
    fn serialized(&self) -> String;
}

impl CursorSerialize for WebUiTimelineCursor {
    fn serialized(&self) -> String {
        serde_json::to_string(self).expect("cursor serializes")
    }
}

// Silence the unused-import lint for `Uuid` when no test currently uses it.
#[allow(dead_code)]
fn _retain_uuid_import() -> Uuid {
    Uuid::nil()
}
