use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RuntimeKind, TenantId, ThreadId};
use ironclaw_turns::{
    AgentLoopDriverDescriptor, LoopFailureKind, LoopMessageRef, RunProfileId, RunProfileVersion,
    TurnCheckpointId, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, CancellationPolicy,
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityDescriptorView,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceProfileId,
        CapabilitySurfaceVersion, CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass,
        ContextProfileId, FinalizeAssistantMessage, LoopCancelReasonKind, LoopCancellationPort,
        LoopCancellationSignal, LoopCheckpointKind, LoopCheckpointRequest, LoopCheckpointStateRef,
        LoopContextBundle, LoopContextRequest, LoopDriverId, LoopInputAck, LoopInputAckToken,
        LoopInputBatch, LoopInputCursor, LoopInputCursorToken, LoopModelMessage, LoopModelRequest,
        LoopModelResponse, LoopPromptBundle, LoopPromptBundleRef, LoopPromptBundleRequest,
        LoopRunContext, ModelProfileId, ModelStreamChunk, ParentLoopOutput, ProviderToolCallReplay,
        RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier,
        RunClassId, RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass,
        StageCheckpointPayloadRequest, SteeringPolicy, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};

use crate::{
    default_planner::DefaultPlanner,
    family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId},
    state::{CheckpointKind, GateStrategyState, LoopExecutionState},
    strategies::{
        CapabilityErrorClass, CapabilityErrorSummary, CapabilityFilter, CapabilityStrategy,
        GateHandlingStrategy, GateOutcome, GateSummary, InputDrainStrategy, ModelErrorSummary,
        RecoveryOutcome, RecoveryStrategy, RetryScope,
    },
};

#[derive(Clone)]
pub(super) struct MockHost {
    context: LoopRunContext,
    model_responses: Arc<Mutex<VecDeque<LoopModelResponse>>>,
    model_errors: Arc<Mutex<VecDeque<AgentLoopHostError>>>,
    model_requests: Arc<Mutex<Vec<LoopModelRequest>>>,
    prompt_requests: Arc<Mutex<Vec<LoopPromptBundleRequest>>>,
    input_batches: Arc<Mutex<VecDeque<LoopInputBatch>>>,
    acked_input_tokens: Arc<Mutex<Vec<LoopInputAckToken>>>,
    batch_outcomes: Arc<Mutex<VecDeque<ironclaw_turns::run_profile::CapabilityBatchOutcome>>>,
    single_outcomes: Arc<Mutex<VecDeque<CapabilityOutcome>>>,
    checkpoints: Arc<Mutex<Vec<LoopCheckpointKind>>>,
    batch_invocations: Arc<Mutex<Vec<CapabilityBatchInvocation>>>,
    single_invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    staged_payloads: Arc<Mutex<Vec<StageCheckpointPayloadRequest>>>,
    appended_result_refs: Arc<Mutex<Vec<AppendCapabilityResultRef>>>,
    events: Arc<Mutex<Vec<String>>>,
    prompt_surface_version: Option<CapabilitySurfaceVersion>,
    visible_surface_version: CapabilitySurfaceVersion,
    progress_events: Arc<Mutex<Vec<ironclaw_turns::run_profile::LoopProgressEvent>>>,
    fail_progress_port: bool,
    fail_append_result_ref: bool,
    cancellation: Arc<Mutex<Option<LoopCancellationSignal>>>,
    cancel_after_poll_inputs: Arc<Mutex<bool>>,
    cancel_after_prompt_bundle_count: Arc<Mutex<Option<usize>>>,
    cancel_after_checkpoint: Arc<Mutex<Option<LoopCheckpointKind>>>,
    cancel_after_model_response: Arc<Mutex<bool>>,
    cancel_after_batch_invocation: Arc<Mutex<bool>>,
    fail_checkpoint: Arc<Mutex<Option<LoopCheckpointKind>>>,
    fail_visible_capabilities: bool,
    fail_prompt_bundle: bool,
}

