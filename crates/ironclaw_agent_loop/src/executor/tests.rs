use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RuntimeKind, TenantId, ThreadId};
use ironclaw_turns::{
    AgentLoopDriverDescriptor, LoopGateRef, LoopMessageRef, LoopResultRef, RunProfileId,
    RunProfileVersion, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, CancellationPolicy,
        CapabilityDescriptorView, CapabilityInputRef, CapabilitySurfaceProfileId,
        CapabilitySurfaceVersion, CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass,
        ContextProfileId, LoopCancelReasonKind, LoopCancellationPort, LoopCancellationSignal,
        LoopCheckpointRequest, LoopCheckpointStateRef, LoopContextBundle, LoopContextRequest,
        LoopDriverId, LoopInputAck, LoopInputAckToken, LoopInputBatch, LoopInputCursor,
        LoopInputCursorToken, LoopInterruptKind, LoopModelMessage, LoopModelResponse,
        LoopProcessRef, LoopPromptBundle, LoopPromptBundleRef, LoopPromptBundleRequest,
        LoopRunContext, LoopRunInfoPort, ModelProfileId, ModelStreamChunk, ProcessHandleSummary,
        ProviderToolCallReplay, RedactedRunProfileProvenance, ResolvedRunProfile,
        ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
        RuntimeProfileConstraints, SchedulingClass, StageCheckpointPayloadRequest, SteeringPolicy,
    },
};

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::strategies::{
    CapabilityStrategy, InputDrainStrategy, RecoveryOutcome, RecoveryStrategy, RetryScope,
};

use super::*;

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}

mod support;
use support::*;

mod cancellation;

#[tokio::test]
async fn reply_only_completes_with_final_checkpoint() {
    let host = MockHost::new(vec![reply_response()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected completed, got {other:?}"),
    }
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
    );
    assert_eq!(
        host.progress_event_names(),
        vec![
            "iteration_started",
            "prompt_bundle_built",
            "checkpoint_written",
            "checkpoint_written",
        ]
    );
}

#[tokio::test]
async fn progress_port_failure_does_not_abort_reply_only_run() {
    let host = MockHost::new(vec![reply_response()]).with_failing_progress_port();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(
                completed.reply_message_refs,
                vec![message_ref("msg:assistant")]
            );
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected completed, got {other:?}"),
    }
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
    );
    assert!(host.progress_events().is_empty());

    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.assistant_refs,
        vec![message_ref("msg:assistant")]
    );
    assert_eq!(
        final_state.last_checkpoint,
        Some(crate::state::CheckpointMarker {
            kind: CheckpointKind::Final,
            iteration_at_checkpoint: final_state.iteration,
        })
    );
}

