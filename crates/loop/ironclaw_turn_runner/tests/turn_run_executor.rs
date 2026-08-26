use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    ids::UserId,
    resolution::{Resolution, ResolutionBatch},
    turn::{
        AcceptedMessageRef, EventCursor, LoopExitId, LoopMessageRef, LoopResultRef, RunProfileId,
        TurnActor, TurnCheckpointId, TurnLeaseToken, TurnRunnerId, TurnStatus,
    },
};
use ironclaw_loop_contracts::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, AgentLoopHostError,
    AgentLoopHostErrorKind, FinalizeAssistantMessage, LoopCancellationPort, LoopCancellationSignal,
    LoopCheckpointPort, LoopCheckpointRequest, LoopCompactionError, LoopCompactionOutcome,
    LoopCompactionPort, LoopCompactionRequest, LoopCompleted, LoopCompletionKind,
    LoopContextBundle, LoopContextPort, LoopContextRequest, LoopExit, LoopInputAckToken,
    LoopInputBatch, LoopInputCursor, LoopInputPort, LoopModelPort, LoopModelRequest,
    LoopModelResponse, LoopModelUsage, LoopProgressEvent, LoopProgressPort, LoopPromptBundle,
    LoopPromptBundleRequest, LoopPromptPort, LoopRequest, LoopRequestBatch, LoopRunContext,
    LoopRunInfoPort, LoopTranscriptPort, VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_loop_host::AwaitEdgeSettler;
use ironclaw_processes::ProcessTransitionPort;
use ironclaw_turn_runner::{
    driver_registry::{DriverKind, DriverRegistry, DriverRequirements},
    loop_exit_applier::InMemoryLoopExitEvidencePort,
    turn_run_executor::RebornTurnRunExecutor,
    turn_runner::{HostFactory, HostFactoryError},
    turn_scheduler::TurnRunExecutor,
};
use ironclaw_turns::{
    TurnError, TurnRunState, loop_exit::LoopExitApplier, runner::ClaimedTurnRun,
    test_support::in_memory_agent_turn_process_system,
};

/// No-op `AwaitEdgeSettler` test double (per the task brief: executor tests
/// that don't exercise the sweep wire a no-op fake rather than the real
/// resolver, which needs a live process journal/thread service this test
/// doesn't stand up).
struct NoOpAwaitEdgeSettler;

#[async_trait]
impl AwaitEdgeSettler for NoOpAwaitEdgeSettler {
    async fn on_child_terminal(
        &self,
        _event: &ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<ironclaw_loop_host::ResolveOutcome, AgentLoopHostError> {
        Ok(ironclaw_loop_host::ResolveOutcome::NotApplicable)
    }

    async fn sweep_thread_on_run_start(
        &self,
        _scope: &ironclaw_turns::TurnScope,
        _human_initiated: bool,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    fn bind_coordinator(
        &self,
        _coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_turn_tree_store(
        &self,
        _store: Arc<dyn ironclaw_turns::AgentTurnSpawnTreeRuntimePort>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_result_writer(
        &self,
        _result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_input_enqueue(
        &self,
        _port: Arc<dyn ironclaw_loop_host::HostInputEnqueuePort>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn as_turn_committed_event_observer(
        self: Arc<Self>,
    ) -> Arc<dyn ironclaw_turns::TurnCommittedEventObserver> {
        self
    }
}

#[async_trait::async_trait]
impl ironclaw_turns::TurnCommittedEventObserver for NoOpAwaitEdgeSettler {
    fn observes_state(&self, _state: &ironclaw_turns::TurnRunState) -> bool {
        false
    }

    fn observes_event(&self, _event: &ironclaw_turns::TurnLifecycleEvent) -> bool {
        false
    }

    async fn observe_committed_state(
        &self,
        _state: ironclaw_turns::TurnRunState,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    async fn observe_committed_event(
        &self,
        _event: ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }
}

/// The executor test must reach the caller-level error mapping without making
/// any host-port calls. Keeping those ports fail-closed makes the test prove
/// that the runner, rather than a driver helper, carries finalizer usage into
/// the returned failure metadata.
struct ExecutorTestHost {
    context: LoopRunContext,
    supplemental_usage: LoopModelUsage,
    /// `true` reproduces the finalizer-failure path the usage test needs;
    /// `false` lets the exit apply cleanly so post-terminal work (the
    /// `after_turn` dispatch) is reachable.
    finalize_fails: bool,
}

fn unsupported(name: &str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("{name} is not used by this test driver"),
    )
}

impl LoopRunInfoPort for ExecutorTestHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.context
    }

    fn finalize_terminal_output<'a>(
        &'a self,
        _exit: &'a LoopExit,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), AgentLoopHostError>> + Send + 'a>,
    > {
        Box::pin(async {
            if !self.finalize_fails {
                return Ok(());
            }
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "finalizer failed",
            ))
        })
    }

    fn supplemental_model_usage(&self) -> Option<LoopModelUsage> {
        Some(self.supplemental_usage)
    }
}

#[async_trait]
impl LoopContextPort for ExecutorTestHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        Err(unsupported("load_loop_context"))
    }
}

#[async_trait]
impl LoopPromptPort for ExecutorTestHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        Err(unsupported("build_prompt_bundle"))
    }
}

