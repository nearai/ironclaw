use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_threads::{
    AcceptedInboundMessage, AcceptedInboundMessageReplay, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendToolResultReferenceRequest, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, InMemorySessionThreadService,
    LatestThreadMessageRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord, SummaryArtifact,
    ThreadHistory, ThreadHistoryRequest, ThreadMessageRecord, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
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

struct RegisteringSpawnInputCodec;

struct RejectingSpawnInputCodec {
    error: AgentLoopHostError,
}

struct FixedToolPort {
    definition: ProviderToolDefinition,
    capability_ids: ProviderToolCallCapabilityIds,
}

struct StaticDefinitionResolver {
    resolved: Option<SubagentDefinition>,
    parent: Option<SubagentDefinition>,
}

struct AuthPassPort;

#[derive(Default)]
struct SurfacePrimedSpawnAuthPort {
    visible_calls: std::sync::Mutex<u32>,
    register_calls: std::sync::Mutex<Vec<ProviderToolCall>>,
}

#[derive(Default)]
struct StrictSpawnAuthPort {
    visible_calls: std::sync::Mutex<u32>,
    register_calls: std::sync::Mutex<Vec<ProviderToolCall>>,
}

#[derive(Default)]
struct RecordingBatchPort {
    batches: std::sync::Mutex<Vec<CapabilityBatchInvocation>>,
}

struct FailingBatchPort;

#[derive(Default)]
struct SuspendedBatchPort {
    batches: std::sync::Mutex<Vec<CapabilityBatchInvocation>>,
}

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
impl SpawnSubagentInputCodec for RegisteringSpawnInputCodec {
    async fn decode(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<SpawnSubagentArgs, AgentLoopHostError> {
        Ok(default_spawn_args())
    }

    async fn register_provider_tool_call_input(
        &self,
        _run_context: &LoopRunContext,
        _tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        Ok(CapabilityInputRef::new("input:spawn-provider").unwrap())
    }
}

#[async_trait]
impl SpawnSubagentInputCodec for RejectingSpawnInputCodec {
    async fn decode(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<SpawnSubagentArgs, AgentLoopHostError> {
        Err(self.error.clone())
    }
}

#[async_trait]
impl LoopCapabilityPort for SurfacePrimedSpawnAuthPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        if *self.visible_calls.lock().unwrap() == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![spawn_tool_definition()])
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        if *self.visible_calls.lock().unwrap() == 0 {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "surface not primed",
            ));
        }
        self.register_calls.lock().unwrap().push(tool_call);
        Ok(CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            effective_capability_ids: vec![
                CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            ],
            input_ref: CapabilityInputRef::new("input:auth").unwrap(),
            provider_replay: None,
        })
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        *self.visible_calls.lock().unwrap() += 1;
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string(),
                safe_description: SPAWN_SUBAGENT_DESCRIPTION.to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: build_spawn_subagent_parameters_schema(&[]),
            }],
        })
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new("result:auth").unwrap(),
            safe_summary: "authorized".to_string(),
            progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 0,
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            outcomes.push(self.invoke_capability(invocation).await?);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for StrictSpawnAuthPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        if *self.visible_calls.lock().unwrap() == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![spawn_tool_definition()])
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.register_calls.lock().unwrap().push(tool_call);
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "strict inner does not accept spawn provider tool names",
        ))
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        *self.visible_calls.lock().unwrap() += 1;
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string(),
                safe_description: SPAWN_SUBAGENT_DESCRIPTION.to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: build_spawn_subagent_parameters_schema(&[]),
            }],
        })
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "strict inner should not authorize synthetic spawn provider calls",
        ))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            outcomes.push(self.invoke_capability(invocation).await?);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
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
            progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 0,
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            outcomes.push(self.invoke_capability(invocation).await?);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for FixedToolPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(vec![self.definition.clone()])
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if tool_call.name != self.definition.name {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is outside the visible capability surface",
            ));
        }
        Ok(self.capability_ids.clone())
    }

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
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(completed_outcome(request.capability_id.as_str()))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            outcomes.push(self.invoke_capability(invocation).await?);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingBatchPort {
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
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(completed_outcome(request.capability_id.as_str()))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.batches.lock().unwrap().push(request.clone());
        Ok(CapabilityBatchOutcome {
            outcomes: request
                .invocations
                .iter()
                .map(|invocation| completed_outcome(invocation.capability_id.as_str()))
                .collect(),
            stopped_on_suspension: false,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for SuspendedBatchPort {
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
        Ok(CapabilityOutcome::ApprovalRequired {
            gate_ref: LoopGateRef::new("gate:inner-suspended").unwrap(),
            safe_summary: "approval required".to_string(),
            approval_resume: None,
        })
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.batches.lock().unwrap().push(request);
        Ok(CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:inner-suspended").unwrap(),
                safe_summary: "approval required".to_string(),
                approval_resume: None,
            }],
            stopped_on_suspension: true,
        })
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for NoopResultWriter {
    async fn write_capability_result(
        &self,
        _write: CapabilityResultWrite<'_>,
    ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
        Ok((LoopResultRef::new("result:spawn").unwrap(), 0))
    }
}

#[async_trait]
impl LoopCapabilityPort for FailingBatchPort {
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
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(completed_outcome(request.capability_id.as_str()))
    }

    async fn invoke_capability_batch(
        &self,
        _request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "forced batch failure",
        ))
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

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        unreachable!("spawn tests do not retry turns")
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

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_capability_display_preview(request).await
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

    async fn delete_thread(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        self.inner.delete_thread(scope, thread_id).await
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

fn provider_tool_call(name: &str) -> ProviderToolCall {
    ProviderToolCall {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        turn_id: Some("provider-turn:test".to_string()),
        id: "call-spawn".to_string(),
        name: name.to_string(),
        arguments: json!({
            "flavor_id": "general",
            "task": "investigate"
        }),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

fn spawn_provider_tool_call() -> ProviderToolCall {
    provider_tool_call(SPAWN_SUBAGENT_PROVIDER_TOOL_NAME)
}

fn spawn_tool_definition() -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        name: SPAWN_SUBAGENT_PROVIDER_TOOL_NAME.to_string(),
        description: SPAWN_SUBAGENT_DESCRIPTION.to_string(),
        parameters: build_spawn_subagent_parameters_schema(&[]),
    }
}

fn custom_tool_definition() -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new("builtin.custom_tool").unwrap(),
        name: "demo__custom".to_string(),
        description: "Custom delegated tool".to_string(),
        parameters: json!({"type": "object"}),
    }
}