#[tokio::test]
async fn budget_stage_exits_at_iteration_limit() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.iteration = family.planner().budget().iteration_limit(&state);

    let step = BudgetStage
        .process(
            ctx,
            BudgetInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("budget stage");

    assert!(matches!(step, BudgetStep::Exit(LoopExit::Failed(_))));
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn input_stage_steering_drain_carries_pending_ack() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user-drained"),
        }],
        input_acks: vec![input_ack(
            &run_context,
            "input-cursor:after-user",
            "input-ack:after-user",
        )],
        next_cursor: input_cursor(&run_context, "input-cursor:after-user"),
    }]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let step = InputStage
        .process(
            ctx,
            InputInput {
                state,
                pending_input_ack: PendingInputAck::default(),
                mode: UserFacingInputDrainMode::Steering,
            },
        )
        .await
        .expect("input stage");

    match step {
        InputStep::Continue {
            state,
            mut pending_input_ack,
            drained,
        } => {
            assert!(drained);
            assert_eq!(
                state.input_cursor,
                input_cursor(&run_context, "input-cursor:after-user")
            );
            assert!(host.acked_input_tokens().is_empty());
            pending_input_ack.ack(&host).await.expect("ack inputs");
            assert_eq!(
                host.acked_input_tokens(),
                vec![LoopInputAckToken::new("input-ack:after-user").expect("valid")]
            );
        }
        InputStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn assistant_reply_stage_returns_reply_summary() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let reply = match reply_response().output {
        ParentLoopOutput::AssistantReply(reply) => reply,
        ParentLoopOutput::CapabilityCalls(_) => panic!("expected reply fixture"),
    };

    let step = AssistantReplyStage
        .process(ctx, AssistantReplyInput { state, reply })
        .await
        .expect("assistant reply stage");

    match step {
        TurnCompletedStep::Continue { state, summary } => {
            assert_eq!(state.assistant_refs, vec![message_ref("msg:assistant")]);
            assert_eq!(
                summary,
                TurnSummary::reply_only(message_ref("msg:assistant"))
            );
        }
        TurnCompletedStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn capability_stage_returns_after_batch_summary() {
    let result_ref = LoopResultRef::new("result:done").expect("valid");
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "done".to_string(),
                terminate_hint: false,
            })],
            stopped_on_suspension: false,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let calls = match calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected calls fixture"),
    };

    let step = CapabilityStage
        .process(
            ctx,
            CapabilityBatchInput {
                state,
                surface: ironclaw_turns::run_profile::LoopCapabilityPort::visible_capabilities(
                    &host,
                    VisibleCapabilityRequest,
                )
                .await
                .expect("visible surface"),
                calls,
            },
        )
        .await
        .expect("capability stage");

    match step {
        TurnCompletedStep::Continue { state, summary } => {
            assert_eq!(state.result_refs, vec![result_ref.clone()]);
            assert_eq!(
                summary,
                TurnSummary::after_capability_batch(vec![result_ref])
            );
        }
        TurnCompletedStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn stop_stage_preserves_ack_and_returns_stop_kind() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.stop_state.last_batch_total = 1;
    state.stop_state.terminate_hints_in_last_batch = 1;
    let mut pending_input_ack = PendingInputAck::default();
    pending_input_ack
        .replace(vec![
            LoopInputAckToken::new("input-ack:pending").expect("valid"),
        ])
        .expect("store pending ack");

    let step = StopStage
        .process(
            ctx,
            StopInput {
                state,
                summary: TurnSummary::after_capability_batch(vec![
                    LoopResultRef::new("result:done").expect("valid"),
                ]),
                pending_input_ack,
            },
        )
        .await
        .expect("stop stage");

    match step {
        StopStep::Stop {
            mut pending_input_ack,
            kind,
            ..
        } => {
            assert_eq!(kind, StopKind::GracefulStop);
            assert!(host.acked_input_tokens().is_empty());
            pending_input_ack.ack(&host).await.expect("ack inputs");
            assert_eq!(
                host.acked_input_tokens(),
                vec![LoopInputAckToken::new("input-ack:pending").expect("valid")]
            );
        }
        StopStep::Continue { .. } | StopStep::Exit(_) => panic!("expected graceful stop"),
    }
}

#[tokio::test]
async fn terminate_hint_after_batch_completes_without_extra_model_call() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:done").expect("valid"),
                safe_summary: "done".to_string(),
                terminate_hint: true,
            })],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::Final,
        ]
    );
    assert_eq!(
        host.progress_event_names(),
        vec![
            "iteration_started",
            "prompt_bundle_built",
            "checkpoint_written",
            "checkpoint_written",
            "capability_batch_started",
            "capability_batch_completed",
            "checkpoint_written",
        ]
    );
    let completed = host
        .progress_events()
        .into_iter()
        .find_map(|event| match event {
            ironclaw_turns::run_profile::LoopProgressEvent::CapabilityBatchCompleted {
                result_count,
                denied_count,
                gated_count,
                failed_count,
                ..
            } => Some((result_count, denied_count, gated_count, failed_count)),
            _ => None,
        })
        .expect("batch completed progress event");
    assert_eq!(completed, (1, 0, 0, 0));
}

#[tokio::test]
async fn gate_blocks_with_before_block_checkpoint() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:approval").expect("valid"),
                safe_summary: "approval required".to_string(),
            }],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Blocked(_)));
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::BeforeBlock,
        ]
    );
    assert_eq!(
        host.progress_event_names(),
        vec![
            "iteration_started",
            "prompt_bundle_built",
            "checkpoint_written",
            "checkpoint_written",
            "capability_batch_started",
            "capability_batch_completed",
            "gate_blocked",
            "checkpoint_written",
        ]
    );
    let completed = host
        .progress_events()
        .into_iter()
        .find_map(|event| match event {
            ironclaw_turns::run_profile::LoopProgressEvent::CapabilityBatchCompleted {
                result_count,
                denied_count,
                gated_count,
                failed_count,
                ..
            } => Some((result_count, denied_count, gated_count, failed_count)),
            _ => None,
        })
        .expect("batch completed progress event");
    assert_eq!(completed, (0, 0, 1, 0));
}

