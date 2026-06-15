use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use chrono::Utc;
use ironclaw_agent_loop::{
    state::CheckpointKind,
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedModelResponse,
        test_run_context,
    },
};
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, CorrelationId, ResourceEstimate, RuntimeKind,
};
use ironclaw_reborn::app_loop_family::build_loop_family_registry;
use ironclaw_reborn::planned_driver::PlannedDriver;
use ironclaw_reborn::planned_driver_factory::{
    PLANNED_DEFAULT_PROFILE_ID, PLANNED_DRIVER_DEFAULT_ID, default_planned_run_profile_resolver,
};
use ironclaw_turns::{
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopCancelledReasonKind, LoopExit,
    LoopGateRef, LoopMessageRef, RedactedCheckpointPayload, TurnCheckpointId,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverError, AgentLoopHostError, AgentLoopHostErrorKind,
        AppendCapabilityResultRef, AssistantReply, BeginAssistantDraft, CapabilityApprovalResume,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityFailure,
        CapabilityFailureKind, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        CapabilityResumeToken, FinalizeAssistantMessage, LoadCheckpointPayloadRequest,
        LoadedCheckpointPayload, LoopCancelReasonKind, LoopCancellationPort,
        LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort,
        LoopCheckpointRequest, LoopCheckpointStateRef, LoopCompactionError, LoopCompactionOutcome,
        LoopCompactionPort, LoopCompactionRequest, LoopContextBundle, LoopContextPort,
        LoopContextRequest, LoopInput, LoopInputAckToken, LoopInputBatch, LoopInputCursor,
        LoopInputPort, LoopModelMessage, LoopModelPort, LoopModelRequest, LoopModelResponse,
        LoopProgressEvent, LoopProgressPort, LoopPromptBundle, LoopPromptBundleRef,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopRunInfoPort, LoopSafeSummary,
        LoopTranscriptPort, ModelStreamChunk, ParentLoopOutput, RunProfileResolver,
        StageCheckpointPayloadRequest, UpdateAssistantDraft, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};

fn run_request(driver: &PlannedDriver, host: &impl LoopRunInfoPort) -> AgentLoopDriverRunRequest {
    let mut profile = host.run_context().resolved_run_profile.clone();
    let descriptor = driver.descriptor();
    profile.loop_driver = descriptor.clone();
    profile.checkpoint_schema_id = descriptor
        .checkpoint_schema_id
        .clone()
        .expect("planned driver descriptor should carry checkpoint schema");
    profile.checkpoint_schema_version = descriptor
        .checkpoint_schema_version
        .expect("planned driver descriptor should carry checkpoint version");
    AgentLoopDriverRunRequest {
        turn_id: host.run_context().turn_id,
        run_id: host.run_context().run_id,
        resolved_run_profile: profile,
    }
}

fn resume_request(
    context: &LoopRunContext,
    checkpoint_id: TurnCheckpointId,
) -> AgentLoopDriverResumeRequest {
    AgentLoopDriverResumeRequest {
        turn_id: context.turn_id,
        run_id: context.run_id,
        checkpoint_id,
        resolved_run_profile: context.resolved_run_profile.clone(),
    }
}

fn run_context_for_driver(driver: &PlannedDriver) -> LoopRunContext {
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();
    let mut context = host.run_context().clone();
    let descriptor = driver.descriptor();
    context.resolved_run_profile.loop_driver = descriptor.clone();
    context.resolved_run_profile.checkpoint_schema_id = descriptor
        .checkpoint_schema_id
        .clone()
        .expect("planned driver descriptor should carry checkpoint schema");
    context.resolved_run_profile.checkpoint_schema_version = descriptor
        .checkpoint_schema_version
        .expect("planned driver descriptor should carry checkpoint version");
    context.loop_driver_id = descriptor.id;
    context.loop_driver_version = descriptor.version;
    context.checkpoint_schema_id = context.resolved_run_profile.checkpoint_schema_id.clone();
    context.checkpoint_schema_version = context.resolved_run_profile.checkpoint_schema_version;
    context
}

#[tokio::test]
async fn default_planned_driver_smoke() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(driver.descriptor().id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
}

