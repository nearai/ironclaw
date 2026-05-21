//! Contract tests for the InboundTurnService.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostInputBatch, HostInputQueue, HostInputQueueError,
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, ProductLiveCancellationProbe, RunCancellationFactory,
    RunCancellationHandle,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, FakeConversationBindingService, InboundTurnOutcome,
    InboundTurnService, ProductWorkflowError,
};
use ironclaw_reborn::loop_driver_host::LoopCapabilityPortFactory;
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::model_routes::{
    ModelRoute, ModelRoutePolicy, ModelSelectionMode, ModelSlot, StaticModelRouteResolver,
};
use ironclaw_reborn::planned_driver_factory::{
    PLANNED_DEFAULT_PROFILE_ID, default_planned_run_profile_resolver,
};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, build_product_live_planned_runtime,
};
use ironclaw_threads::{
    InMemorySessionThreadService, MessageStatus, SessionThreadService, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator, EventCursor, GetRunStateRequest,
    IdempotencyKey, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion,
    SanitizedCancelReason, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor,
    TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnRunWake, TurnScope,
    TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
        LoopCancelReasonKind, LoopCapabilityPort, LoopInputAckToken, LoopInputCursorToken,
        LoopRunContext, NoOpBudgetAccountant, NoOpPolicyGuard, PromptMode,
    },
};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

fn sample_user_message_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_user_message_envelope_with_install_and_text(event_suffix, "install_alpha", "hello world")
}

#[derive(Default)]
struct CapturingTurnCoordinator {
    last_submit: Arc<Mutex<Option<SubmitTurnRequest>>>,
}

#[async_trait]
impl TurnCoordinator for CapturingTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let response = SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
        };
        *self
            .last_submit
            .lock()
            .expect("capturing coordinator lock poisoned") = Some(request);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by inbound turn contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by inbound turn contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by inbound turn contract tests")
    }
}

#[derive(Clone, Default)]
struct ScriptedTurnCoordinator {
    results: Arc<Mutex<VecDeque<Result<SubmitTurnResponse, TurnError>>>>,
    submissions: Arc<Mutex<Vec<SubmitTurnRequest>>>,
}

impl ScriptedTurnCoordinator {
    fn push_result(&self, result: Result<SubmitTurnResponse, TurnError>) {
        self.results
            .lock()
            .expect("scripted coordinator lock poisoned")
            .push_back(result);
    }

    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions
            .lock()
            .expect("scripted coordinator submissions lock poisoned")
            .clone()
    }
}

#[async_trait]
impl TurnCoordinator for ScriptedTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.submissions
            .lock()
            .expect("scripted coordinator submissions lock poisoned")
            .push(request.clone());
        self.results
            .lock()
            .expect("scripted coordinator lock poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                Ok(SubmitTurnResponse::Accepted {
                    turn_id: TurnId::new(),
                    run_id: TurnRunId::new(),
                    status: TurnStatus::Queued,
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    event_cursor: EventCursor::default(),
                    accepted_message_ref: request.accepted_message_ref.clone(),
                    reply_target_binding_ref: request.reply_target_binding_ref.clone(),
                })
            })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by inbound turn contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by inbound turn contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by inbound turn contract tests")
    }
}

struct ReplyModelGateway {
    reply: String,
    requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
}

#[async_trait]
impl HostManagedModelGateway for ReplyModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("reply model gateway requests lock poisoned")
            .push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            self.reply.clone(),
        ))
    }
}

struct PausingReplyModelGateway {
    reply: String,
    requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
    release: CancellationToken,
}

#[async_trait]
impl HostManagedModelGateway for PausingReplyModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("pausing model gateway requests lock poisoned")
            .push(request);
        self.release.cancelled().await;
        Ok(HostManagedModelResponse::assistant_reply(
            self.reply.clone(),
        ))
    }
}

struct EmptyCapabilityFactory;

#[async_trait]
impl LoopCapabilityPortFactory for EmptyCapabilityFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(Arc::new(EmptyLoopCapabilityPort))
    }
}

struct AllowAllCapabilitySurfaceResolver;

