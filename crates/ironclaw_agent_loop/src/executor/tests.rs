use ironclaw_turns::{
    LoopCancelledReasonKind, LoopCompletionKind, LoopDiagnosticRef, LoopExit, LoopFailureKind,
    LoopGateRef, LoopResultRef, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityCallCandidate, CapabilityFailureKind,
        CapabilityInputRef, CapabilityOutcome, CapabilityResultMessage, LoopCancelReasonKind,
        LoopCheckpointKind, LoopCompactionError, LoopCompactionResponse, LoopContextCompactionKind,
        LoopContextCompactionMetadata, LoopInput, LoopInputAckToken, LoopInputBatch,
        LoopInputCursor, LoopInterruptKind, LoopProcessRef, LoopRunInfoPort, LoopSafeSummary,
        LoopSummaryArtifactId, ParentLoopOutput, ProcessHandleSummary, ProviderToolCallReplay,
        VisibleCapabilityRequest,
    },
};

use crate::state::{CheckpointKind, IndexedMessageKind, LoopExecutionState, MessageIndexEntry};
use crate::strategies::{
    CapabilityFilter, DefaultCompactionStrategy, GateKind, GateOutcome, StopKind, TurnSummary,
};

use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AssistantReplyInput, AssistantReplyStage, BatchStep,
    BudgetInput, BudgetStage, BudgetStep, CanonicalAgentLoopExecutor, CapabilityInput,
    CapabilityStage, DrainInput, ExecutorStage, ExitInput, ExitStage, GateInput, GateStage,
    HostStage, InputStage, InputStep, PendingInputAck, PromptInput, PromptStage, PromptStep,
    StageContext, StopInput, StopStage, StopStep, TurnCompletedStep, UserFacingInputDrainMode,
    consume_drainable_inputs, sanitize_result_ref_suffix, synthetic_provider_error_result_ref,
};

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}

fn compaction_metadata(
    sequence: u64,
    kind: LoopContextCompactionKind,
    estimated_tokens: u64,
) -> LoopContextCompactionMetadata {
    LoopContextCompactionMetadata {
        sequence,
        kind,
        estimated_tokens,
    }
}

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
async fn reply_only_drains_follow_up_before_stop_strategy_completes() {
    let host = MockHost::new(vec![reply_response(), reply_response()]);
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![
        LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: input_cursor(&run_context, "input-cursor:no-input"),
        },
        LoopInputBatch {
            inputs: vec![LoopInput::FollowUp {
                message_ref: message_ref("msg:follow-up"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-follow-up",
                "input-ack:after-follow-up",
            )],
            next_cursor: input_cursor(&run_context, "input-cursor:after-follow-up"),
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:after-follow-up").expect("valid")]
    );
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::Final,
        ]
    );
    assert_eq!(final_staged_state(&host).stop_state.turns_completed, 2);
}

#[tokio::test]
async fn reply_only_uses_configured_stop_strategy_decision() {
    let host = MockHost::new(vec![reply_response(), reply_response()]);
    let family = family_with_stop_after_observed_turns(2);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::Final,
        ]
    );
    assert_eq!(final_staged_state(&host).stop_state.turns_completed, 2);
}

#[tokio::test]
async fn budget_stage_exits_at_iteration_limit() {
    let host = MockHost::new(Vec::new());
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 1,
        ..Default::default()
    });
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
async fn prompt_stage_compacts_candidate_prompt_then_rebuilds_final_bundle() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(vec![
            vec![
                compaction_metadata(1, LoopContextCompactionKind::User, 10),
                compaction_metadata(2, LoopContextCompactionKind::Assistant, 10),
            ],
            vec![compaction_metadata(
                2,
                LoopContextCompactionKind::Assistant,
                10,
            )],
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 1,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    let output = match step {
        PromptStep::Prepared(output) => output,
        PromptStep::Exit(exit) => panic!("expected prepared prompt, got {exit:?}"),
    };
    assert_eq!(host.prompt_requests().len(), 2);
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        Some(1)
    );
    assert!(
        !output
            .state
            .compaction_state
            .force_compact_on_next_iteration
    );
    assert_eq!(
        output.state.compaction_prompt.message_index,
        vec![MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 10,
        }]
    );
    assert_eq!(output.state.compaction_prompt.observed_prompt_tokens, 10);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel]
    );
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "compaction_completed",
            "checkpoint_written",
            "prompt_bundle_built",
        ]
    );
}