impl MockHost {
    pub(super) fn new(model_responses: Vec<LoopModelResponse>) -> Self {
        Self {
            context: test_run_context(),
            model_responses: Arc::new(Mutex::new(model_responses.into())),
            model_errors: Arc::new(Mutex::new(VecDeque::new())),
            model_requests: Arc::new(Mutex::new(Vec::new())),
            prompt_requests: Arc::new(Mutex::new(Vec::new())),
            input_batches: Arc::new(Mutex::new(VecDeque::new())),
            acked_input_tokens: Arc::new(Mutex::new(Vec::new())),
            batch_outcomes: Arc::new(Mutex::new(VecDeque::new())),
            single_outcomes: Arc::new(Mutex::new(VecDeque::new())),
            checkpoints: Arc::new(Mutex::new(Vec::new())),
            batch_invocations: Arc::new(Mutex::new(Vec::new())),
            single_invocations: Arc::new(Mutex::new(Vec::new())),
            staged_payloads: Arc::new(Mutex::new(Vec::new())),
            appended_result_refs: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            prompt_surface_version: Some(surface_version()),
            visible_surface_version: surface_version(),
            progress_events: Arc::new(Mutex::new(Vec::new())),
            fail_progress_port: false,
            fail_append_result_ref: false,
            cancellation: Arc::new(Mutex::new(None)),
            cancel_after_poll_inputs: Arc::new(Mutex::new(false)),
            cancel_after_prompt_bundle_count: Arc::new(Mutex::new(None)),
            cancel_after_checkpoint: Arc::new(Mutex::new(None)),
            cancel_after_model_response: Arc::new(Mutex::new(false)),
            cancel_after_batch_invocation: Arc::new(Mutex::new(false)),
            fail_checkpoint: Arc::new(Mutex::new(None)),
            fail_visible_capabilities: false,
            fail_prompt_bundle: false,
        }
    }

    pub(super) fn with_prompt_surface_version(
        mut self,
        version: Option<CapabilitySurfaceVersion>,
    ) -> Self {
        self.prompt_surface_version = version;
        self
    }

    pub(super) fn with_batch_outcomes(
        self,
        outcomes: Vec<ironclaw_turns::run_profile::CapabilityBatchOutcome>,
    ) -> Self {
        *self.batch_outcomes.lock().expect("lock") = outcomes.into();
        self
    }

    pub(super) fn with_single_outcomes(self, outcomes: Vec<CapabilityOutcome>) -> Self {
        *self.single_outcomes.lock().expect("lock") = outcomes.into();
        self
    }

    pub(super) fn with_model_errors(self, errors: Vec<AgentLoopHostError>) -> Self {
        *self.model_errors.lock().expect("lock") = errors.into();
        self
    }

    pub(super) fn with_failing_progress_port(mut self) -> Self {
        self.fail_progress_port = true;
        self
    }

    pub(super) fn with_failing_result_append(mut self) -> Self {
        self.fail_append_result_ref = true;
        self
    }

    pub(super) fn with_failing_visible_capabilities(mut self) -> Self {
        self.fail_visible_capabilities = true;
        self
    }

    pub(super) fn with_failing_prompt_bundle(mut self) -> Self {
        self.fail_prompt_bundle = true;
        self
    }

    pub(super) fn with_input_batches(self, batches: Vec<LoopInputBatch>) -> Self {
        *self.input_batches.lock().expect("lock") = batches.into();
        self
    }

    pub(super) fn checkpoint_kinds(&self) -> Vec<LoopCheckpointKind> {
        self.checkpoints.lock().expect("lock").clone()
    }

    pub(super) fn batch_invocations(&self) -> Vec<CapabilityBatchInvocation> {
        self.batch_invocations.lock().expect("lock").clone()
    }

    pub(super) fn single_invocations(&self) -> Vec<CapabilityInvocation> {
        self.single_invocations.lock().expect("lock").clone()
    }

    pub(super) fn model_requests(&self) -> Vec<LoopModelRequest> {
        self.model_requests.lock().expect("lock").clone()
    }

    pub(super) fn prompt_requests(&self) -> Vec<LoopPromptBundleRequest> {
        self.prompt_requests.lock().expect("lock").clone()
    }

