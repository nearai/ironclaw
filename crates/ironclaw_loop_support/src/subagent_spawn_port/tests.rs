use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_threads::{
    AcceptedInboundMessage, AcceptedInboundMessageReplay, AppendAssistantDraftRequest,
    AppendToolResultReferenceRequest, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    InMemorySessionThreadService, LatestThreadMessageRequest, ListThreadsForScopeRequest,
    ListThreadsForScopeResponse, LoadContextMessagesRequest, LoadContextWindowRequest,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageRecord,
    UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunResponse, EventCursor, GetRunStateRequest,
    InMemoryRunProfileResolver, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileResolutionRequest, RunProfileResolver, RunProfileVersion, SpawnTreeReservation,
    SubmitTurnRequest, TurnId, TurnRunProfile, TurnRunRecord, TurnRunState, TurnStateStore,
    TurnStatus,
    run_profile::{CapabilityResultMessage, CapabilitySurfaceVersion},
};
use serde_json::json;

use super::*;

struct StaticInputResolver {
    value: Result<serde_json::Value, AgentLoopHostError>,
}

struct StaticSpawnInputCodec {
    args: SpawnSubagentArgs,
}

struct StaticDefinitionResolver {
    resolved: Option<SubagentDefinition>,
    parent: Option<SubagentDefinition>,
}

struct AuthPassPort;

struct NoopResultWriter;

struct NoopGoalStore;

struct StaticCoordinator;

struct StaticTurnStateStore {
    record: Option<TurnRunRecord>,
    cancels: std::sync::Mutex<Vec<CancelRunRequest>>,
    releases: std::sync::Mutex<Vec<(TurnScope, TurnRunId, u32)>>,
}

#[derive(Default)]
struct RecordingChildRuns {
    requests: std::sync::Mutex<Vec<SubmitChildRunRequest>>,
}

#[derive(Default)]
struct RecordingGoalStore {
    puts: std::sync::Mutex<Vec<(TurnScope, TurnRunId, SubagentGoalRecord)>>,
    deletes: std::sync::Mutex<Vec<(TurnScope, TurnRunId)>>,
}

#[derive(Default)]
struct FailingMarkThreadService {
    inner: InMemorySessionThreadService,
}

impl StaticTurnStateStore {
    fn new(record: Option<TurnRunRecord>) -> Self {
        Self {
            record,
            cancels: std::sync::Mutex::new(Vec::new()),
            releases: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn cancels(&self) -> Vec<CancelRunRequest> {
        self.cancels.lock().unwrap().clone()
    }
}

impl RecordingChildRuns {
    fn requests(&self) -> Vec<SubmitChildRunRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RecordingGoalStore {
    fn puts(&self) -> Vec<(TurnScope, TurnRunId, SubagentGoalRecord)> {
        self.puts.lock().unwrap().clone()
    }

    fn deletes(&self) -> Vec<(TurnScope, TurnRunId)> {
        self.deletes.lock().unwrap().clone()
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for StaticInputResolver {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        self.value.clone()
    }
}

#[async_trait]
impl SpawnSubagentInputCodec for StaticSpawnInputCodec {
    async fn decode(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<SpawnSubagentArgs, AgentLoopHostError> {
        Ok(self.args.clone())
    }
}

#[async_trait]
impl SubagentDefinitionResolver for StaticDefinitionResolver {
    async fn resolve_kind(
        &self,
        _kind: &SubagentKindId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError> {
        Ok(self.resolved.clone())
    }

    async fn definition_of_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError> {
        Ok(self.parent.clone())
    }
}

#[async_trait]
impl LoopCapabilityPort for AuthPassPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            descriptors: Vec::new(),
        })
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new("result:auth").unwrap(),
            safe_summary: "authorized".to_string(),
            terminate_hint: false,
        }))
    }

    async fn invoke_capability_batch(
        &self,
        _request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        unreachable!("batch auth is not used by these tests")
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for NoopResultWriter {
    async fn write_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
        _invocation_id: InvocationId,
        _capability_id: &CapabilityId,
        _output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        Ok(LoopResultRef::new("result:spawn").unwrap())
    }
}