#[tokio::test]
async fn prompt_stage_compaction_index_maps_system_summary_and_other_kinds() {
    let host = MockHost::new(Vec::new()).with_prompt_compaction_index(vec![
        compaction_metadata(1, LoopContextCompactionKind::System, 4),
        compaction_metadata(2, LoopContextCompactionKind::Summary, 5),
        compaction_metadata(3, LoopContextCompactionKind::Other, 6),
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    let output = match step {
        PromptStep::Prepared(output) => output,
        PromptStep::Exit(exit) => panic!("expected prepared prompt, got {exit:?}"),
    };
    assert_eq!(
        output.state.compaction_prompt.message_index,
        vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::System,
                estimated_tokens: 4,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Summary,
                estimated_tokens: 5,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::Other,
                estimated_tokens: 6,
            },
        ]
    );
    assert_eq!(host.prompt_requests().len(), 1);
}

#[tokio::test]
async fn prompt_stage_cancellation_after_prompt_bundle_returns_cancelled_exit() {
    let host = MockHost::new(Vec::new()).cancel_after_prompt_bundle(1);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    match step {
        PromptStep::Exit(LoopExit::Cancelled(cancelled)) => {
            assert!(cancelled.checkpoint_id.is_some());
        }
        PromptStep::Prepared(_) => panic!("expected cancelled exit"),
        PromptStep::Exit(exit) => panic!("expected cancelled exit, got {exit:?}"),
    }
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.progress_event_names(),
        vec!["prompt_bundle_built", "checkpoint_written"]
    );
}

#[tokio::test]
async fn prompt_stage_compaction_timeout_returns_failed_exit() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .with_compaction_delay(std::time::Duration::from_millis(25));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 1,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    match step {
        PromptStep::Exit(LoopExit::Failed(failed)) => {
            assert!(failed.checkpoint_id.is_some());
        }
        _ => panic!("expected failed exit"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "compaction_failed",
            "checkpoint_written",
        ]
    );
}

#[tokio::test]
async fn prompt_stage_compaction_security_rejection_returns_failed_exit() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Err(LoopCompactionError::SecurityRejected {
            safe_summary: LoopSafeSummary::new("injection detected").unwrap(),
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 100,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    match step {
        PromptStep::Exit(LoopExit::Failed(failed)) => {
            assert!(failed.checkpoint_id.is_some());
        }
        PromptStep::Prepared(_) => panic!("security rejection should end the run"),
        PromptStep::Exit(exit) => panic!("expected failed exit, got {exit:?}"),
    }
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "compaction_failed",
            "checkpoint_written",
        ]
    );
}