#[tokio::test]
async fn planned_driver_cancellation_short_circuits_through_adapter() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let signal = LoopCancellationSignal {
        reason_kind: LoopCancelReasonKind::UserRequested,
        requested_at: Utc::now(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("should not be requested"))
        .cancellation_signal(signal)
        .build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver cancellation should be a loop exit");

    match exit {
        LoopExit::Cancelled(cancelled) => {
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
            assert!(cancelled.checkpoint_id.is_some());
        }
        other => panic!("expected cancelled exit, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 0);
    checkpoints.assert_kinds(&[CheckpointKind::Final]);
}

#[tokio::test]
async fn planned_driver_live_default_smoke() {
    let resolver = default_planned_run_profile_resolver().expect("resolver should build");
    let resolved = resolver
        .resolve_run_profile(ironclaw_turns::RunProfileResolutionRequest::interactive_default())
        .await
        .expect("implicit profile should resolve");
    assert_eq!(resolved.profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);
    assert_eq!(resolved.loop_driver.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);

    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let base_context = test_run_context("planned-live-default");
    let context = LoopRunContext::new(
        base_context.scope,
        base_context.turn_id,
        base_context.run_id,
        resolved.clone(),
    );
    let (host, _) = MockAgentLoopDriverHost::builder()
        .run_context(context)
        .script(ScenarioScript::reply_only("hi"))
        .build();
    let request = run_request(&driver, &host);
    assert_eq!(request.resolved_run_profile, resolved);

    let exit = driver
        .run(request, &host)
        .await
        .expect("planned live default should run");

    assert!(matches!(exit, LoopExit::Completed(_)));
}

#[tokio::test]
async fn planned_driver_executor_error_maps_to_unavailable() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .fail_prompt_with(AgentLoopHostErrorKind::Unavailable)
        .build();

    let error = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect_err("model unavailability should map to driver error");

    assert_eq!(
        error,
        AgentLoopDriverError::Unavailable {
            reason: "Prompt: unavailable".to_string()
        }
    );
    let debug = format!("{error:?}");
    assert!(!debug.contains("sk-fake"));
    assert!(!debug.contains("/host/path"));
}

#[tokio::test]
async fn planned_driver_rejects_mismatched_profile_assignment() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();
    let mut request = run_request(&driver, &host);
    request.resolved_run_profile.loop_driver.version = ironclaw_turns::RunProfileVersion::new(99);

    let error = driver
        .run(request, &host)
        .await
        .expect_err("mismatched descriptor should be rejected");

    assert!(matches!(error, AgentLoopDriverError::InvalidRequest { .. }));
}

#[tokio::test]
async fn planned_driver_consumes_steering_message_before_model_call() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Reply {
            text: "hi".to_string(),
        }]),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::from([
            vec![LoopInput::Steering {
                message_ref: LoopMessageRef::new("msg:steering").unwrap(),
            }],
            Vec::new(),
        ]),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let calls = host.call_log();
    let poll_inputs = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::PollInputs))
        .expect("inputs should be polled");
    let first_prompt = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::BuildPromptBundle))
        .expect("prompt should be built");
    let before_model_checkpoint = calls
        .iter()
        .position(|call| {
            matches!(
                call,
                MockHostCall::SaveCheckpoint(CheckpointKind::BeforeModel)
            )
        })
        .expect("advanced cursor should be checkpointed before model call");
    let ack_inputs = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::AckInputs))
        .expect("drained input should be acknowledged");
    let model_call = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::StreamModel))
        .expect("model should be called");
    assert_eq!(poll_inputs, 0);
    assert!(
        first_prompt > poll_inputs,
        "steering input must be consumed before the prompt/model path"
    );
    assert!(
        before_model_checkpoint < ack_inputs,
        "physical input ack must wait until the advanced cursor is durable"
    );
    assert!(
        ack_inputs < model_call,
        "input ack should happen before model IO"
    );
}

#[tokio::test]
async fn planned_driver_followup_restarts_after_natural_stop() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Reply {
                text: "first".to_string(),
            },
            ScriptedModelResponse::Reply {
                text: "second".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::from([
            Vec::new(),
            vec![LoopInput::FollowUp {
                message_ref: LoopMessageRef::new("msg:followup").unwrap(),
            }],
            Vec::new(),
            Vec::new(),
        ]),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_call_count(), 2);
    assert!(
        host.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::AckInputs))
            .count()
            >= 1,
        "followup consumption should ack the advanced input cursor"
    );
}