#[async_trait]
impl CapabilitySurfaceProfileResolver for AllowAllCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
    }
}

struct EmptyInputQueue;

#[async_trait]
impl HostInputQueue for EmptyInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
}

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct ReadyRunCancellationFactory {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
}

impl ReadyRunCancellationFactory {
    fn handle_for(&self, run_id: TurnRunId) -> Option<RunCancellationHandle> {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&run_id)
            .cloned()
    }

    fn product_cancellation_observed(&self, run_id: TurnRunId) -> bool {
        self.handle_for(run_id)
            .map(|handle| handle.is_requested())
            .unwrap_or(false)
    }
}

#[async_trait]
impl RunCancellationFactory for ReadyRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle.clone());
        Ok(handle)
    }

    fn notify_run_wake(&self, wake: &TurnRunWake) {
        // End-to-end product-live cancellation observation: when the
        // coordinator publishes a `CancelRequested` wake, flip the retained
        // run handle so product code can observe cancellation without any
        // factory backdoor.
        if wake.status != TurnStatus::CancelRequested {
            return;
        }
        if let Some(handle) = self.handle_for(wake.run_id) {
            handle.request(LoopCancelReasonKind::UserRequested);
        }
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        // Probe owns its handle directly. Not inserted into `self.handles` —
        // the readiness probe is ephemeral and self-contained, so the factory's
        // run-keyed handle map must not grow on every verifier call.
        Some(Box::new(ReadyRunCancellationProbe {
            handle: RunCancellationHandle::default(),
        }))
    }

    fn is_product_cancellation_observed(
        &self,
        run_id: TurnRunId,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(self.product_cancellation_observed(run_id))
    }
}

struct ReadyRunCancellationProbe {
    handle: RunCancellationHandle,
}

impl ProductLiveCancellationProbe for ReadyRunCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle.request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle.is_requested())
    }
}

struct UnretainedRunCancellationFactory;

#[async_trait]
impl RunCancellationFactory for UnretainedRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        Ok(RunCancellationHandle::default())
    }
}

fn turn_state_store_dyn(store: &Arc<InMemoryTurnStateStore>) -> Arc<dyn TurnStateStore> {
    Arc::clone(store) as Arc<dyn TurnStateStore>
}

fn test_safety_context() -> InstructionSafetyContext {
    InstructionSafetyContext::new("policy:test", "test safety context")
        .expect("test safety context")
}

fn binding_with_user(user: &str, thread: &str) -> ironclaw_product_workflow::ResolvedBinding {
    ironclaw_product_workflow::ResolvedBinding {
        tenant_id: TenantId::new("tenant:install_alpha").expect("valid tenant"),
        user_id: UserId::new(user).expect("valid user"),
        thread_id: ThreadId::new(thread).expect("valid thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("valid agent")),
        project_id: None,
    }
}

fn sample_user_message_envelope_with_text(
    event_suffix: &str,
    text: &str,
) -> ProductInboundEnvelope {
    sample_user_message_envelope_with_install_and_text(event_suffix, "install_alpha", text)
}

fn sample_user_message_envelope_with_install_and_text(
    event_suffix: &str,
    installation_id: &str,
    text: &str,
) -> ProductInboundEnvelope {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id,
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new(installation_id).expect("valid"),
        Utc::now(),
        &evidence,
    )
    .expect("verified");

    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat).expect("valid"),
        ),
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

#[tokio::test]
async fn user_message_resolves_binding_persists_message_and_submits_turn() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("turn1");
    let outcome: InboundTurnOutcome = service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let binding = match &outcome {
        InboundTurnOutcome::Submitted { binding, .. } => binding,
        _ => panic!("expected Submitted, got {outcome:?}"),
    };

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("hello world"));
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert!(history.messages[0].turn_run_id.is_some());
}