#[tokio::test]
async fn prompt_stage_compaction_cancelled_returns_cancelled_exit() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Err(LoopCompactionError::Cancelled));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 100,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    match step {
        PromptStep::Exit(LoopExit::Cancelled(cancelled)) => {
            assert!(cancelled.checkpoint_id.is_some());
        }
        _ => panic!("expected cancelled exit"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn prompt_stage_cancellation_during_compaction_aborts_prompt_planning() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .with_compaction_delay(std::time::Duration::from_millis(50));
    let host_for_cancel = host.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        host_for_cancel.request_cancellation(LoopCancelReasonKind::UserRequested);
    });
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 500,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    assert!(matches!(step, PromptStep::Exit(LoopExit::Cancelled(_))));
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn prompt_stage_compaction_aborts_immediately_when_cancellation_already_set() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .with_compaction_delay(std::time::Duration::from_secs(1))
        .cancel_on_compaction_start();
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 5_000,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        PromptStage.process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        ),
    )
    .await
    .expect("already-requested cancellation should not wait for compaction")
    .expect("prompt stage");

    assert!(matches!(step, PromptStep::Exit(LoopExit::Cancelled(_))));
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn prompt_stage_cancellation_after_compaction_success_skips_final_bundle_rebuild() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(vec![
            vec![compaction_metadata(1, LoopContextCompactionKind::User, 10)],
            vec![],
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .cancel_after_compaction_success();
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 100,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage");

    assert!(matches!(step, PromptStep::Exit(LoopExit::Cancelled(_))));
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "checkpoint_written",
        ]
    );
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
            DrainInput {
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
async fn input_stage_steering_input_is_drained_like_user_message() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![LoopInput::Steering {
            message_ref: message_ref("msg:steering-drained"),
        }],
        input_acks: vec![input_ack(
            &run_context,
            "input-cursor:after-steering",
            "input-ack:after-steering",
        )],
        next_cursor: input_cursor(&run_context, "input-cursor:after-steering"),
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
            DrainInput {
                state,
                pending_input_ack: PendingInputAck::default(),
                mode: UserFacingInputDrainMode::Steering,
            },
        )
        .await
        .expect("input stage");

    match step {
        InputStep::Continue { state, drained, .. } => {
            assert!(drained);
            assert_eq!(
                state.input_cursor,
                input_cursor(&run_context, "input-cursor:after-steering")
            );
        }
        InputStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[test]
fn consume_drainable_inputs_empty_batch_short_circuits() {
    let host = MockHost::new(Vec::new());
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    let before_cursor = state.input_cursor.clone();
    let batch = LoopInputBatch {
        inputs: Vec::new(),
        input_acks: Vec::new(),
        next_cursor: before_cursor.clone(),
    };

    let (drained, ack_tokens, cancelled_reason_kind) =
        consume_drainable_inputs(&batch, UserFacingInputDrainMode::Steering, &mut state)
            .expect("consume inputs");

    assert!(!drained);
    assert!(ack_tokens.is_empty());
    assert!(cancelled_reason_kind.is_none());
    assert_eq!(state.input_cursor, before_cursor);
}

#[test]
fn consume_drainable_inputs_returns_planner_contract_error_when_acks_missing() {
    let host = MockHost::new(Vec::new());
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    let batch = LoopInputBatch {
        inputs: vec![LoopInput::Steering {
            message_ref: message_ref("msg:steering-missing-ack"),
        }],
        input_acks: Vec::new(),
        next_cursor: state.input_cursor.clone(),
    };

    let error = consume_drainable_inputs(&batch, UserFacingInputDrainMode::Steering, &mut state)
        .expect_err("missing ack metadata violates the host contract");

    assert!(matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "input batch omitted ack metadata for consumed inputs"
        }
    ));
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
async fn reply_admission_rejects_candidate_before_finalizing_and_continues() {
    let result_ref = LoopResultRef::new("result:done").expect("valid");
    let host = MockHost::new(vec![reply_response(), calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "done".to_string(),
                terminate_hint: false,
            })],
            stopped_on_suspension: false,
        }]);
    let family = family_with_reply_admission(FixedReplyAdmissionPolicy::RejectFirst);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 3);
    let prompt_requests = host.prompt_requests();
    assert_eq!(prompt_requests.len(), 3);
    assert!(prompt_requests[0].inline_messages.is_empty());
    assert_eq!(prompt_requests[1].inline_messages.len(), 1);
    assert_eq!(
        prompt_requests[1].inline_messages[0].safe_body.as_str(),
        "loop control reply rejected stop condition not met continue"
    );
    assert!(prompt_requests[2].inline_messages.is_empty());

    let before_model_states = host
        .staged_payloads()
        .into_iter()
        .filter(|request| request.kind == LoopCheckpointKind::BeforeModel)
        .map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                CheckpointKind::BeforeModel,
            )
            .expect("checkpoint payload")
        })
        .collect::<Vec<_>>();
    assert!(before_model_states.iter().any(|state| {
        state.reply_admission_state.pending_rejection.is_some()
            && state.reply_admission_state.pending_rejection_rendered
    }));

    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.assistant_refs,
        vec![message_ref("msg:assistant")]
    );
    assert_eq!(
        final_state.reply_admission_state.rejected_reply_candidates,
        1
    );
    assert!(
        final_state
            .reply_admission_state
            .pending_rejection
            .is_none()
    );
    assert_eq!(final_state.stop_state.turns_completed, 3);
}