#[tokio::test]
async fn planned_driver_resume_rejects_mismatched_ids_before_checkpoint_load() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let context = run_context_for_driver(&driver);
    let host = ForbiddenResumeHost::new(context.clone());
    let mut request = resume_request(&context, TurnCheckpointId::new());
    let other_context = ironclaw_agent_loop::test_support::test_run_context("foreign-run");
    request.turn_id = other_context.turn_id;
    request.run_id = other_context.run_id;

    let error = driver
        .resume(request, &host)
        .await
        .expect_err("mismatched request ids should be rejected");

    assert_eq!(
        error,
        AgentLoopDriverError::InvalidRequest {
            reason: "driver request does not match loop host run context".to_string()
        }
    );
    host.assert_no_checkpoint_load_or_host_side_effects();
}

#[tokio::test]
async fn planned_driver_resume_rejects_mismatched_profile_before_checkpoint_load() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let context = run_context_for_driver(&driver);
    let host = ForbiddenResumeHost::new(context.clone());
    let mut request = resume_request(&context, TurnCheckpointId::new());
    let other_context = ironclaw_agent_loop::test_support::test_run_context("foreign-profile");
    request.resolved_run_profile.context_profile_id =
        other_context.resolved_run_profile.context_profile_id;

    let error = driver
        .resume(request, &host)
        .await
        .expect_err("mismatched request profile should be rejected");

    assert_eq!(
        error,
        AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile does not match loop host run context".to_string()
        }
    );
    host.assert_no_checkpoint_load_or_host_side_effects();
}

struct ForbiddenResumeHost {
    context: LoopRunContext,
    checkpoint_load_calls: AtomicUsize,
    host_side_effect_calls: AtomicUsize,
}

impl ForbiddenResumeHost {
    fn new(context: LoopRunContext) -> Self {
        Self {
            context,
            checkpoint_load_calls: AtomicUsize::new(0),
            host_side_effect_calls: AtomicUsize::new(0),
        }
    }

    fn forbidden_call(&self, method: &'static str) -> AgentLoopHostError {
        self.host_side_effect_calls.fetch_add(1, Ordering::SeqCst);
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("{method} should not be called for invalid resume request context"),
        )
    }

    fn assert_no_checkpoint_load_or_host_side_effects(&self) {
        assert_eq!(
            self.checkpoint_load_calls.load(Ordering::SeqCst),
            0,
            "invalid resume context must fail before checkpoint payload load"
        );
        assert_eq!(
            self.host_side_effect_calls.load(Ordering::SeqCst),
            0,
            "invalid resume context must fail before host side effects"
        );
    }
}

impl LoopRunInfoPort for ForbiddenResumeHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.context
    }
}

#[async_trait::async_trait]
impl LoopContextPort for ForbiddenResumeHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        Err(self.forbidden_call("load_loop_context"))
    }
}

#[async_trait::async_trait]
impl LoopPromptPort for ForbiddenResumeHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        Err(self.forbidden_call("build_prompt_bundle"))
    }
}

#[async_trait::async_trait]
impl LoopInputPort for ForbiddenResumeHost {
    async fn poll_inputs(
        &self,
        _after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        Err(self.forbidden_call("poll_inputs"))
    }

    async fn ack_inputs(&self, _tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        Err(self.forbidden_call("ack_inputs"))
    }
}

#[async_trait::async_trait]
impl LoopModelPort for ForbiddenResumeHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        Err(self.forbidden_call("stream_model"))
    }
}

#[async_trait::async_trait]
impl LoopCapabilityPort for ForbiddenResumeHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Err(self.forbidden_call("visible_capabilities"))
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Err(self.forbidden_call("invoke_capability"))
    }

    async fn invoke_capability_batch(
        &self,
        _request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        Err(self.forbidden_call("invoke_capability_batch"))
    }
}