    pub(super) fn acked_input_tokens(&self) -> Vec<LoopInputAckToken> {
        self.acked_input_tokens.lock().expect("lock").clone()
    }

    pub(super) fn staged_payloads(&self) -> Vec<StageCheckpointPayloadRequest> {
        self.staged_payloads.lock().expect("lock").clone()
    }

    pub(super) fn appended_result_refs(&self) -> Vec<AppendCapabilityResultRef> {
        self.appended_result_refs.lock().expect("lock").clone()
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.events.lock().expect("lock").clone()
    }

    pub(super) fn progress_events(&self) -> Vec<ironclaw_turns::run_profile::LoopProgressEvent> {
        self.progress_events.lock().expect("lock").clone()
    }

    pub(super) fn progress_event_names(&self) -> Vec<&'static str> {
        self.progress_events()
            .iter()
            .map(|event| event.kind_name())
            .collect()
    }

    pub(super) fn request_cancellation(&self, reason_kind: LoopCancelReasonKind) {
        *self.cancellation.lock().expect("lock") = Some(LoopCancellationSignal {
            reason_kind,
            requested_at: chrono::Utc::now(),
        });
    }

    pub(super) fn cancel_after_checkpoint(self, kind: LoopCheckpointKind) -> Self {
        *self.cancel_after_checkpoint.lock().expect("lock") = Some(kind);
        self
    }

    pub(super) fn cancel_after_poll_inputs(self) -> Self {
        *self.cancel_after_poll_inputs.lock().expect("lock") = true;
        self
    }

    pub(super) fn cancel_after_prompt_bundle(self, count: usize) -> Self {
        *self.cancel_after_prompt_bundle_count.lock().expect("lock") = Some(count);
        self
    }

    pub(super) fn cancel_after_model_response(self) -> Self {
        *self.cancel_after_model_response.lock().expect("lock") = true;
        self
    }

    pub(super) fn cancel_after_batch_invocation(self) -> Self {
        *self.cancel_after_batch_invocation.lock().expect("lock") = true;
        self
    }

    pub(super) fn fail_checkpoint(self, kind: LoopCheckpointKind) -> Self {
        *self.fail_checkpoint.lock().expect("lock") = Some(kind);
        self
    }

    pub(super) fn with_require_final_checkpoint(mut self, require_final_checkpoint: bool) -> Self {
        self.context
            .resolved_run_profile
            .checkpoint_policy
            .require_final_checkpoint = require_final_checkpoint;
        self
    }
}

pub(super) struct FixedCapabilityStrategy {
    filter: CapabilityFilter,
}

#[async_trait]
impl CapabilityStrategy for FixedCapabilityStrategy {
    async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
        self.filter.clone()
    }
}

pub(super) struct FixedDrainStrategy {
    drain_steering: bool,
    drain_followup: bool,
}

#[async_trait]
impl InputDrainStrategy for FixedDrainStrategy {
    async fn drain_steering(&self, _state: &LoopExecutionState) -> bool {
        self.drain_steering
    }

    async fn drain_followup(&self, _state: &LoopExecutionState) -> bool {
        self.drain_followup
    }
}

pub(super) struct FixedGateStrategy {
    outcome: GateOutcome,
}

#[async_trait]
impl GateHandlingStrategy for FixedGateStrategy {
    async fn handle(&self, _state: &LoopExecutionState, _gate: &GateSummary) -> GateOutcome {
        self.outcome.clone()
    }
}

pub(super) struct RetryPolicyDeniedRecoveryStrategy;

#[async_trait]
impl RecoveryStrategy for RetryPolicyDeniedRecoveryStrategy {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome {
        if err.class == CapabilityErrorClass::PolicyDenied {
            return RecoveryOutcome::Retry {
                recovery: state.recovery_state.clone(),
                scope: RetryScope::Call,
                alter: None,
            };
        }
        RecoveryOutcome::Abort {
            recovery: state.recovery_state.clone(),
            failure_kind: LoopFailureKind::CapabilityProtocolError,
        }
    }

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        _err: &ModelErrorSummary,
    ) -> RecoveryOutcome {
        RecoveryOutcome::Abort {
            recovery: state.recovery_state.clone(),
            failure_kind: LoopFailureKind::DriverBug,
        }
    }
}