#[tokio::test]
async fn reply_admission_rendered_flag_stays_false_when_context_suppresses_control_message() {
    let result_ref = LoopResultRef::new("result:done").expect("valid");
    let host = MockHost::new(vec![reply_response(), calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "done".to_string(),
                terminate_hint: false,
            })],
            stopped_on_suspension: false,
        }]);
    let family =
        family_with_reply_admission_without_inline_context(FixedReplyAdmissionPolicy::RejectFirst);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert!(
        host.prompt_requests()
            .iter()
            .all(|request| request.inline_messages.is_empty())
    );

    let before_model_states = host
        .staged_payloads()
        .into_iter()
        .filter(|request| request.kind == LoopCheckpointKind::BeforeModel)
        .map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                CheckpointKind::BeforeModel,
            )
            .expect("checkpoint payload")
        })
        .collect::<Vec<_>>();
    assert!(before_model_states.iter().any(|state| {
        state.reply_admission_state.pending_rejection.is_some()
            && !state.reply_admission_state.pending_rejection_rendered
    }));
}

#[tokio::test]
async fn repeated_reply_rejections_stop_as_invalid_model_output() {
    let host = MockHost::new(vec![reply_response(), reply_response(), reply_response()]);
    let family = family_with_reply_admission(FixedReplyAdmissionPolicy::RejectAlways);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::InvalidModelOutput);
        }
        other => panic!("expected failed invalid-model-output exit, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 3);
    let final_state = final_staged_state(&host);
    assert!(final_state.assistant_refs.is_empty());
    assert_eq!(
        final_state.reply_admission_state.rejected_reply_candidates,
        3
    );
    assert_eq!(final_state.stop_state.trailing_rejected_replies, 3);
}

#[tokio::test]
async fn default_reply_admission_rejects_tool_history_echo_and_continues() {
    let host = MockHost::new(vec![
        reply_response_with_text("Previous tool event: demo__echo was invoked."),
        reply_response_with_text("done"),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.assistant_refs,
        vec![message_ref("msg:assistant")]
    );
    assert_eq!(
        final_state.reply_admission_state.rejected_reply_candidates,
        1
    );
    assert_eq!(final_state.stop_state.turns_completed, 2);
}

#[tokio::test]
async fn prompt_stage_host_unavailable_on_visible_capabilities_propagates_error() {
    let host = MockHost::new(Vec::new()).with_failing_visible_capabilities();
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await;
    let error = match result {
        Ok(_) => panic!("visible capabilities failure should propagate"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability
        }
    ));
}

#[tokio::test]
async fn prompt_stage_host_unavailable_on_build_prompt_bundle_propagates_error() {
    let host = MockHost::new(Vec::new()).with_failing_prompt_bundle();
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await;
    let error = match result {
        Ok(_) => panic!("prompt bundle failure should propagate"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Prompt
        }
    ));
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
            CapabilityInput {
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

#[test]
fn sanitize_result_ref_suffix_handles_empty_special_chars_and_truncation() {
    assert_eq!(sanitize_result_ref_suffix(""), "unknown");
    assert_eq!(
        sanitize_result_ref_suffix("turn/with spaces:and?symbols"),
        "turn-with-spaces-and-symbols"
    );

    let oversized = "a".repeat(300);
    let sanitized = sanitize_result_ref_suffix(&oversized);
    assert_eq!(sanitized.len(), 300);

    let result_ref = synthetic_provider_error_result_ref(&CapabilityCallCandidate {
        surface_version: surface_version(),
        capability_id: capability_id(),
        input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            provider_turn_id: oversized,
            provider_call_id: "call/with space".to_string(),
            provider_tool_name: "demo__echo".to_string(),
            arguments: serde_json::json!({}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }),
    })
    .expect("synthetic provider error ref");
    assert!(result_ref.as_str().starts_with("result:provider-error-"));
    assert_eq!("result:".len() + 240, result_ref.as_str().len());
}