fn invocation(capability_id: &str) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
        capability_id: CapabilityId::new(capability_id).unwrap(),
        input_ref: input_ref(),
        approval_resume: None,
        auth_resume: None,
    }
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
    }
}

fn test_flavor_descriptor(id: &str, summary: &str) -> SpawnSubagentFlavorDescriptor {
    SpawnSubagentFlavorDescriptor {
        id: SubagentKindId::new(id).expect("test fixture: valid SubagentKindId"),
        summary: summary.to_string(),
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
        credential_requirements: Vec::new(),
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
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());
    port
}

fn spawn_test_port_with_inner(
    run_context: LoopRunContext,
    inner: Arc<dyn LoopCapabilityPort>,
    codec: Arc<dyn SpawnSubagentInputCodec>,
) -> SubagentSpawnCapabilityPort {
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: Arc::new(StaticCoordinator),
        turn_state_store: Arc::new(StaticTurnStateStore::new(Some(turn_record(
            &run_context,
            0,
        )))),
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: codec,
        result_writer: Arc::new(NoopResultWriter),
    });
    SubagentSpawnCapabilityPort::new(
        inner,
        run_context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    )
}

struct SpawnPortWithRecorders {
    port: SubagentSpawnCapabilityPort,
    child_runs: Arc<RecordingChildRuns>,
    goal_store: Arc<RecordingGoalStore>,
    gate_store: Arc<InMemorySubagentGateResolutionStore>,
}

fn spawn_test_port_with_codec_and_recorders(
    run_context: LoopRunContext,
    codec: Arc<dyn SpawnSubagentInputCodec>,
) -> SpawnPortWithRecorders {
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: Arc::new(StaticTurnStateStore::new(Some(turn_record(
            &run_context,
            0,
        )))),
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: goal_store.clone(),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: codec,
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        run_context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());
    SpawnPortWithRecorders {
        port,
        child_runs,
        goal_store,
        gate_store,
    }
}

async fn invoke_spawn(port: &SubagentSpawnCapabilityPort) -> CapabilityOutcome {
    port.invoke_capability(CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
        capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        input_ref: input_ref(),
        approval_resume: None,
        auth_resume: None,
    })
    .await
    .unwrap()
}

fn completed_outcome(label: &str) -> CapabilityOutcome {
    CapabilityOutcome::Completed(CapabilityResultMessage {
        result_ref: LoopResultRef::new(format!("result:{label}")).unwrap(),
        safe_summary: "completed".to_string(),
        progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: 0,
    })
}

fn denied_reason(outcome: CapabilityOutcome) -> String {
    let CapabilityOutcome::Denied(denied) = outcome else {
        panic!("expected denied outcome");
    };
    denied.reason_kind.as_str().to_string()
}

#[tokio::test]
async fn spawn_descriptor_is_present_in_visible_capabilities() {
    let context = test_run_context("spawn-visible").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible capabilities");
    let descriptors = surface
        .descriptors
        .iter()
        .filter(|descriptor| {
            descriptor.capability_id.as_str() == DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
        })
        .collect::<Vec<_>>();

    assert_eq!(descriptors.len(), 1);
    assert_eq!(
        descriptors[0].safe_name,
        DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
    );
    assert_eq!(descriptors[0].runtime, RuntimeKind::FirstParty);
    assert_eq!(descriptors[0].concurrency_hint, ConcurrencyHint::Exclusive);
}

#[tokio::test]
async fn spawn_descriptor_is_not_duplicated_when_inner_already_has_it() {
    let context = test_run_context("spawn-visible-dedup").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(SurfacePrimedSpawnAuthPort::default()),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible capabilities");

    assert_eq!(
        surface
            .descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.capability_id.as_str() == DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
            })
            .count(),
        1
    );
}

#[tokio::test]
async fn spawn_tool_definition_is_not_duplicated_when_inner_already_has_it() {
    let context = test_run_context("spawn-tools-dedup").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(FixedToolPort {
            definition: spawn_tool_definition(),
            capability_ids: ProviderToolCallCapabilityIds::single(
                CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            ),
        }),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let definitions = port.tool_definitions().expect("tool definitions");

    assert_eq!(
        definitions
            .iter()
            .filter(|definition| {
                definition.capability_id.as_str() == DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
            })
            .count(),
        1
    );
}

#[tokio::test]
async fn spawn_tool_definition_is_present_in_structured_tools() {
    let context = test_run_context("spawn-tools").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let definitions = port.tool_definitions().expect("tool definitions");
    let definition = definitions
        .iter()
        .find(|definition| {
            definition.capability_id.as_str() == DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
        })
        .expect("spawn tool definition");

    assert_eq!(definition.name, SPAWN_SUBAGENT_PROVIDER_TOOL_NAME);
    assert_eq!(
        definition.parameters["required"],
        json!(["subagent_type", "task"])
    );
    assert_eq!(
        definition.parameters["properties"]["task"]["maxLength"],
        json!(DEFAULT_SUBAGENT_GOAL_MAX_BYTES)
    );
    assert_eq!(
        definition.parameters["properties"]["handoff"]["maxLength"],
        json!(DEFAULT_SUBAGENT_GOAL_MAX_BYTES)
    );
    assert!(
        definition.parameters["properties"]["task"]["description"]
            .as_str()
            .expect("task description")
            .contains("UTF-8 byte budget")
    );
    assert!(
        definition.parameters["properties"]["handoff"]["description"]
            .as_str()
            .expect("handoff description")
            .contains("UTF-8 byte budget")
    );
    assert!(definition.parameters["properties"].get("handoff").is_some());
    assert!(definition.parameters["properties"].get("mode").is_none());
    assert!(
        definition.parameters["properties"]
            .get("run_in_background")
            .is_none()
    );
}

#[tokio::test]
async fn spawn_provider_tool_call_capability_ids_use_spawn_branch() {
    let context = test_run_context("spawn-capability-ids").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let ids = port
        .provider_tool_call_capability_ids(&spawn_provider_tool_call())
        .expect("spawn capability ids");

    assert_eq!(
        ids.provider_capability_id.as_str(),
        DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
    );
    assert_eq!(
        ids.effective_capability_ids,
        vec![CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap()]
    );
}