#[async_trait]
impl SubagentSpawnGoalStore for NoopGoalStore {
    async fn put_goal(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _goal: SubagentGoalRecord,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    async fn delete_goal(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

#[async_trait]
impl SubagentSpawnGoalStore for RecordingGoalStore {
    async fn put_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoalRecord,
    ) -> Result<(), AgentLoopHostError> {
        self.puts
            .lock()
            .unwrap()
            .push((scope.clone(), run_id, goal));
        Ok(())
    }

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), AgentLoopHostError> {
        self.deletes.lock().unwrap().push((scope.clone(), run_id));
        Ok(())
    }
}

#[async_trait]
impl TurnCoordinator for StaticCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("spawn early-return tests do not submit child turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("spawn tests do not resume turns")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("spawn tests do not cancel turns")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("spawn tests do not read run state through coordinator")
    }
}

#[async_trait]
impl TurnSpawnTreePort for StaticCoordinator {
    async fn submit_child_run(
        &self,
        _request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("spawn early-return tests do not submit child turns")
    }
}

#[async_trait]
impl TurnSpawnTreePort for RecordingChildRuns {
    async fn submit_child_run(
        &self,
        request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let run_id = request.requested_run_id.unwrap_or_default();
        let turn_id = TurnId::new();
        let accepted_message_ref = request.accepted_message_ref.clone();
        let reply_target_binding_ref = request.reply_target_binding_ref.clone();
        let resolved_run_profile_id = request
            .requested_run_profile
            .as_ref()
            .map(RunProfileId::from_request)
            .unwrap_or_else(RunProfileId::interactive_default);
        self.requests.lock().unwrap().push(request);
        Ok(SubmitTurnResponse::Accepted {
            turn_id,
            run_id,
            status: TurnStatus::Queued,
            resolved_run_profile_id,
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor(2),
            accepted_message_ref,
            reply_target_binding_ref,
        })
    }
}

#[async_trait]
impl SessionThreadService for FailingMarkThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.inner.replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "forced mark_message_submitted failure".to_string(),
        ))
    }

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_deferred_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_tool_result_reference(request).await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_tool_result_reference(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.inner.load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.inner.load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn latest_thread_message(
        &self,
        request: LatestThreadMessageRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.inner.latest_thread_message(request).await
    }

    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.read_thread(request).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.inner.create_summary_artifact(request).await
    }

    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        self.inner.list_threads_for_scope(request).await
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("spawn tests do not submit through state store")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("spawn tests do not resume through state store")
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancels.lock().unwrap().push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::CancelRequested,
            event_cursor: EventCursor(3),
            already_terminal: false,
            actor: None,
        })
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("spawn tests do not get run state")
    }
}

#[async_trait]
impl TurnSpawnTreeStateStore for StaticTurnStateStore {
    async fn submit_child_turn(
        &self,
        _request: ironclaw_turns::SubmitChildRunRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("spawn tests do not submit child turns through state store")
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }

    async fn get_run_record(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        Ok(self.record.clone())
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        _cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        Ok(SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id,
            descendant_count: u64::from(delta),
        })
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
    ) -> Result<(), TurnError> {
        self.releases
            .lock()
            .unwrap()
            .push((scope.clone(), root_run_id, delta));
        Ok(())
    }
}

fn input_ref() -> CapabilityInputRef {
    CapabilityInputRef::new("input:spawn").unwrap()
}

