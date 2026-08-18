use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    resolution::{Resolution, ResolutionBatch},
    turn::{
        AcceptedMessageRef, EventCursor, LoopExitId, LoopMessageRef, TurnCheckpointId,
        TurnLeaseToken, TurnRunnerId, TurnStatus,
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
use ironclaw_turn_runner::{
    driver_registry::{DriverKind, DriverRegistry, DriverRequirements},
    loop_exit_applier::InMemoryLoopExitEvidencePort,
    turn_run_executor::RebornTurnRunExecutor,
    turn_runner::{HostFactory, HostFactoryError},
    turn_scheduler::TurnRunExecutor,
};
use ironclaw_turns::{
    TurnRunState, loop_exit::LoopExitApplier, runner::ClaimedTurnRun,
    test_support::in_memory_agent_turn_process_system,
};

/// The executor test must reach the caller-level error mapping without making
/// any host-port calls. Keeping those ports fail-closed makes the test prove
/// that the runner, rather than a driver helper, carries finalizer usage into
/// the returned failure metadata.
struct FinalizationFailureHost {
    context: LoopRunContext,
    supplemental_usage: LoopModelUsage,
}

fn unsupported(name: &str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("{name} is not used by this test driver"),
    )
}

impl LoopRunInfoPort for FinalizationFailureHost {
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
impl LoopContextPort for FinalizationFailureHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        Err(unsupported("load_loop_context"))
    }
}

#[async_trait]
impl LoopPromptPort for FinalizationFailureHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        Err(unsupported("build_prompt_bundle"))
    }
}

#[async_trait]
impl LoopInputPort for FinalizationFailureHost {
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
impl LoopModelPort for FinalizationFailureHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        Err(unsupported("stream_model"))
    }
}

#[async_trait]
impl ironclaw_loop_contracts::LoopCapabilityPort for FinalizationFailureHost {
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
impl LoopTranscriptPort for FinalizationFailureHost {
    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported("finalize_assistant_message"))
    }
}

#[async_trait]
impl LoopCheckpointPort for FinalizationFailureHost {
    async fn checkpoint(
        &self,
        _request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        Err(unsupported("checkpoint"))
    }
}

#[async_trait]
impl LoopProgressPort for FinalizationFailureHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        Err(unsupported("emit_loop_progress"))
    }
}

#[async_trait]
impl LoopCompactionPort for FinalizationFailureHost {
    async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        Err(LoopCompactionError::UnsupportedMode)
    }
}

#[async_trait]
impl LoopCancellationPort for FinalizationFailureHost {
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

struct FinalizationFailureHostFactory {
    context: LoopRunContext,
    supplemental_usage: LoopModelUsage,
}

#[async_trait]
impl HostFactory for FinalizationFailureHostFactory {
    async fn create_host(
        &self,
        _claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        Ok(Box::new(FinalizationFailureHost {
            context: self.context.clone(),
            supplemental_usage: self.supplemental_usage,
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

fn completed_exit() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::ResultOnly,
        reply_message_refs: Vec::new(),
        result_refs: Vec::new(),
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
    ClaimedTurnRun {
        state: TurnRunState {
            scope: context.scope.clone(),
            actor: None,
            turn_id: context.turn_id,
            run_id: context.run_id,
            status: TurnStatus::Queued,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid ref"),
            output_contract: ironclaw_host_api::output::OutputContract::AssistantMessage,
            resolved_run_profile_id: context.resolved_run_profile.profile_id.clone(),
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
        Arc::new(FinalizationFailureHostFactory {
            context: context.clone(),
            supplemental_usage,
        }),
        None,
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