#[async_trait::async_trait]
impl LoopTranscriptPort for ForbiddenResumeHost {
    async fn begin_assistant_draft(
        &self,
        _request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(self.forbidden_call("begin_assistant_draft"))
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        Err(self.forbidden_call("update_assistant_draft"))
    }

    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(self.forbidden_call("finalize_assistant_message"))
    }

    async fn append_capability_result_ref(
        &self,
        _request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(self.forbidden_call("append_capability_result_ref"))
    }
}

#[async_trait::async_trait]
impl LoopCheckpointPort for ForbiddenResumeHost {
    async fn checkpoint(
        &self,
        _request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        Err(self.forbidden_call("checkpoint"))
    }

    async fn stage_checkpoint_payload(
        &self,
        _request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        Err(self.forbidden_call("stage_checkpoint_payload"))
    }

    async fn load_checkpoint_payload(
        &self,
        _request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        self.checkpoint_load_calls.fetch_add(1, Ordering::SeqCst);
        Err(self.forbidden_call("load_checkpoint_payload"))
    }
}

#[async_trait::async_trait]
impl LoopProgressPort for ForbiddenResumeHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        Err(self.forbidden_call("emit_loop_progress"))
    }
}

#[async_trait::async_trait]
impl LoopCompactionPort for ForbiddenResumeHost {
    async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        let error = self.forbidden_call("compact_loop_context");
        Err(LoopCompactionError::PersistenceFailed {
            safe_summary: LoopSafeSummary::new(error.safe_summary)
                .expect("forbidden call summary should be loop-safe"),
        })
    }
}

#[async_trait::async_trait]
impl LoopCancellationPort for ForbiddenResumeHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        std::future::pending().await
    }
}

/// Regression test for PR #4899: a resume-origin Backend failure must surface
/// as a model-visible tool error rather than triggering a scope_mismatch /
/// terminal `HostUnavailable` run death.
///
/// Before the fix, `handle_capability_error` returned `RecoveryOutcome::Retry`
/// for a `Backend` failure and called `capability_invocation_from_candidate(call, None)`,
/// dropping the resume context and causing scope_mismatch.  After the fix, the
/// `is_resume_origin` guard intercepts the retry and surfaces the error as a
/// `ToolErrorResult` so the model can re-approve.
///
/// Flow:
///   1. `run()`: model returns a call to "demo.echo" → batch returns
///      `ApprovalRequired { approval_resume: Some(...) }` → driver writes
///      `BeforeBlock` checkpoint, exits `LoopExit::Blocked`.
///   2. `resume()`: driver loads checkpoint (state has `pending_approval_resume`)
///      → executor re-dispatches batch → batch returns
///      `Failed { error_kind: Backend }` → fix intercepts retry, surfaces as
///      tool error → model returns reply → `LoopExit::Completed`.
///   3. Assert `invoke_capability` (single-call retry) was NOT called — the fix
///      must NOT reach the re-dispatch path.
#[tokio::test]
async fn planned_driver_resume_approval_backend_failure_surfaces_as_tool_error_not_retry() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let context = run_context_for_driver(&driver);
    let host = CapableResumeHost::new(context.clone());

    // Run phase: model calls "demo.echo", batch returns ApprovalRequired with
    // approval_resume populated.  Driver suspends with LoopExit::Blocked.
    let run_req = run_request(&driver, &host);
    let exit = driver
        .run(run_req, &host)
        .await
        .expect("run should produce a loop exit");

    let blocked = match exit {
        LoopExit::Blocked(blocked) => blocked,
        other => panic!("expected blocked exit from approval gate, got {other:?}"),
    };
    assert_eq!(blocked.kind, ironclaw_turns::LoopBlockedKind::Approval);
    let checkpoint_id = blocked.checkpoint_id;

    assert_eq!(
        host.single_retry_call_count(),
        0,
        "invoke_capability must not be called during the run phase"
    );

    // Resume phase: driver loads checkpoint, executor sees pending_approval_resume,
    // re-dispatches batch which returns Backend failure.  The fix surfaces it as a
    // tool error, model returns a reply, loop completes.
    let exit = driver
        .resume(resume_request(&context, checkpoint_id), &host)
        .await
        .expect("resume should produce a loop exit");

    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "after Backend surface-and-continue, model reply should complete the loop; got {exit:?}"
    );
    assert_eq!(
        host.single_retry_call_count(),
        0,
        "invoke_capability must NOT be called after resume-origin Backend failure (PR #4899 fix)"
    );

    // --- Strengthened assertions (Codex P2 review) ---
    // These ensure the test cannot false-green when the resume batch is skipped.

    // (a) The resume-phase batch must have been invoked: total batch calls == 2
    //     (one in run phase for ApprovalRequired, one in resume phase for Backend failure).
    //     If checkpoint/resume regressed and the executor skipped invoke_capability_batch
    //     during resume, this count would be 1, failing the test.
    assert_eq!(
        host.batch_call_count(),
        2,
        "invoke_capability_batch must be called exactly twice: once in run (ApprovalRequired) \
         and once in resume (Backend failure); a count of 1 means the resume batch was skipped"
    );

    // (b) The resume batch must have carried approval_resume: Some(...) on its invocation.
    //     This confirms the checkpoint's pending_approval_resume was correctly threaded
    //     through to the re-dispatch.  If approval_resume were stripped (None), the
    //     is_resume_origin guard would not fire and the PR #4899 fix would not apply.
    assert!(
        host.resume_batch_had_approval_resume(),
        "the resume-phase batch must carry approval_resume: Some(...) so the Backend failure \
         is intercepted by the is_resume_origin guard (PR #4899 fix path)"
    );

    // (c) Both scripted batch outcomes must be consumed.
    //     A non-empty queue means the resume batch was never called and the scripted
    //     Failed(Backend) outcome was never delivered, so the test exercised nothing.
    assert!(
        host.scripted_batch_outcomes_fully_consumed(),
        "all scripted batch outcomes must be consumed: a leftover outcome means the resume \
         invoke_capability_batch call was skipped and the Backend failure path was not exercised"
    );
}