#[tokio::test]
async fn spawn_provider_tool_call_capability_ids_delegate_non_spawn_calls() {
    let context = test_run_context("spawn-capability-ids-delegate").await;
    let expected = ProviderToolCallCapabilityIds {
        provider_capability_id: CapabilityId::new("builtin.custom_tool").unwrap(),
        effective_capability_ids: vec![
            CapabilityId::new("builtin.custom_tool").unwrap(),
            CapabilityId::new("builtin.custom_tool.audit").unwrap(),
        ],
    };
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(FixedToolPort {
            definition: custom_tool_definition(),
            capability_ids: expected.clone(),
        }),
        Arc::new(RegisteringSpawnInputCodec),
    );

    let ids = port
        .provider_tool_call_capability_ids(&provider_tool_call("demo__custom"))
        .expect("delegated capability ids");

    assert_eq!(ids, expected);
}

#[tokio::test]
async fn spawn_provider_tool_call_validation_accepts_valid_spawn_args() {
    let context = test_run_context("spawn-validate").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );

    port.validate_provider_tool_call(&spawn_provider_tool_call())
        .expect("valid spawn provider tool call");
}

#[tokio::test]
async fn spawn_provider_tool_call_validation_rejects_invalid_spawn_args() {
    let context = test_run_context("spawn-validate-invalid").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );
    let mut call = spawn_provider_tool_call();
    call.arguments = json!({
        "flavor_id": "general",
        "task": "background task",
        "mode": "background"
    });

    let error = port
        .validate_provider_tool_call(&call)
        .expect_err("background spawn provider call rejects");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
}

#[tokio::test]
async fn spawn_provider_tool_call_validation_rejects_missing_or_malformed_flavor_id() {
    let context = test_run_context("spawn-validate-flavor").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );

    for arguments in [
        json!({
            "task": "investigate"
        }),
        json!({
            "flavor_id": 42,
            "task": "investigate"
        }),
    ] {
        let mut call = spawn_provider_tool_call();
        call.arguments = arguments;

        let error = port
            .validate_provider_tool_call(&call)
            .expect_err("invalid spawn provider tool call rejects");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("invalid spawn_subagent input"));
    }
}

#[tokio::test]
async fn spawn_provider_tool_call_validation_rejects_oversized_goal_fields() {
    let context = test_run_context("spawn-validate-goal-size").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );
    let oversized = "x".repeat(DEFAULT_SUBAGENT_GOAL_MAX_BYTES + 1);

    for (field, arguments) in [
        (
            "task",
            json!({
                "flavor_id": "general",
                "task": oversized.clone(),
            }),
        ),
        (
            "handoff",
            json!({
                "flavor_id": "general",
                "task": "investigate",
                "handoff": oversized,
            }),
        ),
    ] {
        let mut call = spawn_provider_tool_call();
        call.arguments = arguments;

        let error = port
            .validate_provider_tool_call(&call)
            .expect_err("oversized spawn provider tool call rejects");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            error
                .safe_summary
                .contains(&format!("spawn_subagent {field} is too large")),
            "unexpected error for {field}: {}",
            error.safe_summary
        );
    }
}

#[tokio::test]
async fn spawn_provider_tool_call_is_registered_for_spawn_dispatch() {
    let context = test_run_context("spawn-register").await;
    let inner = Arc::new(SurfacePrimedSpawnAuthPort::default());
    let port =
        spawn_test_port_with_inner(context, inner.clone(), Arc::new(RegisteringSpawnInputCodec));

    let candidate = port
        .register_provider_tool_call(spawn_provider_tool_call())
        .await
        .expect("provider tool call registration");

    assert_eq!(
        candidate.capability_id.as_str(),
        DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
    );
    assert_eq!(candidate.input_ref.as_str(), "input:spawn-provider");
    assert!(
        port.auth_input_refs
            .lock()
            .unwrap()
            .contains(&candidate.input_ref)
    );
    assert_eq!(*inner.visible_calls.lock().unwrap(), 1);
    assert_eq!(inner.register_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn spawn_provider_tool_call_registration_rejects_missing_turn_id() {
    let context = test_run_context("spawn-register-missing-turn-id").await;
    let inner = Arc::new(SurfacePrimedSpawnAuthPort::default());
    let port =
        spawn_test_port_with_inner(context, inner.clone(), Arc::new(RegisteringSpawnInputCodec));
    let mut call = spawn_provider_tool_call();
    call.turn_id = None;

    let error = port
        .register_provider_tool_call(call)
        .await
        .expect_err("provider tool call missing turn id rejects");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("provider tool call is missing a provider turn id")
    );
}

#[tokio::test]
async fn spawn_provider_tool_call_registration_does_not_require_inner_spawn_name() {
    let context = test_run_context_with_agent_actor("spawn-register-strict").await;
    let inner = Arc::new(StrictSpawnAuthPort::default());
    let child_runs = Arc::new(RecordingChildRuns::default());
    let port = SubagentSpawnCapabilityPort::new(
        inner.clone(),
        context.clone(),
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        Arc::new(SubagentSpawnDeps {
            coordinator: Arc::new(StaticCoordinator),
            child_runs: child_runs.clone(),
            turn_state_store: Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0)))),
            thread_service: Arc::new(InMemorySessionThreadService::default()),
            goal_store: Arc::new(NoopGoalStore),
            gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
            definition_resolver: Arc::new(StaticDefinitionResolver {
                resolved: Some(subagent_definition(false)),
                parent: None,
            }),
            spawn_input_codec: Arc::new(RegisteringSpawnInputCodec),
            result_writer: Arc::new(NoopResultWriter),
        }),
        Vec::new(),
    );

    let candidate = port
        .register_provider_tool_call(spawn_provider_tool_call())
        .await
        .expect("provider tool call registration");

    assert_eq!(
        candidate.capability_id.as_str(),
        DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
    );
    assert_eq!(candidate.input_ref.as_str(), "input:spawn-provider");
    assert_eq!(
        candidate
            .provider_replay
            .as_ref()
            .expect("provider replay")
            .provider_tool_name
            .as_str(),
        SPAWN_SUBAGENT_PROVIDER_TOOL_NAME
    );
    assert!(
        port.auth_input_refs
            .lock()
            .unwrap()
            .contains(&candidate.input_ref)
    );
    assert_eq!(*inner.visible_calls.lock().unwrap(), 1);
    assert_eq!(inner.register_calls.lock().unwrap().len(), 0);

    let outcome = port
        .invoke_capability(CapabilityInvocation {
            surface_version: candidate.surface_version.clone(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: candidate.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("registered spawn invocation");

    assert!(matches!(
        outcome,
        CapabilityOutcome::AwaitDependentRun { .. }
    ));
    assert_eq!(child_runs.requests().len(), 1);
}

#[test]
fn spawn_args_hold_blocking_runtime_request() {
    let args = SpawnSubagentArgs {
        subagent_kind: SubagentKindId::new("general").unwrap(),
        task: "task".to_string(),
        handoff: None,
    };
    assert_eq!(args.subagent_kind.as_str(), "general");
    assert_eq!(args.task, "task");
    assert_eq!(args.handoff, None);
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
            approval_resume: None,
            auth_resume: None,
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
        Vec::new(),
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
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

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
async fn invoke_capability_batch_handles_mixed_spawn_and_non_spawn_invocations() {
    let context = test_run_context_with_agent_actor("spawn-batch-mixed").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let inner = Arc::new(RecordingBatchPort::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: Arc::new(RecordingChildRuns::default()),
        turn_state_store: turn_store,
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
    });
    let port = SubagentSpawnCapabilityPort::new(
        inner.clone(),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![
                invocation("regular.one"),
                invocation(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID),
                invocation("regular.two"),
            ],
            stop_on_first_suspension: false,
        })
        .await
        .unwrap();

    assert_eq!(outcome.outcomes.len(), 3);
    assert!(!outcome.stopped_on_suspension);
    assert!(matches!(
        outcome.outcomes[1],
        CapabilityOutcome::AwaitDependentRun { .. }
    ));
    let batches = inner.batches.lock().unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[0].invocations[0].capability_id.as_str(),
        "regular.one"
    );
    assert_eq!(
        batches[1].invocations[0].capability_id.as_str(),
        "regular.two"
    );
}