async fn test_run_context(label: &str) -> LoopRunContext {
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile resolves");
    LoopRunContext::new(
        TurnScope::new(
            TenantId::new(format!("tenant-{label}")).unwrap(),
            None,
            None,
            ThreadId::new(format!("thread-{label}")).unwrap(),
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}

async fn test_run_context_with_agent_actor(label: &str) -> LoopRunContext {
    let mut context = test_run_context(label).await.with_actor(TurnActor::new(
        UserId::new(format!("user-{label}")).unwrap(),
    ));
    context.scope.agent_id = Some(AgentId::new(format!("agent-{label}")).unwrap());
    context
}

fn default_spawn_args() -> SpawnSubagentArgs {
    SpawnSubagentArgs {
        subagent_kind: SubagentKindId::new("general").unwrap(),
        task: "task".to_string(),
        handoff: None,
        mode: SpawnSubagentMode::Blocking,
    }
}

fn subagent_definition(allow_nesting: bool) -> SubagentDefinition {
    SubagentDefinition {
        subagent_kind: SubagentKindId::new("general").unwrap(),
        allow_nesting,
        requested_run_profile: RunProfileRequest::new("subagent-test").unwrap(),
    }
}

fn turn_record(run_context: &LoopRunContext, subagent_depth: u32) -> TurnRunRecord {
    let lineage_root = (subagent_depth > 0).then(TurnRunId::new);
    TurnRunRecord {
        run_id: run_context.run_id,
        turn_id: run_context.turn_id,
        scope: run_context.scope.clone(),
        accepted_message_ref: AcceptedMessageRef::new("msg:parent").unwrap(),
        source_binding_ref: SourceBindingRef::new("source:parent").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:parent").unwrap(),
        status: TurnStatus::Queued,
        profile: TurnRunProfile::from_resolved(run_context.resolved_run_profile.clone()),
        resolved_model_route: None,
        checkpoint_id: None,
        gate_ref: None,
        failure: None,
        event_cursor: EventCursor(1),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: Utc::now(),
        parent_run_id: lineage_root,
        subagent_depth,
        spawn_tree_root_run_id: lineage_root,
    }
}

async fn spawn_test_port(
    run_context: LoopRunContext,
    limits: SubagentSpawnLimits,
    parent_subagent_depth: Option<u32>,
    resolver: StaticDefinitionResolver,
) -> SubagentSpawnCapabilityPort {
    let turn_store = Arc::new(StaticTurnStateStore::new(
        parent_subagent_depth.map(|depth| turn_record(&run_context, depth)),
    ));
    let coordinator: Arc<dyn TurnCoordinator> = Arc::new(StaticCoordinator);
    let child_runs: Arc<dyn TurnSpawnTreePort> = Arc::new(StaticCoordinator);
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator,
        child_runs,
        turn_state_store: turn_store,
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
        definition_resolver: Arc::new(resolver),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: default_spawn_args(),
        }),
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        run_context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        limits,
        deps,
    );
    port.auth_input_refs
        .lock()
        .unwrap()
        .insert(input_ref(), CapabilityInputRef::new("input:auth").unwrap());
    port
}

async fn invoke_spawn(port: &SubagentSpawnCapabilityPort) -> CapabilityOutcome {
    port.invoke_capability(CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
        capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        input_ref: input_ref(),
    })
    .await
    .unwrap()
}

fn denied_reason(outcome: CapabilityOutcome) -> String {
    let CapabilityOutcome::Denied(denied) = outcome else {
        panic!("expected denied outcome");
    };
    denied.reason_kind.as_str().to_string()
}

#[test]
fn spawn_args_store_normalized_mode() {
    let args = SpawnSubagentArgs {
        subagent_kind: SubagentKindId::new("general").unwrap(),
        task: "task".to_string(),
        handoff: None,
        mode: SpawnSubagentMode::Blocking,
    };
    assert_eq!(args.spawn_mode(), SpawnSubagentMode::Blocking);
}

#[tokio::test]
async fn invoke_spawn_rejects_when_fanout_cap_is_exceeded() {
    let context = test_run_context_with_agent_actor("spawn-fanout").await;
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits {
            max_spawn_per_turn: 0,
            ..SubagentSpawnLimits::default()
        },
        Some(0),
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        },
    )
    .await;

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "fanout_cap_exceeded"
    );
}

#[tokio::test]
async fn invoke_spawn_rejects_missing_agent_scope() {
    let mut context = test_run_context_with_agent_actor("spawn-agent-scope").await;
    context.scope.agent_id = None;
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits::default(),
        None,
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        },
    )
    .await;

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "spawn_requires_agent_scope"
    );
}

#[tokio::test]
async fn invoke_spawn_rejects_missing_actor() {
    let mut context = test_run_context("spawn-actor").await;
    context.scope.agent_id = Some(AgentId::new("agent-spawn-actor").unwrap());
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits::default(),
        None,
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        },
    )
    .await;

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "spawn_requires_actor"
    );
}