#[async_trait]
impl LoopInputPort for ExecutorTestHost {
    async fn poll_inputs(
        &self,
        _after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        Err(unsupported("poll_inputs"))
    }

    async fn ack_inputs(&self, _tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        Err(unsupported("ack_inputs"))
    }
}

#[async_trait]
impl LoopModelPort for ExecutorTestHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        Err(unsupported("stream_model"))
    }
}

#[async_trait]
impl ironclaw_loop_contracts::LoopCapabilityPort for ExecutorTestHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Err(unsupported("visible_capabilities"))
    }

    async fn invoke_capability(
        &self,
        _request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        Err(unsupported("invoke_capability"))
    }

    async fn invoke_capability_batch(
        &self,
        _request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        Err(unsupported("invoke_capability_batch"))
    }
}

#[async_trait]
impl LoopTranscriptPort for ExecutorTestHost {
    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported("finalize_assistant_message"))
    }
}

#[async_trait]
impl LoopCheckpointPort for ExecutorTestHost {
    async fn checkpoint(
        &self,
        _request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        Err(unsupported("checkpoint"))
    }
}

#[async_trait]
impl LoopProgressPort for ExecutorTestHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        Err(unsupported("emit_loop_progress"))
    }
}

#[async_trait]
impl LoopCompactionPort for ExecutorTestHost {
    async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        Err(LoopCompactionError::UnsupportedMode)
    }
}

#[async_trait]
impl LoopCancellationPort for ExecutorTestHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        LoopCancellationSignal {
            reason_kind: ironclaw_loop_contracts::LoopCancelReasonKind::UserRequested,
            requested_at: Utc::now(),
        }
    }
}

struct ExecutorTestHostFactory {
    context: LoopRunContext,
    supplemental_usage: LoopModelUsage,
    finalize_fails: bool,
}

#[async_trait]
impl HostFactory for ExecutorTestHostFactory {
    async fn create_host(
        &self,
        _claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        Ok(Box::new(ExecutorTestHost {
            context: self.context.clone(),
            supplemental_usage: self.supplemental_usage,
            finalize_fails: self.finalize_fails,
        }))
    }
}

struct ImmediateCompletedDriver {
    descriptor: AgentLoopDriverDescriptor,
}

#[async_trait]
impl AgentLoopDriver for ImmediateCompletedDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Ok(completed_exit())
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Ok(completed_exit())
    }
}

/// Returns a fixed [`LoopExit`] so a test can drive the executor to a chosen
/// terminal state (completed / failed / cancelled) at the `after_turn` seam.
struct ScriptedExitDriver {
    descriptor: AgentLoopDriverDescriptor,
    exit: LoopExit,
}

#[async_trait]
impl AgentLoopDriver for ScriptedExitDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Ok(self.exit.clone())
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Ok(self.exit.clone())
    }
}

fn failed_exit() -> LoopExit {
    LoopExit::failed(
        ironclaw_loop_contracts::LoopFailureKind::ModelError,
        LoopExitId::new("exit:executor-after-turn-failed").expect("valid test exit id"),
    )
}

fn cancelled_exit() -> LoopExit {
    LoopExit::cancelled_for_observed_interrupt(
        LoopExitId::new("exit:executor-after-turn-cancelled").expect("valid test exit id"),
    )
}

fn completed_exit() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::ResultOnly,
        reply_message_refs: Vec::new(),
        // A `ResultOnly` completion with no result ref is an invalid exit, and
        // the applier terminalizes it as a FAILURE — which would quietly turn
        // any assertion about a completed run into an assertion about a failed
        // one.
        result_refs: vec![LoopResultRef::new("result:executor-test").expect("valid result ref")],
        final_checkpoint_id: None,
        model_usage: Some(LoopModelUsage {
            input_tokens: 120,
            output_tokens: 48,
            cache_read_input_tokens: 11,
            cache_creation_input_tokens: 3,
        }),
        exit_id: LoopExitId::new("exit:finalizer-failure").expect("valid test exit id"),
    })
}

fn claimed_run(context: &LoopRunContext) -> ClaimedTurnRun {
    claimed_run_as(
        context,
        None,
        context.resolved_run_profile.profile_id.clone(),
    )
}

/// `claimed_run` with the two axes the `after_turn` derivation judges: whether
/// the run has a bound actor, and which profile it resolved.
fn claimed_run_as(
    context: &LoopRunContext,
    actor: Option<TurnActor>,
    profile_id: RunProfileId,
) -> ClaimedTurnRun {
    claimed_run_full(context, actor, profile_id, None)
}

/// `claimed_run` with the provenance axis the run-start sweep derivation
/// judges: whether the run was activated as a background subagent.
fn claimed_run_with_provenance(
    context: &LoopRunContext,
    subagent_activation_provenance: Option<ironclaw_turns::ActivationProvenance>,
) -> ClaimedTurnRun {
    claimed_run_full(
        context,
        None,
        context.resolved_run_profile.profile_id.clone(),
        subagent_activation_provenance,
    )
}