#[tokio::test]
async fn user_message_no_profile_submission_uses_planned_reborn_default() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let resolver =
        Arc::new(default_planned_run_profile_resolver().expect("planned default profile resolver"));
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_run_profile_resolver(resolver);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("planned-product-default");
    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("planned default submit");

    let InboundTurnOutcome::Submitted {
        binding,
        submitted_run_id,
        ..
    } = outcome
    else {
        panic!("expected submitted outcome");
    };
    let state = store
        .get_run_state(GetRunStateRequest {
            scope: ironclaw_turns::TurnScope::new(
                binding.tenant_id,
                binding.agent_id,
                binding.project_id,
                binding.thread_id,
            ),
            run_id: submitted_run_id,
        })
        .await
        .unwrap();
    assert_eq!(
        state.resolved_run_profile_id.as_str(),
        PLANNED_DEFAULT_PROFILE_ID
    );
    assert_eq!(state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn user_message_no_profile_uses_product_live_runtime_and_persists_reply() {
    let binding_service = FakeConversationBindingService::new();
    let envelope = sample_user_message_envelope("planned-product-live");
    let binding = binding_with_user("user:product-live", "thread:product-live");
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());

    let thread_service = InMemorySessionThreadService::default();
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let model_requests = Arc::new(Mutex::new(Vec::new()));
    let model_gateway = Arc::new(ReplyModelGateway {
        reply: "planned product reply".to_string(),
        requests: Arc::clone(&model_requests),
    });
    let thread_scope = ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding.agent_id.clone().expect("agent id"),
        project_id: binding.project_id.clone(),
        owner_user_id: Some(binding.user_id.clone()),
        mission_id: None,
    };
    let model_route_resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("nearai", "qwen3-coder").expect("valid model route"),
        ),
    );
    let cancellation_factory = Arc::new(ReadyRunCancellationFactory::default());
    let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_store),
        thread_service: Arc::new(thread_service.clone()),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
        loop_checkpoint_store: checkpoint_store.clone(),
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        capability_factory: Arc::new(EmptyCapabilityFactory),
        capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
        loop_exit_evidence: Arc::new(
            ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                Arc::new(thread_service.clone()),
                turn_state_store_dyn(&turn_store),
                checkpoint_store,
                thread_scope.clone(),
            )
            .with_cancellation_factory(cancellation_factory.clone()),
        ),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(model_route_resolver),
        cancellation_factory: Some(cancellation_factory.clone()),
        skill_context_source: None,
        input_queue: Some(Arc::new(EmptyInputQueue)),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
        model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
        safety_context: Some(test_safety_context()),
    })
    .expect("product-live runtime should build");

    let cancel = CancellationToken::new();
    let worker = Arc::clone(&composition.worker);
    let worker_cancel = cancel.clone();
    let worker_handle = tokio::spawn(async move { worker.run(worker_cancel).await });
    let service = DefaultInboundTurnService::new(
        binding_service,
        thread_service.clone(),
        Arc::clone(&composition.coordinator),
    );

    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("product live submit");
    let InboundTurnOutcome::Submitted {
        submitted_run_id, ..
    } = outcome
    else {
        panic!("expected submitted outcome");
    };
    let turn_scope = TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    );
    let state = match timeout(Duration::from_secs(3), async {
        loop {
            let state = turn_store
                .get_run_state(GetRunStateRequest {
                    scope: turn_scope.clone(),
                    run_id: submitted_run_id,
                })
                .await
                .expect("run state");
            if state.status.is_terminal() {
                return state;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    {
        Ok(state) => state,
        Err(error) => {
            let state = turn_store
                .get_run_state(GetRunStateRequest {
                    scope: turn_scope.clone(),
                    run_id: submitted_run_id,
                })
                .await
                .expect("run state after timeout");
            let history = thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: thread_scope.clone(),
                    thread_id: binding.thread_id.clone(),
                })
                .await
                .expect("history after timeout");
            panic!(
                "product live run should finish: {error}; last state: {state:?}; model requests: {}; history: {:?}",
                model_requests.lock().unwrap().len(),
                history.messages
            );
        }
    };

    cancel.cancel();
    worker_handle.await.expect("worker should stop cleanly");

    assert_eq!(state.status, TurnStatus::Completed);
    assert_eq!(
        state.resolved_run_profile_id.as_str(),
        PLANNED_DEFAULT_PROFILE_ID
    );
    assert_eq!(model_requests.lock().unwrap().len(), 1);

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: binding.thread_id,
        })
        .await
        .expect("history");
    assert!(history.messages.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("planned product reply")
    }));
}