#[tokio::test]
async fn exit_stage_no_progress_detected_exits_with_failed_loop_exit() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = ExitStage
        .process(
            ctx,
            ExitInput {
                state,
                kind: StopKind::NoProgressDetected,
            },
        )
        .await
        .expect("exit stage");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
}

#[tokio::test]
async fn exit_stage_aborted_exits_with_requested_failure_kind() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = ExitStage
        .process(
            ctx,
            ExitInput {
                state,
                kind: StopKind::Aborted(LoopFailureKind::CapabilityProtocolError),
            },
        )
        .await
        .expect("exit stage");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
}

#[tokio::test]
async fn stopped_on_suspension_completed_outcome_still_appends_result() {
    let result_ref = LoopResultRef::new("result:stopped-completed").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "stopped batch completed".to_string(),
                terminate_hint: true,
            })],
            stopped_on_suspension: true,
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
            assert_eq!(completed.result_refs, vec![result_ref.clone()]);
        }
        other => panic!("expected completed, got {other:?}"),
    }
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].result_ref, result_ref);
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
async fn gate_stage_skips_and_continues_records_skipped_summary() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-skip").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Auth,
                gate_ref,
            },
        )
        .await
        .expect("gate stage");

    let BatchStep::Continue(state) = step else {
        panic!("expected skip-and-continue");
    };
    assert_eq!(state.result_refs.len(), 1);
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].safe_summary, "auth gate skipped");
    assert!(host.checkpoint_kinds().is_empty());
}

#[tokio::test]
async fn gate_stage_aborts_returns_failed_exit() {
    let failure_kind = LoopFailureKind::CapabilityProtocolError;
    let family = family_with_gate_outcome(GateOutcome::Abort {
        gate: empty_gate_state(),
        failure_kind,
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-abort").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Auth,
                gate_ref,
            },
        )
        .await
        .expect("gate stage");

    match step {
        BatchStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(failed.reason_kind, failure_kind);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].safe_summary, "auth gate aborted");
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
async fn capability_batch_rejects_outcome_count_exceeding_invocation_count() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:first").expect("valid"),
                    safe_summary: "first".to_string(),
                    terminate_hint: false,
                }),
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:second").expect("valid"),
                    safe_summary: "second".to_string(),
                    terminate_hint: false,
                }),
            ],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("too many outcomes violate the host contract");

    assert!(matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "capability batch outcome count does not match invocations"
        }
    ));
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
        !host
            .progress_event_names()
            .contains(&"capability_batch_started")
    );
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
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model unavailable",
        )])
        .with_prompt_compaction_indexes(vec![
            vec![compaction_metadata(1, LoopContextCompactionKind::User, 10)],
            vec![
                compaction_metadata(2, LoopContextCompactionKind::System, 20),
                compaction_metadata(3, LoopContextCompactionKind::Assistant, 30),
            ],
        ])
        .with_prompt_surface_version(Some(stale_surface_version()));
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].surface_version, Some(surface_version()));
    assert_eq!(requests[1].surface_version, Some(surface_version()));
    assert_eq!(
        host.prompt_requests().len(),
        2,
        "model retry must request a fresh host-built prompt bundle"
    );
    let final_state = final_staged_state(&host);
    assert_eq!(final_state.recovery_state, Default::default());
    assert_eq!(
        final_state.compaction_prompt.message_index,
        vec![
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::System,
                estimated_tokens: 20,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 30,
            },
        ]
    );
    assert_eq!(final_state.compaction_prompt.observed_prompt_tokens, 50);
}