/// Records every `sweep_thread_on_run_start` call
/// (`scope`/`human_initiated`) and replays a scripted `Result` — proves the
/// executor derives `human_initiated` from the claimed run's
/// `subagent_activation_provenance` and that a sweep failure does not stop
/// the driver from running.
#[derive(Default)]
struct RecordingAwaitEdgeSettler {
    calls: std::sync::Mutex<Vec<(ironclaw_turns::TurnScope, bool)>>,
    sweep_result: std::sync::Mutex<Option<Result<(), AgentLoopHostError>>>,
}

impl RecordingAwaitEdgeSettler {
    fn with_sweep_result(self, result: Result<(), AgentLoopHostError>) -> Self {
        *self
            .sweep_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        self
    }

    fn calls(&self) -> Vec<(ironclaw_turns::TurnScope, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl AwaitEdgeSettler for RecordingAwaitEdgeSettler {
    async fn on_child_terminal(
        &self,
        _event: &ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<ironclaw_loop_host::ResolveOutcome, AgentLoopHostError> {
        Ok(ironclaw_loop_host::ResolveOutcome::NotApplicable)
    }

    async fn sweep_thread_on_run_start(
        &self,
        scope: &ironclaw_turns::TurnScope,
        human_initiated: bool,
    ) -> Result<(), AgentLoopHostError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((scope.clone(), human_initiated));
        self.sweep_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or(Ok(()))
    }

    fn bind_coordinator(
        &self,
        _coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_turn_tree_store(
        &self,
        _store: Arc<dyn ironclaw_turns::AgentTurnSpawnTreeRuntimePort>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_result_writer(
        &self,
        _result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn bind_input_enqueue(
        &self,
        _port: Arc<dyn ironclaw_loop_host::HostInputEnqueuePort>,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    fn as_turn_committed_event_observer(
        self: Arc<Self>,
    ) -> Arc<dyn ironclaw_turns::TurnCommittedEventObserver> {
        self
    }
}

#[async_trait::async_trait]
impl ironclaw_turns::TurnCommittedEventObserver for RecordingAwaitEdgeSettler {
    fn observes_state(&self, _state: &ironclaw_turns::TurnRunState) -> bool {
        false
    }

    fn observes_event(&self, _event: &ironclaw_turns::TurnLifecycleEvent) -> bool {
        false
    }

    async fn observe_committed_state(
        &self,
        _state: ironclaw_turns::TurnRunState,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }

    async fn observe_committed_event(
        &self,
        _event: ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<(), ironclaw_turns::TurnError> {
        Ok(())
    }
}

fn claimed_run_full(
    context: &LoopRunContext,
    actor: Option<TurnActor>,
    profile_id: RunProfileId,
    subagent_activation_provenance: Option<ironclaw_turns::ActivationProvenance>,
) -> ClaimedTurnRun {
    ClaimedTurnRun {
        subagent_activation_provenance,
        state: TurnRunState {
            scope: context.scope.clone(),
            actor,
            turn_id: context.turn_id,
            run_id: context.run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid ref"),
            output_contract: ironclaw_host_api::output::OutputContract::AssistantMessage,
            resolved_run_profile_id: profile_id,
            resolved_run_profile_version: context.resolved_run_profile.profile_version,
            allow_steering: false,
            resolved_model_route: None,
            model_usage: Some(LoopModelUsage {
                input_tokens: 100,
                output_tokens: 40,
                cache_read_input_tokens: 10,
                cache_creation_input_tokens: 2,
            }),
            execution_outcome: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(0),
            product_context: None,
            resume_disposition: None,
        },
        resolved_run_profile: context.resolved_run_profile.clone(),
        subagent_depth: 0,
        spawn_tree_descendant_cap: None,
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
    }
}

/// Builds an executor identical to the finalizer-failure test's, except
/// wired with the given `AwaitEdgeSettler`. Shared by the sweep-derivation
/// tests below — none of them care how the run itself ends (finalization
/// always fails in this fixture), only what the executor asked the settler
/// for before the driver ran.
fn build_executor_for_sweep_test(
    context: &LoopRunContext,
    settler: Arc<dyn AwaitEdgeSettler>,
) -> (
    RebornTurnRunExecutor,
    Arc<dyn ProcessTransitionPort<Error = TurnError>>,
) {
    let descriptor = context.resolved_run_profile.loop_driver.clone();
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ImmediateCompletedDriver { descriptor }),
            DriverRequirements::all_optional(),
            DriverKind::Reference,
        )
        .expect("register test driver");
    let process_system = in_memory_agent_turn_process_system();
    let transitions = process_system.transitions();
    let applier = Arc::new(LoopExitApplier::new(
        transitions.clone(),
        Arc::new(InMemoryLoopExitEvidencePort::new()),
    ));
    let executor = RebornTurnRunExecutor::new(
        applier,
        Arc::new(registry),
        Arc::new(ExecutorTestHostFactory {
            context: context.clone(),
            supplemental_usage: LoopModelUsage {
                input_tokens: 1,
                output_tokens: 1,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            finalize_fails: true,
        }),
        None,
        settler,
    );
    (executor, transitions)
}

#[tokio::test]
async fn execute_claimed_run_sweeps_with_human_initiated_true_for_default_provenance() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-sweep-human");
    let settler = Arc::new(RecordingAwaitEdgeSettler::default());
    let (executor, transitions) =
        build_executor_for_sweep_test(&context, Arc::clone(&settler) as Arc<dyn AwaitEdgeSettler>);

    let claimed = claimed_run_with_provenance(&context, None);
    let claimed_scope = claimed.state.scope.clone();
    let _ = executor.execute_claimed_run(claimed, transitions).await;

    let calls = settler.calls();
    assert_eq!(
        calls.len(),
        1,
        "the executor must sweep exactly once per run start"
    );
    assert_eq!(
        calls[0].0, claimed_scope,
        "the sweep must run over the claimed run's own scope"
    );
    assert!(
        calls[0].1,
        "absent activation provenance is a human/permitted start"
    );
}

#[tokio::test]
async fn execute_claimed_run_sweeps_with_human_initiated_false_for_system_provenance() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-sweep-system");
    let settler = Arc::new(RecordingAwaitEdgeSettler::default());
    let (executor, transitions) =
        build_executor_for_sweep_test(&context, Arc::clone(&settler) as Arc<dyn AwaitEdgeSettler>);

    let claimed =
        claimed_run_with_provenance(&context, Some(ironclaw_turns::ActivationProvenance::System));
    let _ = executor.execute_claimed_run(claimed, transitions).await;

    let calls = settler.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        !calls[0].1,
        "a System-provenance (background wake) start is not human/permitted"
    );
}

#[tokio::test]
async fn execute_claimed_run_sweeps_with_human_initiated_false_for_parent_agent_provenance() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-sweep-parent");
    let settler = Arc::new(RecordingAwaitEdgeSettler::default());
    let (executor, transitions) =
        build_executor_for_sweep_test(&context, Arc::clone(&settler) as Arc<dyn AwaitEdgeSettler>);

    let claimed = claimed_run_with_provenance(
        &context,
        Some(ironclaw_turns::ActivationProvenance::ParentAgent),
    );
    let _ = executor.execute_claimed_run(claimed, transitions).await;

    assert!(!settler.calls()[0].1);
}

/// A sweep failure must not stop the run start: the driver still runs and
/// the run reaches its ordinary (here, still-failing) finalization outcome —
/// proven by the SAME finalizer-failure category the baseline test asserts,
/// which is only reachable once `invoke_driver` actually ran.
#[tokio::test]
async fn execute_claimed_run_runs_the_driver_even_when_the_sweep_fails() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-sweep-failure");
    let settler = Arc::new(RecordingAwaitEdgeSettler::default().with_sweep_result(Err(
        AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "sweep unavailable"),
    )));
    let (executor, transitions) =
        build_executor_for_sweep_test(&context, Arc::clone(&settler) as Arc<dyn AwaitEdgeSettler>);