#[tokio::test]
async fn user_message_no_profile_can_cancel_product_live_run_from_product_path() {
    let binding_service = FakeConversationBindingService::new();
    let envelope = sample_user_message_envelope("planned-product-live-cancel");
    let binding = binding_with_user("user:product-live", "thread:product-live-cancel-live");
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());

    let thread_service = InMemorySessionThreadService::default();
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let model_requests = Arc::new(Mutex::new(Vec::new()));
    let model_release = CancellationToken::new();
    let model_gateway = Arc::new(PausingReplyModelGateway {
        reply: "reply after cancel".to_string(),
        requests: Arc::clone(&model_requests),
        release: model_release.clone(),
    });
    let thread_scope = ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding.agent_id.clone().expect("agent id"),
        project_id: binding.project_id.clone(),
        owner_user_id: Some(binding.user_id.clone()),
        mission_id: None,
    };
    let model_route_resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("nearai", "qwen3-coder").expect("valid model route"),
        ),
    );
    let cancellation_factory = Arc::new(ReadyRunCancellationFactory::default());
    let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_store),
        thread_service: Arc::new(thread_service.clone()),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
        loop_checkpoint_store: checkpoint_store.clone(),
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        capability_factory: Arc::new(EmptyCapabilityFactory),
        capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
        // Product-live composition must bind the applier evidence to the
        // runtime cancellation source even if the supplied evidence is not.
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            Arc::new(thread_service.clone()),
            turn_state_store_dyn(&turn_store),
            checkpoint_store,
            thread_scope.clone(),
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(model_route_resolver),
        cancellation_factory: Some(cancellation_factory.clone()),
        skill_context_source: None,
        input_queue: Some(Arc::new(EmptyInputQueue)),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
        model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
        safety_context: Some(test_safety_context()),
    })
    .expect("product-live runtime should build");

    let cancel = CancellationToken::new();
    let worker = Arc::clone(&composition.worker);
    let worker_cancel = cancel.clone();
    let worker_handle = tokio::spawn(async move { worker.run(worker_cancel).await });
    let service = DefaultInboundTurnService::new(
        binding_service,
        thread_service.clone(),
        Arc::clone(&composition.coordinator),
    );

    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("product live submit");
    let InboundTurnOutcome::Submitted {
        submitted_run_id, ..
    } = outcome
    else {
        panic!("expected submitted outcome");
    };
    timeout(Duration::from_secs(3), async {
        loop {
            if !model_requests
                .lock()
                .expect("model requests lock poisoned")
                .is_empty()
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("product live run should reach model call");

    let turn_scope = TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    );
    let cancel_response = composition
        .coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope.clone(),
            actor: TurnActor::new(binding.user_id.clone()),
            run_id: submitted_run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-product-live-cancel").expect("valid"),
        })
        .await
        .expect("product cancel request");
    assert_eq!(cancel_response.status, TurnStatus::CancelRequested);
    // End-to-end proof: `coordinator.cancel_run` alone — no factory backdoor —
    // must reach the retained run handle through the runtime's wake-notifier
    // composition. Poll briefly to absorb async wake propagation.
    timeout(Duration::from_secs(5), async {
        loop {
            if cancellation_factory.product_cancellation_observed(submitted_run_id) {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("coordinator.cancel_run must drive product cancellation observation end-to-end");

    model_release.cancel();
    let state = timeout(Duration::from_secs(3), async {
        loop {
            let state = turn_store
                .get_run_state(GetRunStateRequest {
                    scope: turn_scope.clone(),
                    run_id: submitted_run_id,
                })
                .await
                .expect("run state");
            if state.status.is_terminal() {
                return state;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("product live run should finish after cancellation");

    cancel.cancel();
    worker_handle.await.expect("worker should stop cleanly");

    assert_eq!(state.status, TurnStatus::Cancelled);
    // Reborn-integration's executor preserves the assistant reply that arrived
    // before the cancellation observation; the run still terminates as
    // Cancelled. Verify the reply lands in thread history and the run is
    // cancelled — both must hold together.
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: binding.thread_id,
        })
        .await
        .expect("history");
    assert!(history.messages.iter().any(|message| {
        message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(submitted_run_id.to_string().as_str())
            && message.content.as_deref() == Some("reply after cancel")
    }));
}

#[tokio::test]
async fn product_live_runtime_rejects_unretained_cancellation_factory() {
    let binding_service = FakeConversationBindingService::new();
    let envelope = sample_user_message_envelope("planned-product-inert-cancel");
    let binding = binding_with_user("user:product-live", "thread:product-live-cancel");
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());

    let thread_service = InMemorySessionThreadService::default();
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let model_gateway = Arc::new(ReplyModelGateway {
        reply: "planned product reply".to_string(),
        requests: Arc::new(Mutex::new(Vec::new())),
    });
    let thread_scope = ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding.agent_id.clone().expect("agent id"),
        project_id: binding.project_id.clone(),
        owner_user_id: Some(binding.user_id.clone()),
        mission_id: None,
    };
    let model_route_resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("nearai", "qwen3-coder").expect("valid model route"),
        ),
    );

    let error = match build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: turn_store,
        thread_service: Arc::new(thread_service),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
        loop_checkpoint_store: checkpoint_store.clone(),
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        capability_factory: Arc::new(EmptyCapabilityFactory),
        capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            Arc::new(InMemorySessionThreadService::default()),
            Arc::new(InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
            checkpoint_store,
            thread_scope,
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(model_route_resolver),
        cancellation_factory: Some(Arc::new(UnretainedRunCancellationFactory)),
        skill_context_source: None,
        input_queue: Some(Arc::new(EmptyInputQueue)),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
        model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
        safety_context: Some(test_safety_context()),
    }) {
        Ok(_) => panic!("product-live readiness must reject inert cancellation"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("inert cancellation_factory"));
}

#[tokio::test]
async fn busy_thread_persists_second_message_as_deferred() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let first = sample_user_message_envelope("busy1");
    service.accept_user_message(&first).await.expect("first");
    let second = sample_user_message_envelope_with_text("busy2", "second");
    let outcome = service
        .accept_user_message(&second)
        .await
        .expect("second deferred");
    assert!(matches!(outcome, InboundTurnOutcome::DeferredBusy { .. }));

    let binding = match outcome {
        InboundTurnOutcome::DeferredBusy { binding, .. } => binding,
        _ => unreachable!(),
    };
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].content.as_deref(), Some("second"));
    assert_eq!(history.messages[1].status, MessageStatus::DeferredBusy);
}