#[tokio::test]
async fn invoke_spawn_fails_when_parent_record_is_missing() {
    let context = test_run_context_with_agent_actor("spawn-parent-missing").await;
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits::default(),
        None,
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        },
    )
    .await;

    let error = port
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: input_ref(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(error.safe_summary.contains("parent run record not found"));
}

#[tokio::test]
async fn invoke_spawn_rejects_when_authorization_input_ref_is_missing() {
    let context = test_run_context_with_agent_actor("spawn-missing-auth-ref").await;
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        context.clone(),
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        Arc::new(SubagentSpawnDeps {
            coordinator: Arc::new(StaticCoordinator),
            child_runs: Arc::new(StaticCoordinator),
            turn_state_store: Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0)))),
            thread_service: Arc::new(InMemorySessionThreadService::default()),
            goal_store: Arc::new(NoopGoalStore),
            gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
            definition_resolver: Arc::new(StaticDefinitionResolver {
                resolved: Some(subagent_definition(false)),
                parent: None,
            }),
            spawn_input_codec: Arc::new(StaticSpawnInputCodec {
                args: default_spawn_args(),
            }),
            result_writer: Arc::new(NoopResultWriter),
        }),
    );

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "spawn_requires_provider_registration"
    );
}

#[tokio::test]
async fn invoke_spawn_submits_child_run_through_spawn_tree_port() {
    let context = test_run_context_with_agent_actor("spawn-success").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store,
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: goal_store.clone(),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: SpawnSubagentArgs {
                subagent_kind: SubagentKindId::new("general").unwrap(),
                task: "inspect the logs".to_string(),
                handoff: Some("return concise notes".to_string()),
                mode: SpawnSubagentMode::Blocking,
            },
        }),
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        context.clone(),
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
    );
    port.auth_input_refs
        .lock()
        .unwrap()
        .insert(input_ref(), CapabilityInputRef::new("input:auth").unwrap());

    let outcome = invoke_spawn(&port).await;

    let CapabilityOutcome::AwaitDependentRun {
        gate_ref,
        result_ref,
        ..
    } = outcome
    else {
        panic!("expected blocking child-run wait");
    };
    assert_eq!(gate_ref.as_str(), gate_store.records()[0].gate_ref.as_str());
    assert_eq!(result_ref.as_str(), "result:spawn");

    let requests = child_runs.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.parent_scope, context.scope);
    assert_eq!(request.parent_run_id, context.run_id);
    assert_eq!(
        request.spawn_tree_descendant_cap,
        DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS
    );
    assert_eq!(
        request.requested_run_profile.as_ref().unwrap().as_str(),
        "subagent-test"
    );
    assert!(request.requested_run_id.is_some_and(|run_id| {
        request
            .child_scope
            .thread_id
            .as_str()
            .contains(run_id.as_uuid().simple().to_string().as_str())
    }));

    let goals = goal_store.puts();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].2.task, "inspect the logs");
    assert_eq!(goals[0].2.handoff.as_deref(), Some("return concise notes"));

    let awaited = gate_store.records();
    assert_eq!(awaited.len(), 1);
    assert_eq!(awaited[0].parent_run_context.run_id, context.run_id);
    assert_eq!(awaited[0].child_scope, request.child_scope);
    assert_eq!(awaited[0].result_ref.as_str(), "result:spawn");
    assert_eq!(awaited[0].mode, SpawnSubagentMode::Blocking);
}

#[tokio::test]
async fn invoke_spawn_cancels_child_when_post_submit_thread_mark_fails() {
    let context = test_run_context_with_agent_actor("spawn-mark-fails").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store.clone(),
        thread_service: Arc::new(FailingMarkThreadService::default()),
        goal_store: goal_store.clone(),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: default_spawn_args(),
        }),
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
    );
    port.auth_input_refs
        .lock()
        .unwrap()
        .insert(input_ref(), CapabilityInputRef::new("input:auth").unwrap());

    let error = port
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: input_ref(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert!(error.safe_summary.contains("mark_message_submitted"));
    assert_eq!(child_runs.requests().len(), 1);
    let cancels = turn_store.cancels();
    assert_eq!(cancels.len(), 1);
    assert_eq!(
        Some(cancels[0].run_id),
        child_runs.requests()[0].requested_run_id
    );
    assert!(gate_store.records().is_empty());
    assert_eq!(goal_store.deletes().len(), 1);
}