impl ironclaw_turns::run_profile::LoopRunInfoPort for MockHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.context
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopContextPort for MockHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        Ok(LoopContextBundle {
            identity_messages: Vec::new(),
            messages: Vec::new(),
            instruction_snippets: Vec::new(),
            memory_snippets: Vec::new(),
        })
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopPromptPort for MockHost {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.prompt_requests.lock().expect("lock").push(request);
        if self.fail_prompt_bundle {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "prompt bundle unavailable",
            ));
        }
        let bundle = LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::for_run(&self.context, "bundle").expect("valid"),
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new("msg:user").expect("valid"),
            }],
            surface_version: self.prompt_surface_version.clone(),
            instruction_fingerprint: None,
            identity_message_count: 0,
            instruction_snippet_count: 0,
        };
        let should_cancel = {
            let mut remaining = self.cancel_after_prompt_bundle_count.lock().expect("lock");
            match remaining.as_mut() {
                Some(count) => {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        *remaining = None;
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        };
        if should_cancel {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        Ok(bundle)
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopInputPort for MockHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        if let Some(batch) = self.input_batches.lock().expect("lock").pop_front() {
            if *self.cancel_after_poll_inputs.lock().expect("lock") {
                self.request_cancellation(LoopCancelReasonKind::UserRequested);
            }
            return Ok(batch);
        }
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.events
            .lock()
            .expect("lock")
            .push("ack_inputs".to_string());
        self.acked_input_tokens.lock().expect("lock").extend(tokens);
        Ok(())
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopModelPort for MockHost {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.model_requests.lock().expect("lock").push(request);
        if let Some(error) = self.model_errors.lock().expect("lock").pop_front() {
            return Err(error);
        }
        let response = self
            .model_responses
            .lock()
            .expect("lock")
            .pop_front()
            .ok_or_else(|| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "model script exhausted")
            })?;
        if *self.cancel_after_model_response.lock().expect("lock") {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        Ok(response)
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCapabilityPort for MockHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        if self.fail_visible_capabilities {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "visible capabilities unavailable",
            ));
        }
        Ok(VisibleCapabilitySurface {
            version: self.visible_surface_version.clone(),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: capability_id(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: "demo".to_string(),
                safe_description: "demo capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{"input":{"type":"string"}}}),
            }],
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.single_invocations.lock().expect("lock").push(request);
        self.single_outcomes
            .lock()
            .expect("lock")
            .pop_front()
            .ok_or_else(|| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "single script exhausted")
            })
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<ironclaw_turns::run_profile::CapabilityBatchOutcome, AgentLoopHostError> {
        self.batch_invocations.lock().expect("lock").push(request);
        let outcome = self
            .batch_outcomes
            .lock()
            .expect("lock")
            .pop_front()
            .ok_or_else(|| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "batch script exhausted")
            })?;
        if *self.cancel_after_batch_invocation.lock().expect("lock") {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        Ok(outcome)
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopTranscriptPort for MockHost {
    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Ok(LoopMessageRef::new("msg:assistant").expect("valid"))
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        if self.fail_append_result_ref {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "append failed",
            ));
        }
        self.appended_result_refs
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(LoopMessageRef::new(format!(
            "msg:tool-result-{}",
            request.result_ref.as_str().trim_start_matches("result:")
        ))
        .expect("valid"))
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCheckpointPort for MockHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        self.events
            .lock()
            .expect("lock")
            .push(format!("checkpoint:{}", request.kind.as_str()));
        self.checkpoints.lock().expect("lock").push(request.kind);
        if self
            .fail_checkpoint
            .lock()
            .expect("lock")
            .is_some_and(|kind| kind == request.kind)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "scripted checkpoint failure",
            ));
        }
        if self
            .cancel_after_checkpoint
            .lock()
            .expect("lock")
            .is_some_and(|kind| kind == request.kind)
        {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        Ok(TurnCheckpointId::new())
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.staged_payloads.lock().expect("lock").push(request);
        LoopCheckpointStateRef::for_run(&self.context, "state")
            .map_err(|error| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error))
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopProgressPort for MockHost {
    async fn emit_loop_progress(
        &self,
        event: ironclaw_turns::run_profile::LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        if self.fail_progress_port {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "progress sink unavailable",
            ));
        }
        self.progress_events.lock().expect("lock").push(event);
        Ok(())
    }
}