#[tokio::test]
async fn model_unrecoverable_host_error_preserves_sanitized_diagnostics() {
    let diagnostic_ref = LoopDiagnosticRef::new("diag:model-credentials").expect("valid");
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::CredentialUnavailable,
            "model credentials are unavailable",
        )
        .with_diagnostic_ref(diagnostic_ref),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("credential errors should stop before a loop exit");

    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new("model credentials are unavailable").expect("safe"),
            diagnostic_ref: Some(LoopDiagnosticRef::new("diag:model-credentials").expect("valid")),
        }
    );
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
    for error_kind in [
        CapabilityFailureKind::Transient,
        CapabilityFailureKind::Network,
    ] {
        let host = MockHost::new(vec![calls_response()])
            .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Failed(
                    ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind,
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
async fn spawned_child_run_result_append_failure_propagates_without_completed_result() {
    let result_ref = LoopResultRef::new("result:spawned-child").expect("valid");
    let host = MockHost::new(vec![calls_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::SpawnedChildRun {
                child_run_id: TurnRunId::new(),
                result_ref,
                safe_summary: "spawned child completed".to_string(),
            }],
            stopped_on_suspension: false,
        }])
        .with_failing_result_append();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .unwrap_err();

    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability
        }
    );
    assert!(host.appended_result_refs().is_empty());
}

#[tokio::test]
async fn spawned_child_run_rejects_unsafe_safe_summary_without_appending_result() {
    let result_ref = LoopResultRef::new("result:spawned-child").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::SpawnedChildRun {
                child_run_id: TurnRunId::new(),
                result_ref,
                safe_summary: "/Users/alice/.ssh/id_rsa".to_string(),
            }],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .unwrap_err();

    assert_eq!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "host returned unsafe strategy summary"
        }
    );
    assert!(host.appended_result_refs().is_empty());
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
    assert_eq!(
        appended[1].safe_summary,
        "capability denied with empty_surface: provider call denied"
    );
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

#[tokio::test]
async fn model_visible_provider_tool_failures_append_failure_tool_result_for_replay() {
    for (error_kind, safe_summary, expected_summary) in [
        (
            CapabilityFailureKind::InvalidInput,
            "invalid input",
            "capability failed with invalid_input: invalid input",
        ),
        (
            CapabilityFailureKind::MissingRuntime,
            "runtime missing",
            "capability failed with missing_runtime: runtime missing",
        ),
        (
            CapabilityFailureKind::OperationFailed,
            "operation failed",
            "capability failed with operation_failed: operation failed",
        ),
        (
            CapabilityFailureKind::OutputTooLarge,
            "response body exceeded limit 10000000",
            "capability failed with output_too_large: response body exceeded limit 10000000",
        ),
    ] {
        let host = MockHost::new(vec![provider_calls_response(), reply_response()])
            .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Failed(
                    ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind,
                        safe_summary: safe_summary.to_string(),
                    },
                )],
                stopped_on_suspension: false,
            }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        let appended = host.appended_result_refs();
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].safe_summary, expected_summary);
        assert!(
            appended[0]
                .result_ref
                .as_str()
                .starts_with("result:provider-error-turn_1-call_1")
        );
        let provider_call = appended[0]
            .provider_call
            .as_ref()
            .expect("provider replay metadata");
        assert_eq!(provider_call.provider_turn_id, "turn_1");
        assert_eq!(provider_call.provider_call_id, "call_1");
        assert_eq!(provider_call.provider_tool_name, "demo__echo");
        match exit {
            LoopExit::Completed(completed) => {
                assert_eq!(completed.result_refs, vec![appended[0].result_ref.clone()]);
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert_eq!(
            final_staged_state(&host).result_refs,
            vec![appended[0].result_ref.clone()]
        );
    }

    let long_summary = "a".repeat(512);
    let host = MockHost::new(vec![provider_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Failed(
                ironclaw_turns::run_profile::CapabilityFailure {
                    error_kind: CapabilityFailureKind::OutputTooLarge,
                    safe_summary: long_summary,
                },
            )],
            stopped_on_suspension: false,
        }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert!(appended[0].safe_summary.len() <= 512);
    assert!(
        appended[0]
            .safe_summary
            .starts_with("capability failed with output_too_large: ")
    );
}