/// A host that drives the approval-resume → Backend-failure scenario.
///
/// Phase 1 (run):
///   - `stream_model` → one capability call to "demo.echo"
///   - `invoke_capability_batch` → `ApprovalRequired { approval_resume: Some(...) }`
///   - `stage_checkpoint_payload` + `checkpoint` → stores payload, returns ID
///
/// Phase 2 (resume):
///   - `load_checkpoint_payload` → returns stored payload
///   - `invoke_capability_batch` → `Failed { error_kind: Backend }`
///   - `stream_model` → assistant reply (model sees the surfaced tool error)
///
/// `invoke_capability` must never be called in either phase (tracked separately).
struct CapableResumeHost {
    context: LoopRunContext,
    /// Scripted model responses (pop_front each call).
    model_responses: Mutex<VecDeque<LoopModelResponse>>,
    /// Scripted batch outcomes (pop_front each call).
    batch_outcomes: Mutex<VecDeque<Vec<CapabilityOutcome>>>,
    /// Stores (payload_bytes, kind) staged between stage_checkpoint_payload and
    /// checkpoint().  We only need to handle one pending payload at a time.
    pending_payload: Mutex<Option<(Vec<u8>, LoopCheckpointKind)>>,
    /// Stores the committed (checkpoint_id → (bytes, kind)) mapping.
    committed_payloads: Mutex<Vec<(TurnCheckpointId, Vec<u8>, LoopCheckpointKind)>>,
    /// Tracks calls to `invoke_capability` (single-call retry path).
    single_retry_calls: AtomicUsize,
    /// Counts how many times `invoke_capability_batch` was called (both phases).
    batch_call_count: AtomicUsize,
    /// Set to true when any `invoke_capability_batch` call contained an invocation
    /// with `approval_resume: Some(...)`.  This confirms the resume-origin path was
    /// exercised and the checkpoint payload was correctly threaded through.
    resume_batch_had_approval_resume: AtomicBool,
}

impl CapableResumeHost {
    fn new(context: LoopRunContext) -> Self {
        // Phase 1 model response: one capability call.
        let phase1_model = LoopModelResponse {
            chunks: vec![ModelStreamChunk {
                safe_text_delta: String::new(),
            }],
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![capable_resume_call()]),
            effective_model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("model")
                .unwrap(),
            usage: None,
        };
        // Phase 2 model response: plain assistant reply (after the error is surfaced).
        let phase2_model = LoopModelResponse {
            chunks: vec![ModelStreamChunk {
                safe_text_delta: String::new(),
            }],
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::AssistantReply(AssistantReply {
                content: "error noted, please re-approve".to_string(),
            }),
            effective_model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("model")
                .unwrap(),
            usage: None,
        };