    let error = executor
        .execute_claimed_run(claimed_run(&context), transitions)
        .await
        .expect_err("the driver still runs and finalization still fails past the sweep");

    assert_eq!(error.failure_category(), "structured_finalization_failed");
    assert_eq!(
        settler.calls().len(),
        1,
        "the sweep was still attempted once"
    );
}

#[tokio::test]
async fn execute_claimed_run_preserves_finalizer_usage_on_host_failure() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-finalization");
    let descriptor = context.resolved_run_profile.loop_driver.clone();
    let supplemental_usage = LoopModelUsage {
        input_tokens: 13,
        output_tokens: 5,
        cache_read_input_tokens: 2,
        cache_creation_input_tokens: 1,
    };
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ImmediateCompletedDriver { descriptor }),
            DriverRequirements::all_optional(),
            DriverKind::Reference,
        )
        .expect("register test driver");

    let process_system = in_memory_agent_turn_process_system();
    let transitions = process_system.transitions();
    let applier = Arc::new(LoopExitApplier::new(
        transitions.clone(),
        Arc::new(InMemoryLoopExitEvidencePort::new()),
    ));
    let executor = RebornTurnRunExecutor::new(
        applier,
        Arc::new(registry),
        Arc::new(ExecutorTestHostFactory {
            context: context.clone(),
            supplemental_usage,
            finalize_fails: true,
        }),
        None,
        Arc::new(NoOpAwaitEdgeSettler) as Arc<dyn AwaitEdgeSettler>,
    );

    let error = executor
        .execute_claimed_run(claimed_run(&context), transitions)
        .await
        .expect_err("host finalization failure should reach the executor caller");

    assert_eq!(error.failure_category(), "structured_finalization_failed");
    assert_eq!(
        error
            .failure_metadata()
            .and_then(|metadata| metadata.model_usage()),
        Some(LoopModelUsage {
            input_tokens: 133,
            output_tokens: 53,
            cache_read_input_tokens: 13,
            cache_creation_input_tokens: 4,
        })
    );
}

/// Evidence port that verifies whatever a completion claims. The strict
/// in-memory port distrusts by default and its permissive mutators are
/// `#[cfg(test)]` on the crate itself, so an integration test that needs a run
/// to actually reach `Completed` supplies its own.
struct VerifiedEvidencePort;

#[async_trait]
impl ironclaw_turns::loop_exit::LoopExitEvidencePort for VerifiedEvidencePort {
    async fn verify_completion_refs(
        &self,
        _request: ironclaw_turns::loop_exit::CompletionEvidenceRequest<'_>,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        Ok(true)
    }