#[tokio::test]
async fn invoke_capability_batch_rolls_back_preceding_spawn_on_inner_batch_failure() {
    let context = test_run_context_with_agent_actor("spawn-batch-rollback").await;
    let actor = context.actor.clone().unwrap();
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store.clone(),
        thread_service: thread_service.clone(),
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
        Arc::new(FailingBatchPort),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits {
            max_spawn_per_turn: 1,
            ..SubagentSpawnLimits::default()
        },
        deps,
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let error = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![
                invocation(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID),
                invocation("regular.fails"),
            ],
            stop_on_first_suspension: false,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    let child_requests = child_runs.requests();
    assert_eq!(child_requests.len(), 1);
    let child_request = &child_requests[0];
    let cancels = turn_store.cancels();
    assert_eq!(cancels.len(), 1);
    assert_eq!(Some(cancels[0].run_id), child_request.requested_run_id);
    assert!(gate_store.records().is_empty());
    assert_eq!(goal_store.deletes().len(), 1);
    assert_eq!(turn_store.releases.lock().unwrap().len(), 1);

    let child_thread_scope = ThreadScope {
        tenant_id: child_request.child_scope.tenant_id.clone(),
        agent_id: child_request.child_scope.agent_id.clone().unwrap(),
        project_id: child_request.child_scope.project_id.clone(),
        owner_user_id: Some(actor.user_id),
        mission_id: None,
    };
    let read = thread_service
        .read_thread(ThreadHistoryRequest {
            scope: child_thread_scope,
            thread_id: child_request.child_scope.thread_id.clone(),
        })
        .await;
    assert!(matches!(
        read,
        Err(SessionThreadError::UnknownThread { .. })
    ));

    port.auth_input_refs.lock().unwrap().insert(input_ref());
    assert!(
        matches!(
            invoke_spawn(&port).await,
            CapabilityOutcome::AwaitDependentRun { .. }
        ),
        "rolled-back batch spawns must release their per-turn spawn slot"
    );
}

#[tokio::test]
async fn invoke_capability_batch_stops_on_first_spawn_suspension_when_requested() {
    let context = test_run_context_with_agent_actor("spawn-batch-stop").await;
    let actor = context.actor.clone().unwrap();
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let inner = Arc::new(RecordingBatchPort::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store.clone(),
        thread_service: thread_service.clone(),
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
        inner.clone(),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![
                invocation(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID),
                invocation("regular.after"),
            ],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert_eq!(outcome.outcomes.len(), 1);
    assert!(outcome.stopped_on_suspension);
    assert!(inner.batches.lock().unwrap().is_empty());
    let child_requests = child_runs.requests();
    assert_eq!(child_requests.len(), 1);
    assert!(turn_store.cancels().is_empty());
    assert!(turn_store.releases.lock().unwrap().is_empty());
    assert!(goal_store.deletes().is_empty());
    assert_eq!(goal_store.puts().len(), 1);
    assert_eq!(gate_store.records().len(), 1);

    let child_request = &child_requests[0];
    let child_thread_scope = ThreadScope {
        tenant_id: child_request.child_scope.tenant_id.clone(),
        agent_id: child_request.child_scope.agent_id.clone().unwrap(),
        project_id: child_request.child_scope.project_id.clone(),
        owner_user_id: Some(actor.user_id),
        mission_id: None,
    };
    let read = thread_service
        .read_thread(ThreadHistoryRequest {
            scope: child_thread_scope,
            thread_id: child_request.child_scope.thread_id.clone(),
        })
        .await;
    assert!(
        read.is_ok(),
        "partial-success suspension must keep the child thread committed"
    );
}

#[tokio::test]
async fn invoke_capability_batch_preserves_spawns_on_inner_batch_suspension() {
    let context = test_run_context_with_agent_actor("spawn-inner-batch-stop").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let inner = Arc::new(SuspendedBatchPort::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store.clone(),
        thread_service,
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
        inner.clone(),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    let input_ref_a = CapabilityInputRef::new("input:spawn-a").unwrap();
    let input_ref_b = CapabilityInputRef::new("input:spawn-b").unwrap();
    {
        let mut refs = port.auth_input_refs.lock().unwrap();
        refs.insert(input_ref_a.clone());
        refs.insert(input_ref_b.clone());
    }

    let spawn_id = CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap();
    let inner_id = CapabilityId::new("inner.suspended").unwrap();
    let surface_version = CapabilitySurfaceVersion::new("surface:test").unwrap();
    let outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![
                CapabilityInvocation {
                    surface_version: surface_version.clone(),
                    capability_id: spawn_id.clone(),
                    input_ref: input_ref_a,
                    approval_resume: None,
                    auth_resume: None,
                },
                CapabilityInvocation {
                    surface_version: surface_version.clone(),
                    capability_id: spawn_id,
                    input_ref: input_ref_b,
                    approval_resume: None,
                    auth_resume: None,
                },
                CapabilityInvocation {
                    surface_version,
                    capability_id: inner_id,
                    input_ref: CapabilityInputRef::new("input:inner").unwrap(),
                    approval_resume: None,
                    auth_resume: None,
                },
            ],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert_eq!(outcome.outcomes.len(), 3);
    assert!(outcome.stopped_on_suspension);
    assert_eq!(inner.batches.lock().unwrap().len(), 1);
    assert_eq!(child_runs.requests().len(), 2);
    assert!(turn_store.cancels().is_empty());
    assert!(turn_store.releases.lock().unwrap().is_empty());
    assert!(goal_store.deletes().is_empty());
    assert_eq!(goal_store.puts().len(), 2);
    assert_eq!(gate_store.records().len(), 1);
}

#[tokio::test]
async fn invoke_spawn_cancels_child_when_post_submit_thread_mark_fails() {
    let context = test_run_context_with_agent_actor("spawn-mark-fails").await;
    let actor = context.actor.clone().unwrap();
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let goal_store = Arc::new(RecordingGoalStore::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let thread_service = Arc::new(FailingMarkThreadService::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store.clone(),
        thread_service: thread_service.clone(),
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
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let error = port
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: input_ref(),
            approval_resume: None,
            auth_resume: None,
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
    let child_requests = child_runs.requests();
    let child_request = &child_requests[0];
    let child_thread_scope = ThreadScope {
        tenant_id: child_request.child_scope.tenant_id.clone(),
        agent_id: child_request.child_scope.agent_id.clone().unwrap(),
        project_id: child_request.child_scope.project_id.clone(),
        owner_user_id: Some(actor.user_id),
        mission_id: None,
    };
    let read = thread_service
        .read_thread(ThreadHistoryRequest {
            scope: child_thread_scope,
            thread_id: child_request.child_scope.thread_id.clone(),
        })
        .await;
    assert!(matches!(
        read,
        Err(SessionThreadError::UnknownThread { .. })
    ));
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
async fn json_spawn_input_codec_rejects_legacy_background_flag() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": "investigate",
            "run_in_background": true
        })),
    }));
    let context = test_run_context("spawn-codec").await;

    let error = codec.decode(&context, &input_ref()).await.unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
}