impl LoopCancellationPort for MockHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        self.cancellation.lock().expect("lock").clone()
    }
}

pub(super) fn reply_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: vec![ModelStreamChunk {
            safe_text_delta: "hello".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(ironclaw_turns::run_profile::AssistantReply {
            content: "hello".to_string(),
        }),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn two_calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:first").expect("valid"), // safety: test-only fixture
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:second").expect("valid"), // safety: test-only fixture
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
        ]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"), // safety: test-only fixture
        usage: None,
    }
}

pub(super) fn provider_calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_1".to_string(),
                provider_tool_name: "demo__echo".to_string(),
                arguments: serde_json::json!({"message":"hello"}),
                response_reasoning: Some("response reasoning".to_string()),
                reasoning: Some("call reasoning".to_string()),
                signature: Some("sig-1".to_string()),
            }),
        }]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn provider_two_calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:first").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_1".to_string(),
                    provider_call_id: "call_1".to_string(),
                    provider_tool_name: "demo__echo".to_string(),
                    arguments: serde_json::json!({"message":"first"}),
                    response_reasoning: Some("response reasoning".to_string()),
                    reasoning: Some("first call reasoning".to_string()),
                    signature: Some("sig-1".to_string()),
                }),
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:second").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_1".to_string(),
                    provider_call_id: "call_2".to_string(),
                    provider_tool_name: "demo__echo".to_string(),
                    arguments: serde_json::json!({"message":"second"}),
                    response_reasoning: Some("response reasoning".to_string()),
                    reasoning: Some("second call reasoning".to_string()),
                    signature: Some("sig-2".to_string()),
                }),
            },
        ]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn stale_surface_calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            surface_version: stale_surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn mixed_surface_calls_response() -> LoopModelResponse {
    LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                surface_version: stale_surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:stale").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:visible").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
        ]),
        effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        usage: None,
    }
}

pub(super) fn capability_id() -> CapabilityId {
    CapabilityId::new("demo.echo").expect("valid")
}

pub(super) fn surface_version() -> CapabilitySurfaceVersion {
    CapabilitySurfaceVersion::new("surface:v1").expect("valid")
}

pub(super) fn stale_surface_version() -> CapabilitySurfaceVersion {
    CapabilitySurfaceVersion::new("surface:stale").expect("valid")
}

pub(super) fn input_cursor(context: &LoopRunContext, token: &str) -> LoopInputCursor {
    LoopInputCursor::from_host_token(
        context,
        LoopInputCursorToken::new(token).expect("valid input cursor token"),
    )
}

pub(super) fn input_ack(
    context: &LoopRunContext,
    cursor_token: &str,
    ack_token: &str,
) -> LoopInputAck {
    LoopInputAck {
        cursor: input_cursor(context, cursor_token),
        token: LoopInputAckToken::new(ack_token).expect("valid input ack token"),
    }
}

pub(super) fn message_ref(value: &str) -> LoopMessageRef {
    LoopMessageRef::new(value).expect("valid message ref")
}

pub(super) fn family_with_capability_filter(filter: CapabilityFilter) -> LoopFamily {
    let planner = DefaultPlanner::compose_default()
        .with_capability(Arc::new(FixedCapabilityStrategy { filter }));
    let id = LoopFamilyId::new("executor-filter-test").expect("valid test family id");
    let version = ComponentIdentity::from_static("executor-filter-test", ComponentDigest([1; 32]));
    LoopFamily::new(id, version, Arc::new(planner))
}