    async fn verify_final_checkpoint(
        &self,
        _request: ironclaw_turns::loop_exit::FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        Ok(true)
    }

    async fn verify_blocked_evidence(
        &self,
        _request: ironclaw_turns::loop_exit::BlockedEvidenceRequest<'_>,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        Ok(true)
    }

    async fn verify_failure_evidence(
        &self,
        _request: ironclaw_turns::loop_exit::FailureEvidenceRequest<'_>,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        Ok(true)
    }

    async fn is_cancellation_observed(
        &self,
        _scope: &ironclaw_turns::TurnScope,
        _turn_id: ironclaw_turns::TurnId,
        _run_id: ironclaw_turns::TurnRunId,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        Ok(true)
    }

    async fn latest_checkpoint_kind(
        &self,
        _scope: &ironclaw_turns::TurnScope,
        _turn_id: ironclaw_turns::TurnId,
        _run_id: ironclaw_turns::TurnRunId,
    ) -> Result<Option<ironclaw_loop_contracts::LoopCheckpointKind>, ironclaw_turns::TurnError>
    {
        Ok(None)
    }
}

/// Permissive transition port for the `after_turn` dispatch tests: echoes the
/// claimed run back as a terminal snapshot so the exit applies cleanly and the
/// executor reaches its post-terminal work. The journal-backed system the other
/// test uses would need a genuinely submitted and claimed process; what is
/// under test here is the executor's dispatch at the hook seam, not the
/// journal.
struct CompletingTransitionPort {
    claimed: ClaimedTurnRun,
}

#[async_trait]
impl ironclaw_processes::ProcessTransitionPort for CompletingTransitionPort {
    type Error = ironclaw_turns::TurnError;

    async fn claim_next_processes(
        &self,
        _request: ironclaw_processes::ClaimProcessesRequest,
    ) -> Result<Vec<ironclaw_processes::ClaimedProcess>, Self::Error> {
        Ok(Vec::new())
    }

    async fn heartbeat_process(
        &self,
        _request: ironclaw_processes::ProcessLeaseRequest,
    ) -> Result<ironclaw_processes::ProcessJournalCursor, Self::Error> {
        Ok(ironclaw_processes::ProcessJournalCursor(0))
    }

    async fn recover_expired_process_leases(
        &self,
        _request: ironclaw_processes::RecoverExpiredProcessLeasesRequest,
    ) -> Result<ironclaw_processes::RecoverExpiredProcessLeasesResponse, Self::Error> {
        Ok(ironclaw_processes::RecoverExpiredProcessLeasesResponse { recovered: vec![] })
    }

    async fn suspend_process(
        &self,
        _request: ironclaw_processes::SuspendProcessRequest,
    ) -> Result<ironclaw_processes::JournaledProcessSnapshot, Self::Error> {
        Ok(self.snapshot(ironclaw_processes::ProcessLifecycleStatus::Suspended))
    }

    async fn complete_process(
        &self,
        _request: ironclaw_processes::ProcessStateTransitionRequest,
    ) -> Result<ironclaw_processes::JournaledProcessSnapshot, Self::Error> {
        Ok(self.snapshot(ironclaw_processes::ProcessLifecycleStatus::Completed))
    }

    async fn cancel_process(
        &self,
        _request: ironclaw_processes::ProcessStateTransitionRequest,
    ) -> Result<ironclaw_processes::JournaledProcessSnapshot, Self::Error> {
        Ok(self.snapshot(ironclaw_processes::ProcessLifecycleStatus::Cancelled))
    }

    async fn fail_process(
        &self,
        _request: ironclaw_processes::FailProcessRequest,
    ) -> Result<ironclaw_processes::JournaledProcessSnapshot, Self::Error> {
        Ok(self.snapshot(ironclaw_processes::ProcessLifecycleStatus::Failed))
    }

    async fn relinquish_process(
        &self,
        _request: ironclaw_processes::ProcessLeaseRequest,
    ) -> Result<ironclaw_processes::JournaledProcessSnapshot, Self::Error> {
        Ok(self.snapshot(ironclaw_processes::ProcessLifecycleStatus::Queued))
    }
}

impl CompletingTransitionPort {
    fn snapshot(
        &self,
        status: ironclaw_processes::ProcessLifecycleStatus,
    ) -> ironclaw_processes::JournaledProcessSnapshot {
        let mut snapshot = ironclaw_processes::ClaimedProcess::from(&self.claimed).state;
        snapshot.status = status;
        snapshot
    }
}

/// Recording `after_turn` hook: keeps every context it is handed so a test can
/// assert on WHICH runs reached the point, not merely how many did.
#[derive(Default)]
struct RecordingAfterTurnHook {
    contexts: std::sync::Mutex<Vec<ironclaw_hooks::points::AfterTurnHookContext>>,
}

#[async_trait]
impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for RecordingAfterTurnHook {
    async fn on_turn(&self, ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
        self.contexts
            .lock()
            .expect("hook contexts lock")
            .push(ctx.clone());
    }
}

impl RecordingAfterTurnHook {
    fn observed(&self) -> Vec<ironclaw_hooks::points::AfterTurnHookContext> {
        self.contexts.lock().expect("hook contexts lock").clone()
    }
}