#[tokio::test]
async fn json_spawn_input_codec_rejects_legacy_background_flag_even_with_blocking_mode() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": "investigate",
            "mode": "blocking",
            "run_in_background": true
        })),
    }));
    let context = test_run_context("spawn-codec-background-conflict").await;

    let error = codec.decode(&context, &input_ref()).await.unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
}

#[tokio::test]
async fn json_spawn_input_codec_rejects_background_mode() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": "investigate",
            "mode": "background"
        })),
    }));
    let context = test_run_context("spawn-codec-background").await;

    let error = codec.decode(&context, &input_ref()).await.unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
}

#[tokio::test]
async fn json_spawn_input_codec_defaults_to_blocking_when_mode_is_absent() {
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": "investigate"
        })),
    }));
    let context = test_run_context("spawn-codec-default").await;

    let args = codec.decode(&context, &input_ref()).await.unwrap();

    assert_eq!(args.subagent_kind.as_str(), "general");
    assert_eq!(args.task, "investigate");
}

#[tokio::test]
async fn json_spawn_input_codec_decode_accepts_subagent_type_canonical_key() {
    // Covers the codec decode path with the canonical `subagent_type` wire key.
    // Legacy `flavor_id` coverage already exists in
    // `json_spawn_input_codec_defaults_to_blocking_when_mode_is_absent`.
    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "subagent_type": "planner",
            "task": "build a plan"
        })),
    }));
    let context = test_run_context("spawn-codec-canonical-key").await;

    let args = codec.decode(&context, &input_ref()).await.unwrap();

    assert_eq!(args.subagent_kind.as_str(), "planner");
    assert_eq!(args.task, "build a plan");
}

#[tokio::test]
async fn json_spawn_input_codec_accepts_legacy_blocking_inputs() {
    for value in [
        json!({
            "flavor_id": "general",
            "task": "investigate",
            "mode": "blocking"
        }),
        json!({
            "flavor_id": "general",
            "task": "investigate",
            "run_in_background": false
        }),
    ] {
        let codec =
            JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver { value: Ok(value) }));
        let context = test_run_context("spawn-codec-legacy-blocking").await;

        let args = codec.decode(&context, &input_ref()).await.unwrap();

        assert_eq!(args.subagent_kind.as_str(), "general");
        assert_eq!(args.task, "investigate");
    }
}

#[tokio::test]
async fn invoke_spawn_propagates_decode_rejection_before_side_effects() {
    let context = test_run_context_with_agent_actor("spawn-background-disabled").await;
    let harness = spawn_test_port_with_codec_and_recorders(
        context,
        Arc::new(RejectingSpawnInputCodec {
            error: background_subagents_disabled(),
        }),
    );

    let error = harness
        .port
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: input_ref(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
    assert!(harness.child_runs.requests().is_empty());
    assert!(harness.goal_store.puts().is_empty());
    assert!(harness.gate_store.records().is_empty());
}

#[tokio::test]
async fn invoke_spawn_batch_propagates_decode_rejection_before_side_effects() {
    let context = test_run_context_with_agent_actor("spawn-background-disabled-batch").await;
    let harness = spawn_test_port_with_codec_and_recorders(
        context,
        Arc::new(RejectingSpawnInputCodec {
            error: background_subagents_disabled(),
        }),
    );

    let error = harness
        .port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
                surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
                capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
                input_ref: input_ref(),
                approval_resume: None,
                auth_resume: None,
            }],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("background subagents are disabled")
    );
    assert!(harness.child_runs.requests().is_empty());
    assert!(harness.goal_store.puts().is_empty());
    assert!(harness.gate_store.records().is_empty());
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
async fn json_spawn_input_codec_rejects_oversized_goal_fields() {
    let context = test_run_context("spawn-codec-oversized-goal").await;
    let oversized = "x".repeat(DEFAULT_SUBAGENT_GOAL_MAX_BYTES + 1);
    for (field, value) in [
        (
            "task",
            json!({
                "flavor_id": "general",
                "task": oversized.clone(),
            }),
        ),
        (
            "handoff",
            json!({
                "flavor_id": "general",
                "task": "investigate",
                "handoff": oversized,
            }),
        ),
    ] {
        let codec =
            JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver { value: Ok(value) }));

        let error = codec.decode(&context, &input_ref()).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            error
                .safe_summary
                .contains(&format!("spawn_subagent {field} is too large")),
            "unexpected error for {field}: {}",
            error.safe_summary
        );
    }
}