pub(super) fn family_with_drain(drain_steering: bool, drain_followup: bool) -> LoopFamily {
    let planner = DefaultPlanner::compose_default().with_drain(Arc::new(FixedDrainStrategy {
        drain_steering,
        drain_followup,
    }));
    let id = LoopFamilyId::new("executor-drain-test").expect("valid test family id");
    let version = ComponentIdentity::from_static("executor-drain-test", ComponentDigest([2; 32]));
    LoopFamily::new(id, version, Arc::new(planner))
}

pub(super) fn family_with_gate_outcome(outcome: GateOutcome) -> LoopFamily {
    let planner =
        DefaultPlanner::compose_default().with_gate(Arc::new(FixedGateStrategy { outcome }));
    let id = LoopFamilyId::new("executor-gate-test").expect("valid test family id");
    let version = ComponentIdentity::from_static("executor-gate-test", ComponentDigest([4; 32]));
    LoopFamily::new(id, version, Arc::new(planner))
}

pub(super) fn empty_gate_state() -> GateStrategyState {
    GateStrategyState::default()
}

pub(super) fn family_with_retry_policy_denied_recovery() -> LoopFamily {
    let planner = DefaultPlanner::compose_default()
        .with_recovery(Arc::new(RetryPolicyDeniedRecoveryStrategy));
    let id = LoopFamilyId::new("executor-retry-policy-denied-test").expect("valid test family id"); // safety: test-only fixture
    let version = ComponentIdentity::from_static(
        "executor-retry-policy-denied-test",
        ComponentDigest([3; 32]),
    );
    LoopFamily::new(id, version, Arc::new(planner))
}

pub(super) fn checkpoint_kind_from_host(kind: LoopCheckpointKind) -> CheckpointKind {
    match kind {
        LoopCheckpointKind::BeforeModel => CheckpointKind::BeforeModel,
        LoopCheckpointKind::BeforeSideEffect => CheckpointKind::BeforeSideEffect,
        LoopCheckpointKind::BeforeBlock => CheckpointKind::BeforeBlock,
        LoopCheckpointKind::Final => CheckpointKind::Final,
    }
}

pub(super) fn final_staged_state(host: &MockHost) -> LoopExecutionState {
    final_staged_state_for_kind(host, LoopCheckpointKind::Final)
}

pub(super) fn final_staged_state_for_kind(
    host: &MockHost,
    kind: LoopCheckpointKind,
) -> LoopExecutionState {
    let staged_payloads = host.staged_payloads();
    let final_payload = staged_payloads
        .iter()
        .rev()
        .find(|request| request.kind == kind)
        .expect("checkpoint payload"); // safety: test-only assertion
    LoopExecutionState::from_checkpoint_payload(
        &final_payload.payload,
        checkpoint_kind_from_host(kind),
    )
    .expect("checkpoint payload") // safety: test-only assertion
}

pub(super) fn test_run_context() -> LoopRunContext {
    let scope = TurnScope::new(
        TenantId::new("tenant-executor").expect("valid"),
        None,
        None,
        ThreadId::new("thread-executor").expect("valid"),
    );
    let descriptor = AgentLoopDriverDescriptor {
        id: LoopDriverId::new("executor_test_driver").expect("valid"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(
            CheckpointSchemaId::new("executor_test_checkpoint").expect("valid"),
        ),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let resolved_run_profile = ResolvedRunProfile {
        run_class_id: RunClassId::new("executor_test_class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor
            .checkpoint_schema_id
            .clone()
            .expect("descriptor checkpoint id"),
        checkpoint_schema_version: descriptor
            .checkpoint_schema_version
            .expect("descriptor checkpoint version"),
        model_profile_id: ModelProfileId::new("executor_test_model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new(
            "executor_test_capabilities",
        )
        .expect("valid"),
        context_profile_id: ContextProfileId::new("executor_test_context").expect("valid"),
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
            tier: ResourceBudgetTier::new("executor_test_tier").expect("valid"),
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
        resolution_fingerprint: RunProfileFingerprint::new("executor-test-fingerprint")
            .expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    };
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
}