/// Build the per-run dispatcher FACTORY carrying `hook` at the `after_turn`
/// point, exactly as a composition does: one long-lived hook, one fresh
/// dispatcher per run.
fn after_turn_dispatchers(
    hook: Arc<RecordingAfterTurnHook>,
) -> ironclaw_turn_runner::loop_driver_host::HookDispatcherFactory {
    struct Forwarding(Arc<RecordingAfterTurnHook>);

    #[async_trait]
    impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for Forwarding {
        async fn on_turn(&self, ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
            self.0.on_turn(ctx).await;
        }
    }

    Arc::new(move || {
        ironclaw_hooks::dispatch::HookDispatcherBuilder::new(
            ironclaw_hooks::registry::HookRegistry::new(),
        )
        .install_builtin_after_turn(
            ironclaw_hooks::identity::HookId::for_builtin(
                "ironclaw_turn_runner::tests::recording_after_turn_hook",
                ironclaw_hooks::identity::HookVersion::ONE,
            ),
            ironclaw_hooks::ordering::HookPhase::Telemetry,
            Box::new(Forwarding(Arc::clone(&hook))),
        )
        .expect("the recording hook installs")
        .build_arc()
    })
}

/// Assemble the real executor over a permissive transition port, with the
/// per-run `after_turn` dispatcher factory attached.
fn after_turn_executor(
    context: &LoopRunContext,
    claimed: &ClaimedTurnRun,
    dispatchers: ironclaw_turn_runner::loop_driver_host::HookDispatcherFactory,
    exit: LoopExit,
) -> (
    RebornTurnRunExecutor,
    Arc<dyn ironclaw_processes::ProcessTransitionPort<Error = ironclaw_turns::TurnError>>,
) {
    let descriptor = context.resolved_run_profile.loop_driver.clone();
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ScriptedExitDriver { descriptor, exit }),
            DriverRequirements::all_optional(),
            DriverKind::Reference,
        )
        .expect("register test driver");

    let transitions: Arc<
        dyn ironclaw_processes::ProcessTransitionPort<Error = ironclaw_turns::TurnError>,
    > = Arc::new(CompletingTransitionPort {
        claimed: claimed.clone(),
    });
    let applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&transitions),
        Arc::new(VerifiedEvidencePort),
    ));
    let executor = RebornTurnRunExecutor::new(
        applier,
        Arc::new(registry),
        Arc::new(ExecutorTestHostFactory {
            context: context.clone(),
            supplemental_usage: LoopModelUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            finalize_fails: false,
        }),
        None,
        Arc::new(NoOpAwaitEdgeSettler) as Arc<dyn AwaitEdgeSettler>,
    )
    .with_after_turn_hook_dispatcher_factory(dispatchers);
    (executor, transitions)
}

/// Run one claimed run to completion through the real executor with the
/// dispatcher factory attached, and report what the hook saw.
async fn observed_after_turn_contexts(
    context: &LoopRunContext,
    claimed: ClaimedTurnRun,
) -> Vec<ironclaw_hooks::points::AfterTurnHookContext> {
    observed_after_turn_contexts_for_exit(context, claimed, completed_exit()).await
}

/// [`observed_after_turn_contexts`] for a run whose driver returns `exit`, so a
/// test can pin what the hook sees for each terminal disposition.
async fn observed_after_turn_contexts_for_exit(
    context: &LoopRunContext,
    claimed: ClaimedTurnRun,
    exit: LoopExit,
) -> Vec<ironclaw_hooks::points::AfterTurnHookContext> {
    let hook = Arc::new(RecordingAfterTurnHook::default());
    let (executor, transitions) = after_turn_executor(
        context,
        &claimed,
        after_turn_dispatchers(Arc::clone(&hook)),
        exit,
    );

    executor
        .execute_claimed_run(claimed, transitions)
        .await
        .expect("the run applies its exit");
    hook.observed()
}

/// The seam itself: attaching a dispatcher must actually get a finished
/// conversation run delivered to the hook. Testing the context derivation alone
/// would leave the executor free to never call it — the failure mode where
/// every hook is installed, registered, and silently never fires.
#[tokio::test]
async fn a_completed_conversation_run_reaches_the_after_turn_hook() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-after-turn");
    let user_id = UserId::new("user-after-turn").expect("valid user id");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(user_id.clone())),
        RunProfileId::interactive_default(),
    );

    let observed = observed_after_turn_contexts(&context, claimed).await;

    assert_eq!(observed.len(), 1, "the point fires exactly once per run");
    assert_eq!(
        observed[0].user_id, user_id,
        "follow-on work is attributed to the run's actor"
    );
    assert!(
        observed[0].completed,
        "a run that reached Completed reports success to hooks"
    );
}

/// The guard that keeps hook-started background work from feeding itself: an
/// unbound run must never reach the point, even with the dispatcher attached
/// and an actor present.
#[tokio::test]
async fn an_unbound_run_never_reaches_the_after_turn_hook() {
    let context =
        ironclaw_agent_loop::test_support::test_run_context("executor-after-turn-unbound");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(
            UserId::new("user-after-turn").expect("valid user id"),
        )),
        RunProfileId::unbound_default(),
    );

    let observed = observed_after_turn_contexts(&context, claimed).await;

    assert!(
        observed.is_empty(),
        "a background run must not fire the point: each pass would schedule its successor"
    );
}