#[tokio::test]
async fn json_spawn_input_codec_uses_utf8_byte_budget_for_multibyte_goal_fields() {
    let context = test_run_context("spawn-codec-multibyte-goal").await;
    let multibyte = "\u{8a9e}";
    let oversized = multibyte.repeat(DEFAULT_SUBAGENT_GOAL_MAX_BYTES / multibyte.len() + 1);
    assert!(oversized.chars().count() <= DEFAULT_SUBAGENT_GOAL_MAX_BYTES);
    assert!(oversized.len() > DEFAULT_SUBAGENT_GOAL_MAX_BYTES);

    let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
        value: Ok(json!({
            "flavor_id": "general",
            "task": oversized,
        })),
    }));

    let error = codec.decode(&context, &input_ref()).await.unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error
            .safe_summary
            .contains("spawn_subagent task is too large")
    );
    assert!(error.safe_summary.contains("bytes"));
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

#[tokio::test]
async fn invoke_batch_coalesces_blocking_spawns_under_single_gate() {
    let context = test_run_context_with_agent_actor("spawn-batch-coalesce").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store,
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: SpawnSubagentArgs {
                subagent_kind: SubagentKindId::new("general").unwrap(),
                task: "shared task".to_string(),
                handoff: None,
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
        Vec::new(),
    );
    let input_ref_a = CapabilityInputRef::new("input:spawn-a").unwrap();
    let input_ref_b = CapabilityInputRef::new("input:spawn-b").unwrap();
    {
        let mut refs = port.auth_input_refs.lock().unwrap();
        refs.insert(input_ref_a.clone());
        refs.insert(input_ref_b.clone());
    }

    let make_invocation = |input_ref: CapabilityInputRef| CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
        capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        input_ref,
        approval_resume: None,
        auth_resume: None,
    };
    let batch_outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![make_invocation(input_ref_a), make_invocation(input_ref_b)],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert_eq!(batch_outcome.outcomes.len(), 2);
    assert!(
        !batch_outcome.stopped_on_suspension,
        "shared batch gate must suppress stop_on_first_suspension"
    );

    let mut gate_refs = Vec::new();
    for outcome in &batch_outcome.outcomes {
        let CapabilityOutcome::AwaitDependentRun { gate_ref, .. } = outcome else {
            panic!("expected await dependent run, got: {:?}", outcome);
        };
        gate_refs.push(gate_ref.as_str().to_string());
    }
    assert_eq!(
        gate_refs[0], gate_refs[1],
        "both blocking spawns must share the batch gate"
    );
    assert!(
        gate_refs[0].contains("subagent-batch"),
        "shared gate must use batch naming: {}",
        gate_refs[0]
    );
    let requests = child_runs.requests();
    assert_eq!(
        requests.len(),
        2,
        "both children submitted through spawn tree port"
    );
}

#[tokio::test]
async fn invoke_batch_mixed_spawn_and_non_spawn_capabilities() {
    let context = test_run_context_with_agent_actor("spawn-batch-mixed").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store,
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: SpawnSubagentArgs {
                subagent_kind: SubagentKindId::new("general").unwrap(),
                task: "shared task".to_string(),
                handoff: None,
            },
        }),
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    let input_ref_a = CapabilityInputRef::new("input:spawn-a").unwrap();
    let input_ref_inner = CapabilityInputRef::new("input:inner").unwrap();
    let input_ref_b = CapabilityInputRef::new("input:spawn-b").unwrap();
    {
        let mut refs = port.auth_input_refs.lock().unwrap();
        refs.insert(input_ref_a.clone());
        refs.insert(input_ref_b.clone());
    }

    let spawn_id = CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap();
    let inner_id = CapabilityId::new("inner.echo").unwrap();
    let surface_version = CapabilitySurfaceVersion::new("surface:test").unwrap();
    let batch_outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![
                CapabilityInvocation {
                    surface_version: surface_version.clone(),
                    capability_id: spawn_id.clone(),
                    input_ref: input_ref_a,
                    approval_resume: None,
                    auth_resume: None,
                },
                CapabilityInvocation {
                    surface_version: surface_version.clone(),
                    capability_id: inner_id,
                    input_ref: input_ref_inner,
                    approval_resume: None,
                    auth_resume: None,
                },
                CapabilityInvocation {
                    surface_version,
                    capability_id: spawn_id,
                    input_ref: input_ref_b,
                    approval_resume: None,
                    auth_resume: None,
                },
            ],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert_eq!(batch_outcome.outcomes.len(), 3);
    assert!(
        !batch_outcome.stopped_on_suspension,
        "shared spawn gate must not stop the mixed batch early"
    );
    let CapabilityOutcome::AwaitDependentRun {
        gate_ref: first_gate,
        ..
    } = &batch_outcome.outcomes[0]
    else {
        panic!("first outcome should be a blocking spawn");
    };
    let CapabilityOutcome::Completed(inner_result) = &batch_outcome.outcomes[1] else {
        panic!("second outcome should come from the inner non-spawn port");
    };
    let CapabilityOutcome::AwaitDependentRun {
        gate_ref: second_gate,
        ..
    } = &batch_outcome.outcomes[2]
    else {
        panic!("third outcome should be a blocking spawn");
    };
    assert_eq!(first_gate, second_gate);
    assert_eq!(inner_result.result_ref.as_str(), "result:auth");
    assert_eq!(child_runs.requests().len(), 2);
    let awaited = gate_store.records();
    assert_eq!(awaited.len(), 1);
    assert_eq!(awaited[0].gate_ref.as_str(), first_gate.as_str());
}

