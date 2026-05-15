//! Feature-gated fixtures for loop-family integration tests.
//!
//! The module is intentionally absent from normal production builds. Tests in
//! downstream crates can enable `ironclaw_agent_loop/test-support` and drive
//! the canonical executor through the same host trait used by Reborn.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RuntimeKind, TenantId, ThreadId};
use ironclaw_turns::{
    AgentLoopDriverDescriptor, LoopFailureKind, LoopGateRef, LoopMessageRef, LoopResultRef,
    RunProfileId, RunProfileVersion, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply, CancellationPolicy,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityCallCandidate,
        CapabilityDescriptorView, CapabilityFailure, CapabilityFailureKind, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        CapabilitySurfaceProfileId, CapabilitySurfaceVersion, CheckpointPolicy, CheckpointSchemaId,
        ConcurrencyClass, ConcurrencyHint, ContextProfileId, FinalizeAssistantMessage,
        LoopCancellationPort, LoopCancellationSignal, LoopCheckpointKind, LoopCheckpointRequest,
        LoopCheckpointStateRef, LoopContextBundle, LoopContextRequest, LoopDriverId, LoopInput,
        LoopInputAck, LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputCursorToken,
        LoopModelMessage, LoopModelRequest, LoopModelResponse, LoopProgressEvent, LoopPromptBundle,
        LoopPromptBundleRef, LoopPromptBundleRequest, LoopRunContext, LoopRunInfoPort,
        ModelProfileId, ModelStreamChunk, ParentLoopOutput, RedactedRunProfileProvenance,
        ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier, RunClassId,
        RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass,
        StageCheckpointPayloadRequest, SteeringPolicy, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};

use crate::state::{
    CapabilityCallSignature, CheckpointKind, LoopExecutionState, RecoveryAttemptClass,
    RecoveryStrategyState,
};

/// Scriptable implementation of [`AgentLoopDriverHost`].
///
/// Every port call is recorded into the call log. Model responses, capability
/// batches, single-call retries, pending inputs, and selected host failures are
/// all driven by [`ScenarioScript`].
pub struct MockAgentLoopDriverHost {
    run_context: LoopRunContext,
    script: Mutex<ScenarioScript>,
    call_log: Mutex<Vec<MockHostCall>>,
    checkpoints: Arc<CheckpointRecorder>,
    visible_capabilities: Vec<CapabilityDescriptorView>,
    staged_iterations: Mutex<VecDeque<u32>>,
    fail_prompt_with: Mutex<Option<AgentLoopHostErrorKind>>,
    fail_model_with: Mutex<Option<AgentLoopHostErrorKind>>,
    acked_tokens: Mutex<Vec<LoopInputAckToken>>,
    cancellation: Mutex<Option<LoopCancellationSignal>>,
}

impl MockAgentLoopDriverHost {
    /// Starts a new builder with the default test run context.
    pub fn builder() -> MockAgentLoopDriverHostBuilder {
        MockAgentLoopDriverHostBuilder::new()
    }

    /// Returns the ordered host call log captured so far.
    pub fn call_log(&self) -> Vec<MockHostCall> {
        clone_mutex_vec(&self.call_log)
    }

    /// Returns how many model stream calls the executor made.
    pub fn model_call_count(&self) -> usize {
        self.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::StreamModel))
            .count()
    }

    /// Returns all ack tokens passed to [`ack_inputs`] so far, in call order.
    pub fn acked_tokens(&self) -> Vec<LoopInputAckToken> {
        lock_or_panic(&self.acked_tokens).clone()
    }

    fn record_call(&self, call: MockHostCall) {
        lock_or_panic(&self.call_log).push(call);
    }
}

/// Builder for [`MockAgentLoopDriverHost`].
pub struct MockAgentLoopDriverHostBuilder {
    run_context: LoopRunContext,
    script: ScenarioScript,
    visible_capabilities: Vec<CapabilityDescriptorView>,
    fail_prompt_with: Option<AgentLoopHostErrorKind>,
    fail_model_with: Option<AgentLoopHostErrorKind>,
    cancellation: Option<LoopCancellationSignal>,
}