/// Hook poison is RUN-scoped. A hook that panics gets its binding poisoned for
/// the rest of that run's dispatch — and the next run must try it again.
///
/// This is the property a process-lifetime dispatcher silently breaks: one
/// panic in one turn would bar the hook until the process restarted, so
/// curation (or any lifecycle hook) would go dead with nothing reporting it.
/// The executor therefore mints a fresh dispatcher per run, and this test
/// drives TWO runs through ONE long-lived executor to pin that.
#[tokio::test]
async fn a_panicking_hook_is_retried_on_the_next_run() {
    /// Panics the first time it is invoked, records every time after.
    #[derive(Default)]
    struct PanicOnceHook {
        invocations: std::sync::Mutex<usize>,
        observed: std::sync::Mutex<usize>,
    }

    #[async_trait]
    impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for PanicOnceHook {
        async fn on_turn(&self, _ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
            let first = {
                let mut invocations = self.invocations.lock().expect("invocations lock");
                *invocations += 1;
                *invocations == 1
            };
            assert!(!first, "intentional first-dispatch panic");
            *self.observed.lock().expect("observed lock") += 1;
        }
    }

    let context = ironclaw_agent_loop::test_support::test_run_context("executor-after-turn-poison");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(
            UserId::new("user-after-turn").expect("valid user id"),
        )),
        RunProfileId::interactive_default(),
    );

    let hook = Arc::new(PanicOnceHook::default());
    let dispatchers: ironclaw_turn_runner::loop_driver_host::HookDispatcherFactory = {
        let hook = Arc::clone(&hook);
        Arc::new(move || {
            struct Forwarding(Arc<PanicOnceHook>);
            #[async_trait]
            impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for Forwarding {
                async fn on_turn(&self, ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
                    self.0.on_turn(ctx).await;
                }
            }
            ironclaw_hooks::dispatch::HookDispatcherBuilder::new(
                ironclaw_hooks::registry::HookRegistry::new(),
            )
            .install_builtin_after_turn(
                ironclaw_hooks::identity::HookId::for_builtin(
                    "ironclaw_turn_runner::tests::panic_once_after_turn_hook",
                    ironclaw_hooks::identity::HookVersion::ONE,
                ),
                ironclaw_hooks::ordering::HookPhase::Telemetry,
                Box::new(Forwarding(Arc::clone(&hook))),
            )
            .expect("the panicking hook installs")
            .build_arc()
        })
    };

    let (executor, transitions) =
        after_turn_executor(&context, &claimed, dispatchers, completed_exit());

    for _ in 0..2 {
        executor
            .execute_claimed_run(claimed.clone(), Arc::clone(&transitions))
            .await
            .expect("the run applies its exit");
    }

    assert_eq!(
        *hook.invocations.lock().expect("invocations lock"),
        2,
        "the second run must reach the hook again: poison cannot outlive the run it happened in"
    );
    assert_eq!(
        *hook.observed.lock().expect("observed lock"),
        1,
        "the first dispatch panicked; the second one ran to completion"
    );
}

/// A conversation turn that ended in FAILURE is still a finished turn: the
/// point fires, and it reports the failure honestly so a lifecycle hook can
/// tell a successful turn from a broken one. Without this, a hook that only
/// ever saw completed runs would silently treat every failure as "no turn
/// happened".
#[tokio::test]
async fn a_failed_conversation_run_reaches_the_after_turn_hook_as_incomplete() {
    let context = ironclaw_agent_loop::test_support::test_run_context("executor-after-turn-failed");
    let user_id = UserId::new("user-after-turn").expect("valid user id");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(user_id.clone())),
        RunProfileId::interactive_default(),
    );

    let observed = observed_after_turn_contexts_for_exit(&context, claimed, failed_exit()).await;

    assert_eq!(observed.len(), 1, "the point fires exactly once per run");
    assert_eq!(observed[0].user_id, user_id);
    assert!(
        !observed[0].completed,
        "a run that reached Failed must not be reported to hooks as a success"
    );
}

/// Same contract for a CANCELLED turn: the turn is over, so the point fires,
/// and `completed` stays false.
#[tokio::test]
async fn a_cancelled_conversation_run_reaches_the_after_turn_hook_as_incomplete() {
    let context =
        ironclaw_agent_loop::test_support::test_run_context("executor-after-turn-cancelled");
    let user_id = UserId::new("user-after-turn").expect("valid user id");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(user_id.clone())),
        RunProfileId::interactive_default(),
    );

    let observed = observed_after_turn_contexts_for_exit(&context, claimed, cancelled_exit()).await;

    assert_eq!(observed.len(), 1, "the point fires exactly once per run");
    assert_eq!(observed[0].user_id, user_id);
    assert!(
        !observed[0].completed,
        "a run that reached Cancelled must not be reported to hooks as a success"
    );
}