#[tokio::test]
async fn retry_validates_live_binding_before_accepted_message_replay() {
    let binding_service = FakeConversationBindingService::new();
    let binding_handle = binding_service.clone();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    coordinator.push_result(Err(TurnError::Unavailable {
        reason: "transient submit failure".into(),
    }));
    let coordinator_handle = coordinator.clone();
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("binding-churn");
    let first_err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("first submit fails after message acceptance");
    assert!(matches!(
        first_err,
        ProductWorkflowError::TurnSubmissionFailed { .. }
    ));
    assert_eq!(binding_handle.resolve_count(), 1);

    binding_handle.program_binding(
        envelope.source_binding_key(),
        binding_with_user("user:churned", "thread:churned"),
    );

    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("retry validates the current binding before replay");
    let InboundTurnOutcome::Submitted { binding, .. } = outcome else {
        panic!("expected submitted retry")
    };
    assert_eq!(binding.user_id.as_str(), "user:churned");
    assert_eq!(binding.thread_id.as_str(), "thread:churned");
    assert_eq!(
        binding_handle.resolve_count(),
        2,
        "retry must validate current binding before accepted-message replay"
    );

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    let submissions = coordinator_handle.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(
        submissions[0].idempotency_key.as_str(),
        submissions[1].idempotency_key.as_str(),
        "retry after post-submit failure must reuse stable turn idempotency key"
    );
}