impl MockAgentLoopDriverHostBuilder {
    /// Creates a builder using [`ScenarioScript::reply_only`].
    pub fn new() -> Self {
        Self {
            run_context: test_run_context("agent-loop-test"),
            script: ScenarioScript::reply_only("ok"),
            visible_capabilities: vec![capability_descriptor(
                capability_id("demo.echo"),
                ConcurrencyHint::SafeForParallel,
            )],
            fail_prompt_with: None,
            fail_model_with: None,
            cancellation: None,
        }
    }

    /// Overrides the run context.
    pub fn run_context(mut self, context: LoopRunContext) -> Self {
        self.run_context = context;
        self
    }

    /// Sets the host script.
    pub fn script(mut self, script: ScenarioScript) -> Self {
        self.script = script;
        self
    }

    /// Overrides the visible capability surface descriptors.
    pub fn visible_capabilities(mut self, descriptors: Vec<CapabilityDescriptorView>) -> Self {
        self.visible_capabilities = descriptors;
        self
    }

    /// Forces every model call to fail with the selected host error kind.
    pub fn fail_model_with(mut self, kind: AgentLoopHostErrorKind) -> Self {
        self.fail_model_with = Some(kind);
        self
    }

    /// Forces every prompt-build call to fail with the selected host error kind.
    pub fn fail_prompt_with(mut self, kind: AgentLoopHostErrorKind) -> Self {
        self.fail_prompt_with = Some(kind);
        self
    }

    /// Sets the cancellation signal returned by the host accessor.
    pub fn cancellation_signal(mut self, signal: LoopCancellationSignal) -> Self {
        self.cancellation = Some(signal);
        self
    }

    /// Builds the host and its shared checkpoint recorder.
    pub fn build(self) -> (MockAgentLoopDriverHost, Arc<CheckpointRecorder>) {
        let checkpoints = Arc::new(CheckpointRecorder::default());
        (
            MockAgentLoopDriverHost {
                run_context: self.run_context,
                script: Mutex::new(self.script),
                call_log: Mutex::new(Vec::new()),
                checkpoints: checkpoints.clone(),
                visible_capabilities: self.visible_capabilities,
                staged_iterations: Mutex::new(VecDeque::new()),
                fail_prompt_with: Mutex::new(self.fail_prompt_with),
                fail_model_with: Mutex::new(self.fail_model_with),
                acked_tokens: Mutex::new(Vec::new()),
                cancellation: Mutex::new(self.cancellation),
            },
            checkpoints,
        )
    }
}