#[tokio::test]
async fn invoke_spawn_rejects_depth_cap() {
    let context = test_run_context_with_agent_actor("spawn-depth").await;
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits {
            max_depth: 1,
            ..SubagentSpawnLimits::default()
        },
        Some(1),
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: Some(subagent_definition(true)),
        },
    )
    .await;

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "depth_cap_exceeded"
    );
}

#[tokio::test]
async fn invoke_spawn_rejects_subagent_parent_without_resolved_parent_flavor() {
    let context = test_run_context_with_agent_actor("spawn-nesting").await;
    let port = spawn_test_port(
        context,
        SubagentSpawnLimits {
            max_depth: 2,
            ..SubagentSpawnLimits::default()
        },
        Some(1),
        StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        },
    )
    .await;

    assert_eq!(
        denied_reason(invoke_spawn(&port).await),
        "nesting_not_permitted"
    );
}

#[tokio::test]
async fn json_spawn_input_codec_decodes_legacy_background_flag() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": "investigate",
            "run_in_background": true
        })),
    }));
    let context = test_run_context("spawn-codec").await;

    let args = codec.decode(&context, &input_ref()).await.unwrap();

    assert_eq!(args.subagent_kind.as_str(), "general");
    assert_eq!(args.task, "investigate");
    assert_eq!(args.spawn_mode(), SpawnSubagentMode::Background);
}

#[tokio::test]
async fn json_spawn_input_codec_rejects_invalid_shape() {
    let context = test_run_context("spawn-codec-invalid").await;
    for value in [
        json!({"task": "missing flavor"}),
        json!({"flavor_id": "general", "task": 42}),
        json!({"flavor_id": "general", "task": "task", "mode": "later"}),
    ] {
        let codec =
            JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver { value: Ok(value) }));

        let error = codec.decode(&context, &input_ref()).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("invalid spawn_subagent input"));
    }
}

#[tokio::test]
async fn json_spawn_input_codec_rejects_invalid_subagent_kind_ids() {
    let context = test_run_context("spawn-codec-invalid-kind").await;
    for flavor_id in [
        "",
        "kind with spaces",
        "general/researcher",
        "éxplorer",
        "x12345678901234567890123456789012345678901234567890123456789012345",
    ] {
        let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
            value: Ok(json!({
                "flavor_id": flavor_id,
                "task": "task"
            })),
        }));

        let error = codec.decode(&context, &input_ref()).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("invalid spawn_subagent input"));
    }
}

#[tokio::test]
async fn json_spawn_input_codec_propagates_resolver_error() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "input unavailable",
        )),
    }));
    let context = test_run_context("spawn-codec-error").await;

    let error = codec.decode(&context, &input_ref()).await.unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(error.safe_summary, "input unavailable");
}

#[test]
fn spawn_rejected_preserves_spawn_specific_reason_kind() {
    let CapabilityOutcome::Denied(denied) = spawn_rejected("depth_cap_exceeded") else {
        panic!("spawn_rejected should deny");
    };

    assert_eq!(denied.reason_kind.as_str(), "depth_cap_exceeded");
    assert!(denied.safe_summary.contains("depth_cap_exceeded"));
}

#[test]
fn child_submit_bindings_are_unique_per_prepared_child_run() {
    let parent_run_id = TurnRunId::new();
    let first_child = TurnRunId::new();
    let second_child = TurnRunId::new();

    assert_ne!(
        source_binding_ref(parent_run_id, first_child).unwrap(),
        source_binding_ref(parent_run_id, second_child).unwrap()
    );
    assert_ne!(
        reply_target_binding_ref(parent_run_id, first_child).unwrap(),
        reply_target_binding_ref(parent_run_id, second_child).unwrap()
    );
    assert_ne!(
        idempotency_key(parent_run_id, first_child).unwrap(),
        idempotency_key(parent_run_id, second_child).unwrap()
    );
}