#[tokio::test]
async fn replay_lookup_is_namespaced_by_installation() {
    let binding_service = FakeConversationBindingService::new();
    let binding_handle = binding_service.clone();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    coordinator.push_result(Err(TurnError::Unavailable {
        reason: "transient submit failure".into(),
    }));
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let first = sample_user_message_envelope_with_install_and_text(
        "shared-event",
        "install_alpha",
        "alpha",
    );
    service
        .accept_user_message(&first)
        .await
        .expect_err("first submit fails after accepting alpha message");

    let second =
        sample_user_message_envelope_with_install_and_text("shared-event", "install_beta", "beta");
    let outcome = service
        .accept_user_message(&second)
        .await
        .expect("second install must not replay alpha message");
    let InboundTurnOutcome::Submitted { binding, .. } = outcome else {
        panic!("expected submitted beta message")
    };
    assert_eq!(binding.tenant_id.as_str(), "tenant:install_beta");
    assert_eq!(
        binding_handle.resolve_count(),
        2,
        "same conversation/event under another installation must resolve its own binding"
    );

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("beta"));
}

#[tokio::test]
async fn deferred_busy_retry_resubmits_existing_message() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    let active_run_id = TurnRunId::new();
    coordinator.push_result(Err(TurnError::ThreadBusy(ThreadBusy {
        active_run_id,
        status: TurnStatus::Running,
        event_cursor: EventCursor::default(),
    })));
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("busy-retry-existing");
    let first = service
        .accept_user_message(&envelope)
        .await
        .expect("first busy");
    assert!(matches!(first, InboundTurnOutcome::DeferredBusy { .. }));

    let second = service
        .accept_user_message(&envelope)
        .await
        .expect("retry submits existing deferred message");
    let InboundTurnOutcome::Submitted { binding, .. } = second else {
        panic!("expected submitted retry")
    };
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
}

#[tokio::test]
async fn reply_target_binding_ref_has_single_reply_prefix() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = CapturingTurnCoordinator::default();
    let captured_submit = coordinator.last_submit.clone();
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let envelope = sample_user_message_envelope("reply-prefix");
    service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let request = captured_submit
        .lock()
        .expect("captured submit lock poisoned")
        .clone()
        .expect("submit request captured");
    let reply_ref = request.reply_target_binding_ref.as_str();
    assert!(reply_ref.starts_with("reply:"));
    assert!(!reply_ref.starts_with("reply:reply:"));
    assert_eq!(reply_ref.matches("reply:").count(), 1);
}

#[tokio::test]
async fn max_valid_external_ids_do_not_overflow_turn_refs() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let long_event_id = "e".repeat(250);
    let envelope = sample_user_message_envelope(&long_event_id);
    service
        .accept_user_message(&envelope)
        .await
        .expect("long ids accepted");
}

#[tokio::test]
async fn overflowing_turn_ref_inputs_hash_deterministically() {
    let long_event_id = "e".repeat(250);
    let mut captured = Vec::new();

    for _ in 0..2 {
        let binding_service = FakeConversationBindingService::new();
        let thread_service = InMemorySessionThreadService::default();
        let coordinator = CapturingTurnCoordinator::default();
        let captured_submit = coordinator.last_submit.clone();
        let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

        let envelope = sample_user_message_envelope(&long_event_id);
        service
            .accept_user_message(&envelope)
            .await
            .expect("long id submit");
        let request = captured_submit
            .lock()
            .expect("captured submit lock poisoned")
            .clone()
            .expect("submit request captured");
        captured.push(request.idempotency_key.as_str().to_string());
    }

    assert_eq!(captured[0], captured[1]);
    assert!(captured[0].starts_with("turn:"));
    assert!(captured[0].len() < 64);
}

#[tokio::test]
async fn binding_failure_surfaces_workflow_error() {
    let binding_service = FakeConversationBindingService::new();
    binding_service.force_failure(ProductWorkflowError::BindingResolutionFailed {
        reason: "no tenant found".into(),
    });

    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let envelope = sample_user_message_envelope("fail1");
    let err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("should fail");

    assert!(matches!(
        err,
        ProductWorkflowError::BindingResolutionFailed { .. }
    ));
}