#[tokio::test]
async fn parallel_batch_records_completed_results_before_blocking_on_suspension() {
    let completed_ref = LoopResultRef::new("result:parallel-completed").expect("valid"); // safety: test-only fixture
    let host = MockHost::new(vec![two_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![
                CapabilityOutcome::ApprovalRequired {
                    gate_ref: LoopGateRef::new("gate:approval").expect("valid"), // safety: test-only fixture
                    safe_summary: "approval required".to_string(),
                },
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: completed_ref.clone(),
                    safe_summary: "parallel call completed".to_string(),
                    terminate_hint: false,
                }),
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute"); // safety: test-only assertion

    assert!(matches!(exit, LoopExit::Blocked(_))); // safety: test-only assertion
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1); // safety: test-only assertion
    assert_eq!(appended[0].result_ref, completed_ref); // safety: test-only assertion
    let before_block_refs =
        final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock).result_refs;
    assert!(before_block_refs == vec![completed_ref]); // safety: test-only assertion
}

#[tokio::test]
async fn non_empty_capability_batch_rejects_empty_outcomes() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: Vec::new(),
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("empty outcomes violate the host contract");

    if !matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "capability batch outcome count does not match invocations"
        }
    ) {
        panic!("expected planner contract error, got {error:?}");
    }
}

#[tokio::test]
async fn strategy_filtered_capability_denial_does_not_invoke_host_and_records_policy_denied() {
    let family = family_with_capability_filter(CapabilityFilter::Deny(vec![capability_id()]));
    let host = MockHost::new(vec![calls_response(), reply_response()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert!(host.batch_invocations().is_empty());
    assert!(host.single_invocations().is_empty());
    assert!(
        host.model_requests()[0]
            .capability_view
            .as_ref()
            .expect("model capability view")
            .visible_capability_ids
            .is_empty()
    );
    assert!(
        host.prompt_requests()[0]
            .capability_view
            .as_ref()
            .expect("prompt capability view")
            .visible_capability_ids
            .is_empty()
    );

    let staged_states = host
        .staged_payloads()
        .into_iter()
        .map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                checkpoint_kind_from_host(request.kind),
            )
            .expect("checkpoint payload")
        })
        .collect::<Vec<_>>();
    assert!(staged_states.iter().any(|state| {
        state
            .recent_failure_kinds
            .iter()
            .any(|kind| *kind == LoopFailureKind::PolicyDenied)
    }));
}

#[tokio::test]
async fn model_request_uses_current_visible_surface_not_prompt_bundle_version() {
    let host = MockHost::new(vec![reply_response()])
        .with_prompt_surface_version(Some(stale_surface_version()));
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].surface_version, Some(surface_version()));
}

#[tokio::test]
async fn model_retry_success_clears_recovery_state() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model unavailable",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(final_staged_state(&host).recovery_state, Default::default());
}

#[tokio::test]
async fn stale_surface_capability_call_is_policy_denied_before_host_invocation() {
    let host = MockHost::new(vec![stale_surface_calls_response(), reply_response()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert!(host.batch_invocations().is_empty());
    assert!(host.single_invocations().is_empty());

    let staged_states = host
        .staged_payloads()
        .into_iter()
        .map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                checkpoint_kind_from_host(request.kind),
            )
            .expect("checkpoint payload")
        })
        .collect::<Vec<_>>();
    assert!(staged_states.iter().any(|state| {
        state
            .recent_failure_kinds
            .iter()
            .any(|kind| *kind == LoopFailureKind::PolicyDenied)
    }));
    assert!(
        staged_states
            .iter()
            .any(|state| state.stop_state.last_batch_total == 0)
    );
}

#[tokio::test]
async fn last_batch_total_counts_only_visible_invoked_calls() {
    let host = MockHost::new(vec![mixed_surface_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:visible").expect("valid"),
                safe_summary: "visible call completed".to_string(),
                terminate_hint: true,
            })],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.completion_kind, LoopCompletionKind::ResultOnly);
            assert!(completed.reply_message_refs.is_empty());
            assert_eq!(
                completed.result_refs,
                vec![LoopResultRef::new("result:visible").expect("valid")]
            );
        }
        other => panic!("expected completed, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 1);

    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 1);
    assert_eq!(batch_invocations[0].invocations.len(), 1);
    assert!(!batch_invocations[0].stop_on_first_suspension);
    assert_eq!(
        batch_invocations[0].invocations[0].surface_version,
        surface_version()
    );
}