        // Phase 1 batch outcome: ApprovalRequired with approval_resume.
        let approval_request_id = ApprovalRequestId::new();
        let resume_token = CapabilityResumeToken::new("resume-token:demo-echo").unwrap();
        let correlation_id = CorrelationId::new();
        let input_ref = CapabilityInputRef::new("input:demo-echo").expect("valid input ref");
        let phase1_batch = vec![CapabilityOutcome::ApprovalRequired {
            gate_ref: LoopGateRef::new("gate:demo-echo-approval").unwrap(),
            safe_summary: "approval required for demo.echo".to_string(),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id,
                resume_token,
                correlation_id,
                input_ref,
                input: serde_json::json!({"message": "hello"}),
                estimate: ResourceEstimate::default(),
            }),
        }];
        // Phase 2 batch outcome: Backend failure (triggers the PR #4899 fix path).
        let phase2_batch = vec![CapabilityOutcome::Failed(CapabilityFailure {
            error_kind: CapabilityFailureKind::Backend,
            safe_summary: "backend unavailable during resume".to_string(),
            detail: None,
        })];

        Self {
            context,
            model_responses: Mutex::new(VecDeque::from([phase1_model, phase2_model])),
            batch_outcomes: Mutex::new(VecDeque::from([phase1_batch, phase2_batch])),
            pending_payload: Mutex::new(None),
            committed_payloads: Mutex::new(Vec::new()),
            single_retry_calls: AtomicUsize::new(0),
            batch_call_count: AtomicUsize::new(0),
            resume_batch_had_approval_resume: AtomicBool::new(false),
        }
    }

    fn single_retry_call_count(&self) -> usize {
        self.single_retry_calls.load(Ordering::SeqCst)
    }

    /// Total number of times `invoke_capability_batch` was called across both phases.
    fn batch_call_count(&self) -> usize {
        self.batch_call_count.load(Ordering::SeqCst)
    }

    /// Whether any batch call carried `approval_resume: Some(...)` on its invocations.
    /// True iff the resume path actually threaded the approval context through.
    fn resume_batch_had_approval_resume(&self) -> bool {
        self.resume_batch_had_approval_resume.load(Ordering::SeqCst)
    }

    /// Whether the scripted `batch_outcomes` queue was fully drained (all scripted
    /// outcomes consumed).  A leftover outcome means the resume batch was never called.
    fn scripted_batch_outcomes_fully_consumed(&self) -> bool {
        self.batch_outcomes.lock().unwrap().is_empty()
    }
}

/// Build an `AgentLoopDriverRunRequest` from a `CapableResumeHost` context,
/// mirroring the logic of the `run_request` helper but without requiring a
/// `MockAgentLoopDriverHost`.
fn capable_resume_call() -> ironclaw_turns::run_profile::CapabilityCallCandidate {
    use ironclaw_turns::run_profile::{CapabilityCallCandidate, CapabilitySurfaceVersion};
    CapabilityCallCandidate {
        surface_version: CapabilitySurfaceVersion::new("surface:v1").unwrap(),
        capability_id: CapabilityId::new("demo.echo").unwrap(),
        input_ref: CapabilityInputRef::new("input:demo-echo").unwrap(),
        effective_capability_ids: vec![CapabilityId::new("demo.echo").unwrap()],
        provider_replay: None,
    }
}

impl LoopRunInfoPort for CapableResumeHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.context
    }
}

#[async_trait::async_trait]
impl LoopContextPort for CapableResumeHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        Ok(LoopContextBundle {
            identity_messages: Vec::new(),
            messages: Vec::new(),
            compaction_message_index: Vec::new(),
            instruction_snippets: Vec::new(),
            memory_snippets: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl LoopPromptPort for CapableResumeHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        let bundle_ref = LoopPromptBundleRef::for_run(&self.context, "bundle")
            .map_err(|e| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, e))?;
        Ok(LoopPromptBundle {
            bundle_ref,
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new("msg:user").unwrap(),
            }],
            surface_version: Some(
                ironclaw_turns::run_profile::CapabilitySurfaceVersion::new("surface:v1").unwrap(),
            ),
            compaction_message_index: Vec::new(),
            instruction_fingerprint: None,
            identity_message_count: 0,
            instruction_snippet_count: 0,
        })
    }
}