/// One wedged hook must not take the dispatch down with it. The dispatcher
/// bounds each hook individually (`AFTER_TURN_HOOK_TIMEOUT`) and the executor's
/// outer backstop is deliberately much larger, so the inner bound is the one
/// that fires: the hung hook is classified as a Timeout failure, the hooks
/// ordered after it still run, and the already-terminal run is unaffected.
///
/// The failure mode this pins is the timeout RACE: if the outer backstop were
/// sized at or below the per-hook bound, the whole dispatch would be cancelled
/// mid-classification and the survivor would never be reached.
///
/// The per-hook budget is shortened to milliseconds for this test
/// (`with_after_turn_timeout`) so the wedged hook is classified promptly
/// instead of costing the suite the production 5-second budget. The executor's
/// outer backstop is untouched, which is exactly the asymmetry under test.
#[tokio::test]
async fn a_hung_hook_is_timed_out_and_the_next_hook_still_runs() {
    /// Shortened per-hook budget: long enough that a hook doing real work
    /// would finish, short enough that the wedged one is classified promptly.
    /// Orders of magnitude below the executor's 30s outer backstop, which is
    /// the asymmetry that lets the dispatcher classify rather than be
    /// cancelled mid-flight.
    const HUNG_HOOK_BUDGET: std::time::Duration = std::time::Duration::from_millis(50);

    /// Never returns. Ordered ahead of the survivor by phase.
    struct HungHook;

    #[async_trait]
    impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for HungHook {
        async fn on_turn(&self, _ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
            std::future::pending::<()>().await;
        }
    }

    /// Counts its invocations so the test can prove it was reached.
    #[derive(Default)]
    struct SurvivorHook {
        invocations: std::sync::Mutex<usize>,
    }

    #[async_trait]
    impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for SurvivorHook {
        async fn on_turn(&self, _ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
            *self.invocations.lock().expect("invocations lock") += 1;
        }
    }

    let context = ironclaw_agent_loop::test_support::test_run_context("executor-after-turn-hang");
    let claimed = claimed_run_as(
        &context,
        Some(TurnActor::new(
            UserId::new("user-after-turn").expect("valid user id"),
        )),
        RunProfileId::interactive_default(),
    );

    let survivor = Arc::new(SurvivorHook::default());
    // Hook failures are only observable from outside the dispatcher through the
    // milestone sink, so the test reads the Timeout classification from there.
    let milestones = Arc::new(ironclaw_loop_contracts::InMemoryHookMilestoneSink::default());
    let survivor_id = ironclaw_hooks::identity::HookId::for_builtin(
        "ironclaw_turn_runner::tests::survivor_after_turn_hook",
        ironclaw_hooks::identity::HookVersion::ONE,
    );
    let hung_id = ironclaw_hooks::identity::HookId::for_builtin(
        "ironclaw_turn_runner::tests::hung_after_turn_hook",
        ironclaw_hooks::identity::HookVersion::ONE,
    );

    let dispatchers: ironclaw_turn_runner::loop_driver_host::HookDispatcherFactory = {
        let survivor = Arc::clone(&survivor);
        let milestones = Arc::clone(&milestones);
        Arc::new(move || {
            struct Forwarding(Arc<SurvivorHook>);
            #[async_trait]
            impl ironclaw_hooks::sink::PrivilegedAfterTurnHook for Forwarding {
                async fn on_turn(&self, ctx: &ironclaw_hooks::points::AfterTurnHookContext) {
                    self.0.on_turn(ctx).await;
                }
            }
            ironclaw_hooks::dispatch::HookDispatcherBuilder::new(
                ironclaw_hooks::registry::HookRegistry::new(),
            )
            .with_milestone_sink(Arc::clone(&milestones) as Arc<_>)
            .with_after_turn_timeout(HUNG_HOOK_BUDGET)
            // Phase orders the two: Policy runs before Telemetry, so the hung
            // hook is guaranteed to be the one the survivor follows.
            .install_builtin_after_turn(
                hung_id,
                ironclaw_hooks::ordering::HookPhase::Policy,
                Box::new(HungHook),
            )
            .expect("the hung hook installs")
            .install_builtin_after_turn(
                survivor_id,
                ironclaw_hooks::ordering::HookPhase::Telemetry,
                Box::new(Forwarding(Arc::clone(&survivor))),
            )
            .expect("the survivor hook installs")
            .build_arc()
        })
    };

    let (executor, transitions) =
        after_turn_executor(&context, &claimed, dispatchers, completed_exit());

    executor
        .execute_claimed_run(claimed, transitions)
        .await
        .expect("a wedged lifecycle hook must not fail the already-terminal run");

    assert_eq!(
        *survivor.invocations.lock().expect("invocations lock"),
        1,
        "the hook ordered after the timed-out one must still run"
    );

    let timed_out: Vec<_> = milestones
        .kinds()
        .into_iter()
        .filter_map(|kind| {
            if let ironclaw_loop_contracts::LoopHostMilestoneKind::HookFailed {
                hook_id,
                category,
                ..
            } = kind
            {
                Some((hook_id, category))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        timed_out.len(),
        1,
        "exactly the hung hook is recorded as a failure, got {timed_out:?}"
    );
    assert_eq!(
        timed_out[0].0,
        ironclaw_hooks::telemetry::hook_id_string(hung_id),
        "the recorded failure is the hung hook, not the survivor"
    );
    assert_eq!(
        timed_out[0].1, "timeout",
        "the hung hook is classified as a timeout, not swallowed"
    );
}