#[tokio::test]
async fn invoke_batch_skips_shared_gate_for_single_blocking_spawn() {
    let context = test_run_context_with_agent_actor("spawn-batch-single").await;
    let turn_store = Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0))));
    let child_runs = Arc::new(RecordingChildRuns::default());
    let gate_store = Arc::new(InMemorySubagentGateResolutionStore::default());
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
        turn_state_store: turn_store,
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: gate_store.clone(),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(StaticSpawnInputCodec {
            args: SpawnSubagentArgs {
                subagent_kind: SubagentKindId::new("general").unwrap(),
                task: "task".to_string(),
                handoff: None,
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
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let batch_outcome = port
        .invoke_capability_batch(CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
                surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
                capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
                input_ref: input_ref(),
                approval_resume: None,
                auth_resume: None,
            }],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    let CapabilityOutcome::AwaitDependentRun { gate_ref, .. } = &batch_outcome.outcomes[0] else {
        panic!("expected await dependent");
    };
    assert!(
        !gate_ref.as_str().contains("subagent-batch"),
        "single blocking spawn must not allocate batch gate: {}",
        gate_ref.as_str()
    );
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

/// Stub writer that returns a fixed non-zero byte_len.
struct FixedByteResultWriter {
    byte_len: u64,
}

#[async_trait]
impl LoopCapabilityResultWriter for FixedByteResultWriter {
    async fn write_capability_result(
        &self,
        _write: CapabilityResultWrite<'_>,
    ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
        Ok((
            LoopResultRef::new("result:fixed-bytes").unwrap(),
            self.byte_len,
        ))
    }
}

/// F5: Verify the CapabilityOutcome::AwaitDependentRun produced by the spawn
/// port carries the byte_len returned by the result writer. Tests that use
/// NoopResultWriter (byte_len=0) cannot catch a silent discard of this field.
#[tokio::test]
async fn spawn_subagent_propagates_byte_len_from_result_writer() {
    let context = test_run_context_with_agent_actor("spawn-byte-len").await;
    let child_runs = Arc::new(RecordingChildRuns::default());
    let fixed_byte_len: u64 = 42_000;
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: child_runs.clone(),
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
        result_writer: Arc::new(FixedByteResultWriter {
            byte_len: fixed_byte_len,
        }),
    });
    let port = SubagentSpawnCapabilityPort::new(
        Arc::new(AuthPassPort),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Vec::new(),
    );
    port.auth_input_refs.lock().unwrap().insert(input_ref());

    let outcome = invoke_spawn(&port).await;

    let CapabilityOutcome::AwaitDependentRun { byte_len, .. } = outcome else {
        panic!("expected AwaitDependentRun outcome from blocking spawn");
    };
    assert_eq!(
        byte_len, fixed_byte_len,
        "spawn port must propagate the byte_len returned by the result writer \
         (D2 un-discard regression: byte_len must reach CapabilityOutcome)"
    );
}

// ── New tests for schema redesign ────────────────────────────────────────────

#[test]
fn build_spawn_subagent_parameters_schema_enum_and_description() {
    let catalog = vec![
        test_flavor_descriptor("general", "summary one"),
        test_flavor_descriptor("planner", "summary two"),
    ];
    let schema = build_spawn_subagent_parameters_schema(&catalog);

    // required must list the new wire name
    assert_eq!(schema["required"], json!(["subagent_type", "task"]));

    // enum values come from catalog in order
    let enum_vals = schema["properties"]["subagent_type"]["enum"]
        .as_array()
        .expect("enum array");
    assert_eq!(enum_vals.len(), 2);
    assert_eq!(enum_vals[0], json!("general"));
    assert_eq!(enum_vals[1], json!("planner"));

    // description contains both summaries
    let description = schema["properties"]["subagent_type"]["description"]
        .as_str()
        .expect("description string");
    assert!(
        description.contains("summary one"),
        "description must contain 'summary one', got: {description}"
    );
    assert!(
        description.contains("summary two"),
        "description must contain 'summary two', got: {description}"
    );
}