#[tokio::test]
async fn checkpoint_payload_rehydrates_with_written_marker() {
    let host = MockHost::new(vec![reply_response()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let staged_payloads = host.staged_payloads();
    let final_payload = staged_payloads
        .iter()
        .rev()
        .find(|request| request.kind == LoopCheckpointKind::Final)
        .expect("final checkpoint payload");
    let rehydrated =
        LoopExecutionState::from_checkpoint_payload(&final_payload.payload, CheckpointKind::Final)
            .expect("checkpoint payload");

    assert_eq!(
        rehydrated.last_checkpoint,
        Some(crate::state::CheckpointMarker {
            kind: CheckpointKind::Final,
            iteration_at_checkpoint: rehydrated.iteration,
        })
    );
}

#[tokio::test]
async fn retry_uses_single_call_invocation() {
    let host = MockHost::new(vec![calls_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Failed(
                ironclaw_turns::run_profile::CapabilityFailure {
                    error_kind: CapabilityFailureKind::Transient,
                    safe_summary: "temporary failure".to_string(),
                },
            )],
            stopped_on_suspension: false,
        }])
        .with_single_outcomes(vec![CapabilityOutcome::Completed(
            CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:retry").expect("valid"),
                safe_summary: "retry completed".to_string(),
                terminate_hint: true,
            },
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(final_staged_state(&host).recovery_state, Default::default());
}

#[tokio::test]
async fn policy_denied_capability_error_honors_retry_recovery() {
    let host = MockHost::new(vec![calls_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Denied(
                ironclaw_turns::run_profile::CapabilityDenied {
                    reason_kind:
                        ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "provider call denied".to_string(),
                },
            )],
            stopped_on_suspension: false,
        }])
        .with_single_outcomes(vec![CapabilityOutcome::Completed(
            CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:policy-retry").expect("valid"), // safety: test-only fixture
                safe_summary: "policy retry completed".to_string(),
                terminate_hint: true,
            },
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family_with_retry_policy_denied_recovery(), &host, state)
        .await
        .expect("execute"); // safety: test-only assertion

    assert!(matches!(exit, LoopExit::Completed(_))); // safety: test-only assertion
    assert_eq!(host.single_invocations().len(), 1); // safety: test-only assertion
    assert_eq!(final_staged_state(&host).recovery_state, Default::default()); // safety: test-only assertion
}

#[tokio::test]
async fn spawned_process_fails_closed_until_process_wait_contract_exists() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                process_ref: LoopProcessRef::new("process:alpha").expect("valid"),
                safe_summary: "spawned".to_string(),
            })],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::Final,
        ]
    );
}

#[tokio::test]
async fn completed_provider_call_appends_provider_replay_metadata() {
    let result_ref = LoopResultRef::new("result:provider-call").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "provider call completed".to_string(),
                terminate_hint: true,
            })],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    let provider_call = appended[0]
        .provider_call
        .as_ref()
        .expect("provider replay metadata");
    assert_eq!(provider_call.provider_turn_id, "turn_1");
    assert_eq!(provider_call.provider_call_id, "call_1");
    assert_eq!(provider_call.provider_tool_name, "demo__echo");
    assert_eq!(provider_call.capability_id, capability_id());
    assert_eq!(
        provider_call.arguments,
        serde_json::json!({"message":"hello"})
    );
    assert_eq!(
        provider_call.response_reasoning.as_deref(),
        Some("response reasoning")
    );
    assert_eq!(provider_call.reasoning.as_deref(), Some("call reasoning"));
    assert_eq!(provider_call.signature.as_deref(), Some("sig-1"));
}

#[tokio::test]
async fn denied_provider_call_appends_failure_tool_result_for_replay() {
    let result_ref = LoopResultRef::new("result:provider-call").expect("valid");
    let host = MockHost::new(vec![provider_two_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: result_ref.clone(),
                    safe_summary: "provider call completed".to_string(),
                    terminate_hint: true,
                }),
                CapabilityOutcome::Denied(ironclaw_turns::run_profile::CapabilityDenied {
                    reason_kind:
                        ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "provider call denied".to_string(),
                }),
            ],
            stopped_on_suspension: false,
        }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 2);
    assert_eq!(appended[0].result_ref, result_ref);
    assert_eq!(appended[0].safe_summary, "provider call completed");
    assert_eq!(appended[1].safe_summary, "provider call denied");
    assert!(
        appended[1]
            .result_ref
            .as_str()
            .starts_with("result:provider-error-turn_1-call_2")
    );
    let denied_provider_call = appended[1]
        .provider_call
        .as_ref()
        .expect("provider replay metadata");
    assert_eq!(denied_provider_call.provider_turn_id, "turn_1");
    assert_eq!(denied_provider_call.provider_call_id, "call_2");
    assert_eq!(denied_provider_call.provider_tool_name, "demo__echo");
    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(
                completed.result_refs,
                vec![result_ref.clone(), appended[1].result_ref.clone()]
            );
        }
        other => panic!("expected completed, got {other:?}"),
    }
    assert_eq!(
        final_staged_state(&host).result_refs,
        vec![result_ref, appended[1].result_ref.clone()]
    );
}