impl Default for MockAgentLoopDriverHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Ordered call emitted by [`MockAgentLoopDriverHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockHostCall {
    /// Prompt bundle construction was requested.
    BuildPromptBundle,
    /// The model stream port was invoked.
    StreamModel,
    /// A batch capability invocation was requested.
    InvokeCapabilityBatch {
        /// Number of calls in the batch.
        call_count: usize,
        /// Whether the executor requested stop-on-first-suspension.
        stop_on_first_suspension: bool,
    },
    /// A single capability retry was requested.
    InvokeCapability {
        /// Capability id used for the retry.
        capability_id: CapabilityId,
    },
    /// Assistant reply finalization was requested.
    FinalizeAssistantMessage,
    /// A checkpoint metadata write was requested.
    SaveCheckpoint(CheckpointKind),
    /// Pending inputs were polled.
    PollInputs,
    /// Pending inputs were acknowledged.
    AckInputs,
    /// Visible capabilities were loaded.
    VisibleCapabilities,
    /// Checkpoint payload bytes were staged.
    StageCheckpointPayload(CheckpointKind),
}

/// Script consumed by [`MockAgentLoopDriverHost`].
#[derive(Debug, Clone)]
pub struct ScenarioScript {
    /// Model responses in call order.
    pub model_responses: VecDeque<ScriptedModelResponse>,
    /// Batch outcomes in invocation order.
    pub capability_outcomes: VecDeque<Vec<ScriptedCapabilityOutcome>>,
    /// Single-call retry outcomes in invocation order.
    pub single_call_retry_outcomes: VecDeque<ScriptedCapabilityOutcome>,
    /// Pending input batches in poll order.
    pub pending_inputs: VecDeque<Vec<LoopInput>>,
}

impl ScenarioScript {
    /// Creates a script whose first model call returns an assistant reply.
    pub fn reply_only(text: impl Into<String>) -> Self {
        Self {
            model_responses: VecDeque::from([ScriptedModelResponse::Reply { text: text.into() }]),
            capability_outcomes: VecDeque::new(),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        }
    }

    /// Creates a script whose first model call returns one capability call and
    /// whose second model call returns a reply after the batch completes.
    pub fn calls_then_reply(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            model_responses: VecDeque::from([
                ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new(name)]),
                ScriptedModelResponse::Reply {
                    text: "done".to_string(),
                },
            ]),
            capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
                "result:done",
            )]]),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        }
    }

    /// Creates a script whose model repeats the same single capability call.
    pub fn same_calls_repeated(name: impl Into<String>, count: usize) -> Self {
        let name = name.into();
        Self {
            model_responses: (0..count)
                .map(|_| {
                    ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new(name.clone())])
                })
                .collect(),
            capability_outcomes: (0..count)
                .map(|_| vec![ScriptedCapabilityOutcome::completed("result:repeat")])
                .collect(),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        }
    }

    /// Creates a script whose first capability batch requires approval.
    pub fn approval_required(name: impl Into<String>) -> Self {
        Self {
            model_responses: VecDeque::from([ScriptedModelResponse::Calls(vec![
                ScriptedCapabilityCall::new(name.into()),
            ])]),
            capability_outcomes: VecDeque::from([vec![
                ScriptedCapabilityOutcome::ApprovalRequired {
                    gate_ref: "gate:approval".to_string(),
                },
            ]]),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        }
    }

    /// Creates a script with repeated failures for the same capability call.
    pub fn same_failure_repeated(
        name: impl Into<String>,
        kind: impl Into<String>,
        count: usize,
    ) -> Self {
        let name = name.into();
        let kind = kind.into();
        Self {
            model_responses: (0..count)
                .map(|_| {
                    ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new(name.clone())])
                })
                .collect(),
            capability_outcomes: (0..count)
                .map(|_| vec![ScriptedCapabilityOutcome::failed(kind.clone())])
                .collect(),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        }
    }

    /// Replaces batch outcomes.
    pub fn with_capability_outcomes(
        mut self,
        outcomes: Vec<Vec<ScriptedCapabilityOutcome>>,
    ) -> Self {
        self.capability_outcomes = outcomes.into();
        self
    }

    /// Replaces single-call retry outcomes.
    pub fn with_single_call_retry_outcomes(
        mut self,
        outcomes: Vec<ScriptedCapabilityOutcome>,
    ) -> Self {
        self.single_call_retry_outcomes = outcomes.into();
        self
    }
}

/// Scripted model response.
#[derive(Debug, Clone)]
pub enum ScriptedModelResponse {
    /// Return an assistant reply.
    Reply {
        /// Reply text.
        text: String,
    },
    /// Return capability calls.
    Calls(Vec<ScriptedCapabilityCall>),
    /// Return a sanitized host error.
    Error {
        /// Host error kind to return.
        kind: AgentLoopHostErrorKind,
    },
}

/// Scripted capability call candidate.
#[derive(Debug, Clone)]
pub struct ScriptedCapabilityCall {
    /// Capability id string.
    pub name: String,
    /// Input ref string.
    pub input_ref: String,
}

impl ScriptedCapabilityCall {
    /// Creates a call with a deterministic input ref derived from the name.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            input_ref: format!("input:{}", safe_ref_suffix(&name)),
            name,
        }
    }
}

/// Scripted capability outcome.
#[derive(Debug, Clone)]
pub enum ScriptedCapabilityOutcome {
    /// Completed result.
    Completed {
        /// Result ref.
        result_ref: String,
        /// Whether this result should naturally end the loop.
        terminate_hint: bool,
    },
    /// Approval gate.
    ApprovalRequired {
        /// Gate ref.
        gate_ref: String,
    },
    /// Auth gate.
    AuthRequired {
        /// Gate ref.
        gate_ref: String,
    },
    /// Resource gate.
    ResourceBlocked {
        /// Gate ref.
        gate_ref: String,
    },
    /// Failed result.
    Failed {
        /// Error kind string consumed by the executor classifier.
        error_kind: CapabilityFailureKind,
    },
}

impl ScriptedCapabilityOutcome {
    /// Creates a completed outcome with `terminate_hint = false`.
    pub fn completed(result_ref: impl Into<String>) -> Self {
        Self::Completed {
            result_ref: result_ref.into(),
            terminate_hint: false,
        }
    }

    /// Creates a completed outcome with `terminate_hint = true`.
    pub fn completed_with_terminate_hint(result_ref: impl Into<String>) -> Self {
        Self::Completed {
            result_ref: result_ref.into(),
            terminate_hint: true,
        }
    }

    /// Creates a failed outcome using the provided error kind.
    pub fn failed(error_kind: impl AsRef<str>) -> Self {
        Self::Failed {
            error_kind: scripted_failure_kind(error_kind.as_ref()),
        }
    }
}

/// Captures checkpoint write order and the state iteration at each boundary.
#[derive(Debug, Default)]
pub struct CheckpointRecorder {
    sequence: Mutex<Vec<(CheckpointKind, u32)>>,
}

impl CheckpointRecorder {
    /// Records one checkpoint boundary.
    pub fn record(&self, kind: CheckpointKind, iteration: u32) {
        lock_or_panic(&self.sequence).push((kind, iteration));
    }

    /// Returns the recorded `(kind, iteration)` sequence.
    pub fn sequence(&self) -> Vec<(CheckpointKind, u32)> {
        clone_mutex_vec(&self.sequence)
    }

    /// Returns just the recorded checkpoint kinds.
    pub fn kinds(&self) -> Vec<CheckpointKind> {
        self.sequence().into_iter().map(|(kind, _)| kind).collect()
    }

    /// Asserts the exact checkpoint sequence.
    pub fn assert_sequence(&self, expected: &[(CheckpointKind, u32)]) {
        assert_eq!(self.sequence(), expected); // safety: test-support assertion helper intentionally panics on mismatch.
    }

    /// Asserts the checkpoint kinds, ignoring iteration numbers.
    pub fn assert_kinds(&self, expected: &[CheckpointKind]) {
        assert_eq!(self.kinds(), expected); // safety: test-support assertion helper intentionally panics on mismatch.
    }
}

/// Builder for bespoke [`LoopExecutionState`] values.
pub struct LoopExecutionStateBuilder {
    state: LoopExecutionState,
}

impl LoopExecutionStateBuilder {
    /// Creates a state builder for a default test run context.
    pub fn new() -> Self {
        let context = test_run_context("agent-loop-state-builder");
        Self::for_context(&context)
    }

    /// Creates a state builder for the provided run context.
    pub fn for_context(context: &LoopRunContext) -> Self {
        Self {
            state: LoopExecutionState::initial_for_run(context),
        }
    }

    /// Sets the loop iteration.
    pub fn iteration(mut self, iteration: u32) -> Self {
        self.state.iteration = iteration;
        self
    }

    /// Pushes one call signature into the recent-call ring.
    pub fn push_call_signature(mut self, signature: CapabilityCallSignature) -> Self {
        self.state.recent_call_signatures.push(signature);
        self
    }

    /// Pushes one failure kind into the recent-failure ring.
    pub fn push_failure_kind(mut self, kind: LoopFailureKind) -> Self {
        self.state.recent_failure_kinds.push(kind);
        self
    }

    /// Sets the recovery attempt counter.
    pub fn recovery_attempts(mut self, attempts: u32) -> Self {
        self.state.recovery_state = RecoveryStrategyState::with_attempts_for(
            RecoveryAttemptClass::ModelTransient,
            attempts,
        );
        self
    }

    /// Returns the built state.
    pub fn build(self) -> LoopExecutionState {
        self.state
    }
}

impl Default for LoopExecutionStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopRunInfoPort for MockAgentLoopDriverHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopContextPort for MockAgentLoopDriverHost {
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
impl ironclaw_turns::run_profile::LoopPromptPort for MockAgentLoopDriverHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.record_call(MockHostCall::BuildPromptBundle);
        if let Some(kind) = *lock_or_panic(&self.fail_prompt_with) {
            return Err(AgentLoopHostError::new(kind, "scripted prompt failure"));
        }
        Ok(LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::for_run(&self.run_context, "bundle")
                .expect("test bundle ref should be valid"), // safety: test fixture construction uses a static-valid bundle token.
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: loop_message_ref("msg:user"),
            }],
            surface_version: Some(surface_version()),
            instruction_fingerprint: None,
            identity_message_count: 0,
            instruction_snippet_count: 0,
        })
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopInputPort for MockAgentLoopDriverHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.record_call(MockHostCall::PollInputs);
        let inputs = lock_or_panic(&self.script)
            .pending_inputs
            .pop_front()
            .unwrap_or_default();
        let mut input_acks = Vec::with_capacity(inputs.len());
        for (index, _) in inputs.iter().enumerate() {
            let sequence = index + 1;
            let cursor_token = LoopInputCursorToken::new(format!("input-cursor:script-{sequence}"))
                .map_err(|reason| {
                    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, reason)
                })?;
            let token = LoopInputAckToken::new(format!("input-ack:script-{sequence}")).map_err(
                |reason| AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, reason),
            )?;
            input_acks.push(LoopInputAck {
                cursor: LoopInputCursor::from_host_token(&self.run_context, cursor_token),
                token,
            });
        }
        let next_cursor = input_acks
            .last()
            .map(|ack| ack.cursor.clone())
            .unwrap_or(after);
        Ok(LoopInputBatch {
            inputs,
            input_acks,
            next_cursor,
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.record_call(MockHostCall::AckInputs);
        lock_or_panic(&self.acked_tokens).extend(tokens);
        Ok(())
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopModelPort for MockAgentLoopDriverHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.record_call(MockHostCall::StreamModel);
        if let Some(kind) = *lock_or_panic(&self.fail_model_with) {
            return Err(AgentLoopHostError::new(kind, "scripted model failure"));
        }
        match lock_or_panic(&self.script).model_responses.pop_front() {
            Some(response) => scripted_model_response(response),
            None => Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "model script exhausted",
            )),
        }
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCapabilityPort for MockAgentLoopDriverHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.record_call(MockHostCall::VisibleCapabilities);
        Ok(VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: self.visible_capabilities.clone(),
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.record_call(MockHostCall::InvokeCapability {
            capability_id: request.capability_id,
        });
        lock_or_panic(&self.script)
            .single_call_retry_outcomes
            .pop_front()
            .map(scripted_capability_outcome)
            .unwrap_or_else(|| {
                Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "single-call retry script exhausted",
                ))
            })
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.record_call(MockHostCall::InvokeCapabilityBatch {
            call_count: request.invocations.len(),
            stop_on_first_suspension: request.stop_on_first_suspension,
        });
        let outcomes = lock_or_panic(&self.script)
            .capability_outcomes
            .pop_front()
            .unwrap_or_default()
            .into_iter()
            .map(scripted_capability_outcome)
            .collect::<Result<Vec<_>, _>>()?;
        let stopped_on_suspension = request.stop_on_first_suspension
            && outcomes.iter().any(CapabilityOutcome::is_suspension);
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopTranscriptPort for MockAgentLoopDriverHost {
    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.record_call(MockHostCall::FinalizeAssistantMessage);
        Ok(loop_message_ref("msg:assistant"))
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopCheckpointPort for MockAgentLoopDriverHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        let kind = checkpoint_kind_from_host(request.kind);
        self.record_call(MockHostCall::SaveCheckpoint(kind));
        let iteration = lock_or_panic(&self.staged_iterations)
            .pop_front()
            .unwrap_or_default();
        self.checkpoints.record(kind, iteration);
        Ok(TurnCheckpointId::new())
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        let kind = checkpoint_kind_from_host(request.kind);
        self.record_call(MockHostCall::StageCheckpointPayload(kind));
        let iteration = serde_json::from_slice::<LoopExecutionState>(&request.payload)
            .map(|state| state.iteration)
            .unwrap_or_default();
        lock_or_panic(&self.staged_iterations).push_back(iteration);
        let ordinal = self.checkpoints.sequence().len();
        LoopCheckpointStateRef::for_run(&self.run_context, format!("state-{ordinal}"))
            .map_err(|error| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error))
    }
}

#[async_trait]
impl ironclaw_turns::run_profile::LoopProgressPort for MockAgentLoopDriverHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

impl LoopCancellationPort for MockAgentLoopDriverHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        lock_or_panic(&self.cancellation).clone()
    }
}

/// Builds a valid run context for tests.
pub fn test_run_context(label: &str) -> LoopRunContext {
    let suffix = safe_ref_suffix(label);
    let scope = TurnScope::new(
        TenantId::new(format!("tenant-{suffix}"))
            .unwrap_or_else(|error| panic!("test tenant id should be valid: {error}")),
        None,
        None,
        ThreadId::new(format!("thread-{suffix}"))
            .unwrap_or_else(|error| panic!("test thread id should be valid: {error}")),
    );
    let descriptor = AgentLoopDriverDescriptor {
        id: LoopDriverId::new(format!("driver_{suffix}"))
            .unwrap_or_else(|error| panic!("test driver id should be valid: {error}")),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(
            CheckpointSchemaId::new(format!("checkpoint_{suffix}"))
                .unwrap_or_else(|error| panic!("test checkpoint schema should be valid: {error}")),
        ),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let resolved_run_profile = ResolvedRunProfile {
        run_class_id: RunClassId::new(format!("class_{suffix}"))
            .unwrap_or_else(|error| panic!("test run class should be valid: {error}")),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor
            .checkpoint_schema_id
            .clone()
            .unwrap_or_else(|| panic!("test descriptor should carry checkpoint schema")),
        checkpoint_schema_version: descriptor
            .checkpoint_schema_version
            .unwrap_or_else(|| panic!("test descriptor should carry checkpoint version")),
        model_profile_id: ModelProfileId::new(format!("model_{suffix}"))
            .unwrap_or_else(|error| panic!("test model id should be valid: {error}")),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new(format!(
            "capabilities_{suffix}"
        ))
        .unwrap_or_else(|error| panic!("test capability profile should be valid: {error}")),
        context_profile_id: ContextProfileId::new(format!("context_{suffix}"))
            .unwrap_or_else(|error| panic!("test context id should be valid: {error}")),
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
            tier: ResourceBudgetTier::new(format!("tier_{suffix}"))
                .unwrap_or_else(|error| panic!("test budget tier should be valid: {error}")),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive")
            .unwrap_or_else(|error| panic!("test scheduling class should be valid: {error}")),
        concurrency_class: ConcurrencyClass::new("thread_serial")
            .unwrap_or_else(|error| panic!("test concurrency class should be valid: {error}")),
        resolution_fingerprint: RunProfileFingerprint::new(format!("fingerprint-{suffix}"))
            .unwrap_or_else(|error| panic!("test fingerprint should be valid: {error}")),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    };
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
}

/// Builds a capability descriptor for the mock visible surface.
pub fn capability_descriptor(
    id: CapabilityId,
    concurrency_hint: ConcurrencyHint,
) -> CapabilityDescriptorView {
    CapabilityDescriptorView {
        capability_id: id,
        provider: None,
        runtime: RuntimeKind::FirstParty,
        safe_name: "demo".to_string(),
        safe_description: "demo capability".to_string(),
        concurrency_hint,
    }
}

/// Builds a capability id, panicking if the test value is invalid.
pub fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value)
        .unwrap_or_else(|error| panic!("test capability id should be valid: {error}"))
}

/// Builds the default mock surface version.
pub fn surface_version() -> CapabilitySurfaceVersion {
    CapabilitySurfaceVersion::new("surface:v1")
        .unwrap_or_else(|error| panic!("test surface version should be valid: {error}"))
}

fn scripted_model_response(
    response: ScriptedModelResponse,
) -> Result<LoopModelResponse, AgentLoopHostError> {
    let output = match response {
        ScriptedModelResponse::Reply { text } => ParentLoopOutput::AssistantReply(AssistantReply {
            content: text.clone(),
        }),
        ScriptedModelResponse::Calls(calls) => ParentLoopOutput::CapabilityCalls(
            calls.into_iter().map(scripted_capability_call).collect(),
        ),
        ScriptedModelResponse::Error { kind } => {
            return Err(AgentLoopHostError::new(kind, "scripted model failure"));
        }
    };
    Ok(LoopModelResponse {
        chunks: vec![ModelStreamChunk {
            safe_text_delta: String::new(),
        }],
        output,
        effective_model_profile_id: ModelProfileId::new("model")
            .unwrap_or_else(|error| panic!("test model id should be valid: {error}")),
    })
}

fn scripted_capability_call(call: ScriptedCapabilityCall) -> CapabilityCallCandidate {
    CapabilityCallCandidate {
        surface_version: surface_version(),
        capability_id: capability_id(&call.name),
        input_ref: CapabilityInputRef::new(call.input_ref)
            .unwrap_or_else(|error| panic!("test capability input ref should be valid: {error}")),
    }
}

fn scripted_capability_outcome(
    outcome: ScriptedCapabilityOutcome,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    match outcome {
        ScriptedCapabilityOutcome::Completed {
            result_ref,
            terminate_hint,
        } => Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(result_ref)
                .unwrap_or_else(|error| panic!("test result ref should be valid: {error}")),
            safe_summary: "completed".to_string(),
            terminate_hint,
        })),
        ScriptedCapabilityOutcome::ApprovalRequired { gate_ref } => {
            Ok(CapabilityOutcome::ApprovalRequired {
                gate_ref: loop_gate_ref(&gate_ref),
                safe_summary: "approval required".to_string(),
            })
        }
        ScriptedCapabilityOutcome::AuthRequired { gate_ref } => {
            Ok(CapabilityOutcome::AuthRequired {
                gate_ref: loop_gate_ref(&gate_ref),
                safe_summary: "auth required".to_string(),
            })
        }
        ScriptedCapabilityOutcome::ResourceBlocked { gate_ref } => {
            Ok(CapabilityOutcome::ResourceBlocked {
                gate_ref: loop_gate_ref(&gate_ref),
                safe_summary: "resource blocked".to_string(),
            })
        }
        ScriptedCapabilityOutcome::Failed { error_kind } => {
            Ok(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind,
                safe_summary: "failed".to_string(),
            }))
        }
    }
}

fn checkpoint_kind_from_host(kind: LoopCheckpointKind) -> CheckpointKind {
    match kind {
        LoopCheckpointKind::BeforeModel => CheckpointKind::BeforeModel,
        LoopCheckpointKind::BeforeSideEffect => CheckpointKind::BeforeSideEffect,
        LoopCheckpointKind::BeforeBlock => CheckpointKind::BeforeBlock,
        LoopCheckpointKind::Final => CheckpointKind::Final,
    }
}

fn scripted_failure_kind(kind: &str) -> CapabilityFailureKind {
    match kind {
        "authorization" => CapabilityFailureKind::Authorization,
        "backend" => CapabilityFailureKind::Backend,
        "cancelled" => CapabilityFailureKind::Cancelled,
        "dispatcher" => CapabilityFailureKind::Dispatcher,
        "input_invalid" | "invalid_input" => CapabilityFailureKind::InvalidInput,
        "missing_runtime" => CapabilityFailureKind::MissingRuntime,
        "network" => CapabilityFailureKind::Network,
        "output_too_large" => CapabilityFailureKind::OutputTooLarge,
        "policy_denied" => CapabilityFailureKind::PolicyDenied,
        "process" => CapabilityFailureKind::Process,
        "resource" => CapabilityFailureKind::Resource,
        "transient" => CapabilityFailureKind::Transient,
        "unavailable" => CapabilityFailureKind::Unavailable,
        "internal" => CapabilityFailureKind::Internal,
        "permanent" => CapabilityFailureKind::Permanent,
        other => match CapabilityFailureKind::unknown(other.to_string()) {
            Ok(kind) => kind,
            Err(_) => CapabilityFailureKind::Permanent,
        },
    }
}

fn loop_message_ref(value: &str) -> LoopMessageRef {
    LoopMessageRef::new(value)
        .unwrap_or_else(|error| panic!("test message ref should be valid: {error}"))
}

fn loop_gate_ref(value: &str) -> LoopGateRef {
    LoopGateRef::new(value).unwrap_or_else(|error| panic!("test gate ref should be valid: {error}"))
}

fn safe_ref_suffix(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn lock_or_panic<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|error| panic!("test fixture mutex poisoned: {error}"))
}

fn clone_mutex_vec<T: Clone>(mutex: &Mutex<Vec<T>>) -> Vec<T> {
    lock_or_panic(mutex).clone()
}