#[async_trait::async_trait]
impl LoopInputPort for CapableResumeHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_inputs(&self, _tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl LoopModelPort for CapableResumeHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        let response = self
            .model_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "model script exhausted in CapableResumeHost",
                )
            })?;
        Ok(response)
    }
}

#[async_trait::async_trait]
impl LoopCapabilityPort for CapableResumeHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        use ironclaw_turns::run_profile::{
            CapabilityDescriptorView, CapabilitySurfaceVersion, ConcurrencyHint,
        };
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface:v1").unwrap(),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: "demo_echo".to_string(),
                safe_description: "echo demo capability".to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: serde_json::json!({"type": "object"}),
            }],
        })
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        // This must NOT be called after a resume-origin Backend failure.
        self.single_retry_calls.fetch_add(1, Ordering::SeqCst);
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "invoke_capability must not be called in the resume-origin Backend failure path",
        ))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.batch_call_count.fetch_add(1, Ordering::SeqCst);

        // Record whether any invocation in this batch carries approval_resume.
        // This is set on the resume-phase batch, confirming the checkpoint's
        // pending_approval_resume was threaded through to the re-dispatch.
        if request
            .invocations
            .iter()
            .any(|inv| inv.approval_resume.is_some())
        {
            self.resume_batch_had_approval_resume
                .store(true, Ordering::SeqCst);
        }

        let outcomes = self
            .batch_outcomes
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "batch script exhausted in CapableResumeHost",
                )
            })?;
        let stopped_on_suspension = request.stop_on_first_suspension
            && outcomes.iter().any(CapabilityOutcome::is_suspension);
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

#[async_trait::async_trait]
impl LoopTranscriptPort for CapableResumeHost {
    async fn begin_assistant_draft(
        &self,
        _request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Ok(LoopMessageRef::new("msg:draft").unwrap())
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Ok(LoopMessageRef::new("msg:assistant").unwrap())
    }

    async fn append_capability_result_ref(
        &self,
        _request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Ok(LoopMessageRef::new("msg:tool-result").unwrap())
    }
}

#[async_trait::async_trait]
impl LoopCheckpointPort for CapableResumeHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        let id = TurnCheckpointId::new();
        // Move the pending payload (staged by stage_checkpoint_payload) into the
        // committed map, keyed by the new checkpoint ID.
        let pending = self.pending_payload.lock().unwrap().take();
        if let Some((bytes, _kind)) = pending {
            self.committed_payloads
                .lock()
                .unwrap()
                .push((id, bytes, request.kind));
        }
        Ok(id)
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        // Store the raw bytes with the kind so checkpoint() can commit them.
        *self.pending_payload.lock().unwrap() = Some((request.payload, request.kind));
        LoopCheckpointStateRef::for_run(&self.context, "state-payload")
            .map_err(|e| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, e))
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        let committed = self.committed_payloads.lock().unwrap();
        let (_, bytes, kind) = committed
            .iter()
            .find(|(id, _, _)| *id == request.checkpoint_id)
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    format!(
                        "checkpoint {} not found in CapableResumeHost",
                        request.checkpoint_id.as_uuid()
                    ),
                )
            })?;
        let payload = RedactedCheckpointPayload::new(bytes.clone())
            .map_err(|e| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, e))?;
        Ok(LoadedCheckpointPayload {
            kind: *kind,
            schema_id: request.expected_schema_id.clone(),
            schema_version: request.expected_schema_version,
            payload,
        })
    }
}

#[async_trait::async_trait]
impl LoopProgressPort for CapableResumeHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl LoopCompactionPort for CapableResumeHost {
    async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        Err(LoopCompactionError::PersistenceFailed {
            safe_summary: LoopSafeSummary::new("compaction not supported in test host")
                .expect("valid"),
        })
    }
}

#[async_trait::async_trait]
impl LoopCancellationPort for CapableResumeHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        std::future::pending().await
    }
}