#[test]
fn spawn_subagent_args_wire_rename_and_alias_roundtrip() {
    // Old wire format (flavor_id alias) must still deserialize
    let from_flavor_id: SpawnSubagentArgs =
        serde_json::from_str(r#"{"flavor_id":"general","task":"x"}"#)
            .expect("flavor_id alias must deserialize");
    assert_eq!(from_flavor_id.subagent_kind.as_str(), "general");

    // New wire format (subagent_type) must deserialize
    let from_subagent_type: SpawnSubagentArgs =
        serde_json::from_str(r#"{"subagent_type":"general","task":"x"}"#)
            .expect("subagent_type must deserialize");
    assert_eq!(from_subagent_type.subagent_kind.as_str(), "general");

    // Serialization emits subagent_type, not flavor_id
    let serialized = serde_json::to_value(&from_subagent_type).expect("serializes");
    assert!(
        serialized.get("subagent_type").is_some(),
        "serialized output must have 'subagent_type' key"
    );
    assert!(
        serialized.get("flavor_id").is_none(),
        "serialized output must NOT have 'flavor_id' key"
    );

    // When both the canonical name and alias are present, serde rejects the
    // input as a duplicate field. Document this behavior so any change in
    // serde's handling surfaces as a test failure.
    let duplicate_result = serde_json::from_str::<SpawnSubagentArgs>(
        r#"{"flavor_id":"old","subagent_type":"new","task":"x"}"#,
    );
    assert!(
        duplicate_result.is_err(),
        "serde_json must reject duplicate canonical+alias keys; got: {duplicate_result:?}"
    );
}

#[test]
fn spawn_subagent_description_contains_planner_nudge() {
    assert!(
        SPAWN_SUBAGENT_DESCRIPTION.contains("planner"),
        "SPAWN_SUBAGENT_DESCRIPTION must mention 'planner' to nudge parents toward planning"
    );
}

// ── Gap 1: empty catalog schema satisfiability ───────────────────────────────

#[test]
fn build_spawn_subagent_parameters_schema_empty_catalog_has_no_enum() {
    // After the empty-enum guard (commit 2b2b739a4), passing an empty catalog
    // must NOT emit an "enum" key — an `"enum": []` would make the schema
    // unsatisfiable (JSON Schema §6.1.2).
    let schema = build_spawn_subagent_parameters_schema(&[]);

    let subagent_type = &schema["properties"]["subagent_type"];

    // type and description must still be present so the model knows what to do.
    assert_eq!(
        subagent_type["type"].as_str().expect("type key"),
        "string",
        "subagent_type must be typed 'string' even for an empty catalog"
    );
    assert!(
        !subagent_type["description"]
            .as_str()
            .expect("description key")
            .is_empty(),
        "subagent_type must carry a non-empty description"
    );

    // enum must be absent — not an empty array, not null.
    assert!(
        subagent_type.get("enum").is_none(),
        "subagent_type must NOT have an 'enum' key when catalog is empty, \
         got: {subagent_type}"
    );
}

// ── C3: single-entry catalog produces a single-value enum ───────────────────

#[test]
fn build_spawn_subagent_parameters_schema_single_entry_catalog() {
    let catalog = vec![test_flavor_descriptor("solo", "the only one")];
    let schema = build_spawn_subagent_parameters_schema(&catalog);

    let enum_vals = schema["properties"]["subagent_type"]["enum"]
        .as_array()
        .expect("single-entry catalog must produce an 'enum' key");
    assert_eq!(enum_vals, &[serde_json::json!("solo")]);

    let description = schema["properties"]["subagent_type"]["description"]
        .as_str()
        .expect("description must be present");
    assert!(
        description.contains("solo"),
        "description must contain the flavor id 'solo', got: {description}"
    );
    assert!(
        description.contains("the only one"),
        "description must contain the summary 'the only one', got: {description}"
    );
}

// ── Gap 2: invoke path accepts subagent_type as canonical wire key ────────────

#[tokio::test]
async fn spawn_provider_tool_call_registration_accepts_subagent_type_wire_key() {
    // Exercises the full register→validate→invoke path using the canonical
    // wire key `subagent_type` (not the legacy `flavor_id` alias).
    // This catches wire-name handling bugs beyond the bare serde deser layer.
    let context = test_run_context_with_agent_actor("spawn-wire-subagent-type").await;
    let inner = Arc::new(StrictSpawnAuthPort::default());
    let child_runs = Arc::new(RecordingChildRuns::default());
    let port = SubagentSpawnCapabilityPort::new(
        inner.clone(),
        context.clone(),
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        Arc::new(SubagentSpawnDeps {
            coordinator: Arc::new(StaticCoordinator),
            child_runs: child_runs.clone(),
            turn_state_store: Arc::new(StaticTurnStateStore::new(Some(turn_record(&context, 0)))),
            thread_service: Arc::new(InMemorySessionThreadService::default()),
            goal_store: Arc::new(NoopGoalStore),
            gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
            definition_resolver: Arc::new(StaticDefinitionResolver {
                resolved: Some(subagent_definition(false)),
                parent: None,
            }),
            spawn_input_codec: Arc::new(RegisteringSpawnInputCodec),
            result_writer: Arc::new(NoopResultWriter),
        }),
        Vec::new(),
    );

    // Build a provider tool call using the canonical `subagent_type` key.
    let mut call = spawn_provider_tool_call();
    call.arguments = json!({
        "subagent_type": "general",
        "task": "investigate using canonical key"
    });

    // Register succeeds — validation accepts `subagent_type` as the wire key.
    let candidate = port
        .register_provider_tool_call(call)
        .await
        .expect("registration must succeed with subagent_type wire key");

    assert_eq!(
        candidate.capability_id.as_str(),
        DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID
    );
    assert_eq!(candidate.input_ref.as_str(), "input:spawn-provider");

    // Invoke the registered capability and assert the spawn is dispatched.
    let outcome = port
        .invoke_capability(CapabilityInvocation {
            surface_version: candidate.surface_version.clone(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: candidate.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("invocation must succeed");

    // A child run must have been submitted — the full invoke path ran.
    assert!(
        matches!(outcome, CapabilityOutcome::AwaitDependentRun { .. }),
        "invoke must produce AwaitDependentRun, got: {outcome:?}"
    );
    assert_eq!(
        child_runs.requests().len(),
        1,
        "exactly one child run must have been submitted"
    );
}

// ── C10: deny_unknown_fields rejects unexpected wire fields ───────────────────

#[tokio::test]
async fn spawn_provider_tool_call_registration_rejects_unknown_fields() {
    // Verifies that `deny_unknown_fields` on `SpawnSubagentWireArgs` is
    // enforced end-to-end through `register_provider_tool_call` (which calls
    // `validate_spawn_provider_tool_call` internally). An extra wire field
    // that is not part of the schema must yield `InvalidInvocation` with the
    // existing rejection message.
    let context = test_run_context("spawn-deny-unknown-fields").await;
    let port = spawn_test_port_with_inner(
        context,
        Arc::new(AuthPassPort),
        Arc::new(RegisteringSpawnInputCodec),
    );
    let mut call = spawn_provider_tool_call();
    call.arguments = json!({
        "flavor_id": "general",
        "task": "investigate",
        "unknown_field": "bogus"
    });

    let error = port
        .validate_provider_tool_call(&call)
        .expect_err("payload with unknown_field must be rejected");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error.safe_summary.contains("invalid spawn_subagent input"),
        "rejection message must contain 'invalid spawn_subagent input', got: {}",
        error.safe_summary
    );
}

// ── C11: new_with_schema propagates precomputed schema to tool_definition ─────

#[tokio::test]
async fn new_with_schema_propagates_schema_to_spawn_tool_definition() {
    // Constructs a port via `new_with_schema` with a recognisable marker schema
    // and asserts that `spawn_tool_definition` (called via `tool_definitions`)
    // renders the marker into the resulting `ProviderToolDefinition.parameters`.
    let context = test_run_context("spawn-new-with-schema").await;
    let marker_schema = Arc::new(serde_json::json!({
        "type": "object",
        "description": "MARKER_SCHEMA_FOR_TEST",
        "properties": {}
    }));
    let deps = Arc::new(SubagentSpawnDeps {
        coordinator: Arc::new(StaticCoordinator),
        child_runs: Arc::new(StaticCoordinator),
        turn_state_store: Arc::new(StaticTurnStateStore::new(None)),
        thread_service: Arc::new(ironclaw_threads::InMemorySessionThreadService::default()),
        goal_store: Arc::new(NoopGoalStore),
        gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
        definition_resolver: Arc::new(StaticDefinitionResolver {
            resolved: Some(subagent_definition(false)),
            parent: None,
        }),
        spawn_input_codec: Arc::new(RegisteringSpawnInputCodec),
        result_writer: Arc::new(NoopResultWriter),
    });
    let port = SubagentSpawnCapabilityPort::new_with_schema(
        Arc::new(AuthPassPort),
        context,
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        SubagentSpawnLimits::default(),
        deps,
        Arc::clone(&marker_schema),
    );

    let definitions = port.tool_definitions().expect("tool definitions");
    let spawn_def = definitions
        .iter()
        .find(|d| d.capability_id.as_str() == DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
        .expect("spawn tool definition must be present");

    assert_eq!(
        spawn_def.parameters["description"],
        serde_json::json!("MARKER_SCHEMA_FOR_TEST"),
        "parameters must carry the marker schema injected via new_with_schema"
    );
}
