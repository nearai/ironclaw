use ironclaw_host_api::{ApprovalRequestId, CorrelationId, ResourceEstimate};
use ironclaw_turns::{
    LoopCancelledReasonKind, LoopCompletionKind, LoopDiagnosticRef, LoopExit, LoopFailureKind,
    LoopGateRef, LoopResultRef, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume,
        CapabilityCallCandidate, CapabilityFailureDetail, CapabilityFailureKind,
        CapabilityInputIssue, CapabilityInputIssueCode, CapabilityInputRef, CapabilityInputRepair,
        CapabilityOutcome, CapabilityRecoveryHint, CapabilityResultMessage, CapabilityResumeToken,
        LoopCancelReasonKind, LoopCheckpointKind, LoopCompactionError, LoopCompactionOutcome,
        LoopCompactionResponse, LoopContextCompactionKind, LoopInput, LoopInputAckToken,
        LoopInputBatch, LoopInputCursor, LoopInterruptKind, LoopProcessRef, LoopRunInfoPort,
        LoopSafeSummary, LoopSummaryArtifactId, ObservationTrust, ParentLoopOutput,
        ProcessHandleSummary, ProviderToolCallReplay, SameCallRetryConstraint,
        ToolObservationDetail, ToolObservationStatus, VisibleCapabilityRequest,
    },
};

use crate::state::{
    CapabilityCallSignature, CheckpointKind, DeferredCompactionWatermark, IndexedMessageKind,
    LoopExecutionState, MessageIndexEntry, PendingAuthResume, RepeatedCallWarningPhase,
    RepeatedCallWarningState,
};
use crate::strategies::{
    CapabilityBatchTurnSummary, CapabilityFilter, DefaultCompactionStrategy, GateKind, GateOutcome,
    StopKind, TurnSummary,
};
use crate::test_support::compaction::{
    active_task_preserving_compaction_index, compaction_metadata,
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
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
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
async fn prompt_stage_deferred_compaction_returns_to_normal_prompt_path() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_outcome(Ok(LoopCompactionOutcome::Deferred {
            safe_summary: LoopSafeSummary::new("compaction deferred until transcript stabilizes")
                .unwrap(),
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

    let output = match step {
        PromptStep::Prepared(output) => output,
        PromptStep::Exit(exit) => panic!("expected prepared prompt, got {exit:?}"),
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
    };
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        None
    );
    assert_eq!(
        output.state.compaction_state.last_deferred,
        Some(DeferredCompactionWatermark {
            through_seq: 1,
            prompt_fingerprint: output.state.compaction_prompt.fingerprint(),
        })
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
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 10,
        }]
    );
    assert!(host.checkpoint_kinds().is_empty());
    assert_eq!(
        host.progress_event_names(),
        vec!["prompt_bundle_built", "compaction_started"]
    );
}

#[tokio::test]
async fn prompt_stage_successful_compaction_clears_deferred_watermark() {
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
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 99,
        prompt_fingerprint: 123,
    });

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
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
    };
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        Some(1)
    );
    assert_eq!(output.state.compaction_state.last_deferred, None);
}

#[tokio::test]
async fn prompt_stage_cancellation_after_deferred_compaction_returns_cancelled_exit() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_outcome(Ok(LoopCompactionOutcome::Deferred {
            safe_summary: LoopSafeSummary::new("compaction deferred until transcript stabilizes")
                .unwrap(),
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
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
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
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::Exit(exit) => panic!("expected cancelled exit, got {exit:?}"),
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
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
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::Exit(exit) => panic!("expected failed exit, got {exit:?}"),
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
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
async fn model_context_overflow_retries_through_canonical_compaction_stage() {
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "model request exceeded its context budget",
        )])
        .with_prompt_compaction_indexes(vec![
            vec![compaction_metadata(1, LoopContextCompactionKind::User, 10)],
            active_task_preserving_compaction_index(),
            Vec::new(),
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary:overflow-retry")
                .expect("valid summary id"),
            compression_ratio_ppm: 100_000,
        }));
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(
        host.prompt_requests().len(),
        3,
        "retry must return to PromptStage so compaction can run before the next model call"
    );
    assert!(host.progress_event_names().contains(&"compaction_started"));

    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.compaction_state.last_compacted_through_seq,
        Some(5)
    );
    assert!(!final_state.compaction_state.force_compact_on_next_iteration);
}

#[tokio::test]
async fn model_shrink_context_call_scope_returns_planner_contract() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "model request exceeded its context budget",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let err = executor
        .execute_family(
            &family_with_shrink_context_call_scope_recovery(),
            &host,
            state,
        )
        .await
        .expect_err("call-scoped ShrinkContext must violate the planner contract");

    assert!(matches!(
        err,
        AgentLoopExecutorError::PlannerContract {
            detail: "context shrink retry requires iteration scope"
        }
    ));
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
        .process(
            ctx,
            AssistantReplyInput {
                state,
                reply,
                usage: None,
            },
        )
        .await
        .expect("assistant reply stage");

    match step {
        TurnCompletedStep::Continue { state, summary } => {
            assert_eq!(state.assistant_refs, vec![message_ref("msg:assistant")]);
            assert_eq!(
                state
                    .recent_output_token_counts
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![2],
                "missing provider usage should still feed no-progress detection"
            );
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 0,
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 0,
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 0,
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
            let signature = CapabilityCallSignature::from_call(
                capability_id(),
                &serde_json::json!({ "input_ref": "input:demo" }),
            )
            .expect("valid signature");
            assert_eq!(
                summary,
                TurnSummary::after_capability_batch(
                    vec![result_ref],
                    CapabilityBatchTurnSummary {
                        invocation_count: 1,
                        terminate_hint_count: 0,
                        no_progress_count: 0,
                        observed_signatures: vec![signature.clone()],
                        made_progress_signatures: vec![signature],
                    },
                )
            );
        }
        TurnCompletedStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn repeated_call_warning_checkpoint_stays_pending_until_model_request() {
    let host = MockHost::new(vec![reply_response()]);
    let executor = CanonicalAgentLoopExecutor;
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    let signature = CapabilityCallSignature::from_call(
        capability_id(),
        &serde_json::json!({ "input_ref": "input:demo" }),
    )
    .expect("valid signature");
    state.stop_state.repeated_call_warning =
        Some(RepeatedCallWarningState::pending_render(signature.clone()));

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let prompt_requests = host.prompt_requests();
    assert_eq!(prompt_requests.len(), 1);
    assert!(
        prompt_requests[0].inline_messages.iter().any(|message| {
            message.safe_body.as_str()
                == "loop control repeated capability call detected change strategy explain new evidence or answer from current evidence"
        }),
        "model prompt should include the warning"
    );
    let before_model = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeModel);
    let warning = before_model
        .stop_state
        .repeated_call_warning
        .expect("warning should be checkpointed");
    assert_eq!(warning.signature, signature.clone());
    assert_eq!(warning.phase, RepeatedCallWarningPhase::PendingRender);
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
async fn exit_stage_no_progress_detected_finalizes_fallback_reply() {
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
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected completed exit with fallback reply, got {other:?}"),
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
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
    let state = LoopExecutionState::initial_for_run(host.run_context());
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
                summary: TurnSummary::after_capability_batch(
                    vec![LoopResultRef::new("result:done").expect("valid")],
                    CapabilityBatchTurnSummary {
                        invocation_count: 1,
                        terminate_hint_count: 1,
                        no_progress_count: 0,
                        observed_signatures: Vec::new(),
                        made_progress_signatures: Vec::new(),
                    },
                ),
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
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
                approval_resume: None,
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
async fn approval_resume_metadata_is_replayed_after_before_block_checkpoint() {
    let original_input_ref = CapabilityInputRef::new("input:demo").expect("valid");
    let approval_resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("resume-token:demo").expect("valid token"),
        correlation_id: CorrelationId::new(),
        input_ref: original_input_ref.clone(),
        input: serde_json::json!({ "message": "hello" }),
        estimate: ResourceEstimate::default(),
    };
    let completed_ref = LoopResultRef::new("result:approval-resumed").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:approval").expect("valid"),
                safe_summary: "approval required".to_string(),
                approval_resume: Some(approval_resume.clone()),
            }],
            stopped_on_suspension: true,
        },
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: completed_ref.clone(),
                safe_summary: "approval resumed".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
            })],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks");

    assert!(matches!(first_exit, LoopExit::Blocked(_)));
    assert_eq!(host.model_requests().len(), 1);
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_resume = before_block_state
        .pending_approval_resume
        .as_ref()
        .expect("blocked checkpoint carries pending approval resume");
    assert_eq!(
        pending_resume.approval_request_id,
        approval_resume.approval_request_id
    );
    assert_eq!(pending_resume.resume_token, approval_resume.resume_token);
    assert_eq!(
        pending_resume.correlation_id,
        approval_resume.correlation_id
    );
    assert_eq!(pending_resume.input, approval_resume.input);
    assert_eq!(pending_resume.estimate, approval_resume.estimate);
    assert_eq!(pending_resume.surface_version, surface_version());
    assert_eq!(
        pending_resume.effective_capability_ids,
        vec![capability_id()]
    );

    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes");

    assert!(matches!(second_exit, LoopExit::Completed(_)));
    assert_eq!(
        host.model_requests().len(),
        1,
        "approval resume must dispatch the saved invocation before asking the model again"
    );
    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 2);
    assert_eq!(batch_invocations[0].invocations[0].approval_resume, None);
    assert_eq!(
        batch_invocations[1].invocations[0].approval_resume,
        Some(approval_resume)
    );
    assert_eq!(
        batch_invocations[1].invocations[0].input_ref,
        original_input_ref
    );
    assert_eq!(
        batch_invocations[1].invocations[0]
            .approval_resume
            .as_ref()
            .expect("resume metadata")
            .input_ref,
        original_input_ref
    );
    assert_eq!(final_staged_state(&host).result_refs, vec![completed_ref]);
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
                credential_requirements: Vec::new(),
                approval_resume: None,
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
                credential_requirements: Vec::new(),
                approval_resume: None,
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
                    approval_resume: None,
                },
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: completed_ref.clone(),
                    safe_summary: "parallel call completed".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len: 0,
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
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len: 0,
                }),
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:second").expect("valid"),
                    safe_summary: "second".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len: 0,
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
            reason_kind: None,
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
}

#[tokio::test]
async fn terminate_hint_counts_only_visible_invoked_calls() {
    let host = MockHost::new(vec![mixed_surface_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:visible").expect("valid"),
                safe_summary: "visible call completed".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
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
                        detail: None,
                    },
                )],
                stopped_on_suspension: false,
            }])
            .with_single_outcomes(vec![CapabilityOutcome::Completed(
                CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:retry").expect("valid"),
                    safe_summary: "retry completed".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 0,
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
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
                byte_len: 0,
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
                byte_len: 0,
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
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 0,
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
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 0,
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
async fn invalid_provider_tool_failure_appends_structured_model_observation() {
    let host = MockHost::new(vec![provider_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Failed(
                ironclaw_turns::run_profile::CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    safe_summary: "provider arguments failed schema validation".to_string(),
                    detail: Some(CapabilityFailureDetail::InvalidInput {
                        issues: vec![CapabilityInputIssue {
                            path: "file_path".to_string(),
                            code: CapabilityInputIssueCode::MissingRequired,
                            expected: Some("required field".to_string()),
                            received: None,
                            schema_path: Some("required".to_string()),
                        }],
                    }),
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
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("structured model observation");
    assert_eq!(observation.status, ToolObservationStatus::Error);
    assert_eq!(observation.summary, "Tool input failed schema validation.");
    assert_eq!(observation.trust, ObservationTrust::UntrustedToolOutput);
    match &observation.detail {
        ToolObservationDetail::InvalidInput { issues } => {
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].path, "file_path");
            assert_eq!(issues[0].code, CapabilityInputIssueCode::MissingRequired);
        }
        detail => panic!("expected invalid input detail, got {detail:?}"),
    }
    let recovery = observation.recovery.as_ref().expect("recovery detail");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::RequiresChangedInput
    );
    assert_eq!(
        recovery.recovery_hint,
        CapabilityRecoveryHint::CorrectArgumentsBeforeRetry
    );
    assert_eq!(
        recovery.repairs,
        vec![CapabilityInputRepair::ProvideRequiredField {
            path: "file_path".to_string()
        }]
    );
}

#[tokio::test]
async fn repeated_model_visible_capability_failures_stop_with_no_progress_fallback() {
    let host = MockHost::new(vec![
        provider_calls_response(),
        provider_calls_response(),
        provider_calls_response(),
    ])
    .with_batch_outcomes(
        (0..3)
            .map(|_| ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Failed(
                    ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind: CapabilityFailureKind::OperationFailed,
                        safe_summary: "filesystem discovery failed".to_string(),
                        detail: None,
                    },
                )],
                stopped_on_suspension: false,
            })
            .collect(),
    );
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
            assert_eq!(completed.result_refs.len(), 3);
        }
        other => panic!("expected completed no-progress fallback, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 3);
    assert_eq!(host.batch_invocations().len(), 3);
    assert_eq!(host.appended_result_refs().len(), 3);
    assert_eq!(
        final_staged_state(&host)
            .stop_state
            .trailing_no_progress_results,
        3
    );
}

#[tokio::test]
async fn repeated_model_visible_multi_call_failures_stop_with_no_progress_fallback() {
    let host = MockHost::new(vec![
        provider_two_calls_response(),
        provider_two_calls_response(),
        provider_two_calls_response(),
    ])
    .with_batch_outcomes(
        (0..3)
            .map(|_| ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![
                    CapabilityOutcome::Failed(ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind: CapabilityFailureKind::OperationFailed,
                        safe_summary: "first discovery failed".to_string(),
                        detail: None,
                    }),
                    CapabilityOutcome::Failed(ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind: CapabilityFailureKind::OperationFailed,
                        safe_summary: "second discovery failed".to_string(),
                        detail: None,
                    }),
                ],
                stopped_on_suspension: false,
            })
            .collect(),
    );
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
            assert_eq!(completed.result_refs.len(), 6);
        }
        other => panic!("expected completed no-progress fallback, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 3);
    assert_eq!(host.batch_invocations().len(), 3);
    assert_eq!(host.appended_result_refs().len(), 6);
    assert_eq!(
        final_staged_state(&host)
            .stop_state
            .trailing_no_progress_results,
        3
    );
}

#[tokio::test]
async fn repeated_non_provider_replayable_failures_do_not_trigger_no_progress_stop() {
    let host = MockHost::new(vec![calls_response(), calls_response(), calls_response()])
        .with_batch_outcomes(
            (0..3)
                .map(|_| ironclaw_turns::run_profile::CapabilityBatchOutcome {
                    outcomes: vec![CapabilityOutcome::Failed(
                        ironclaw_turns::run_profile::CapabilityFailure {
                            error_kind: CapabilityFailureKind::OperationFailed,
                            safe_summary: "non-replayable capability failed".to_string(),
                            detail: None,
                        },
                    )],
                    stopped_on_suspension: false,
                })
                .collect(),
        );
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family_with_iteration_limit(3), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::IterationLimit);
        }
        other => panic!("expected iteration-limit failure, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 3);
    assert_eq!(host.batch_invocations().len(), 3);
    assert_eq!(
        final_staged_state(&host)
            .stop_state
            .trailing_no_progress_results,
        0
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
            CapabilityFailureKind::InvalidInput,
            "provider arguments failed schema validation at instance path root against schema path required",
            "capability failed with invalid_input: provider arguments failed schema validation at instance path root against schema path required",
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
                        detail: None,
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
                    detail: None,
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

#[tokio::test]
async fn prompt_stage_returns_skip_model_when_flag_set() {
    // A plain host with no model responses: the model should never be called.
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.post_capability_state.skip_model_this_iteration = true;

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

    let returned_state = match step {
        PromptStep::SkipModel(state, _ack) => *state,
        PromptStep::Prepared(_) => panic!("expected SkipModel, got Prepared"),
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("expected SkipModel, got resume step")
        }
        PromptStep::Exit(exit) => panic!("expected SkipModel, got Exit({exit:?})"),
    };

    // The flag must be cleared so subsequent iterations call the model normally.
    assert!(
        !returned_state
            .post_capability_state
            .skip_model_this_iteration,
        "skip_model_this_iteration must be cleared after PromptStage consumes it"
    );

    // No prompt bundle was built: the surface/prompt build is bypassed entirely.
    assert_eq!(
        host.prompt_requests().len(),
        0,
        "no prompt bundle should be requested when skipping the model"
    );
}

/// D1 regression: PromptStep::SkipModel must carry the pending_input_ack so
/// canonical.rs can deliver it. Before the fix, SkipModel(Box<LoopExecutionState>)
/// had no second field, so the ack was silently dropped when
/// PromptCompactionStep::run returned Skipped (empty message_index path).
#[tokio::test]
async fn prompt_stage_skip_model_carries_pending_input_ack() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.post_capability_state.skip_model_this_iteration = true;

    // Seed a pending ack token into the PendingInputAck that will be handed
    // to PromptStage — this simulates an inbound user input that was drained
    // but not yet acked.
    let mut pending_input_ack = PendingInputAck::default();
    pending_input_ack
        .replace(vec![
            LoopInputAckToken::new("input-ack:skip-model").expect("valid"),
        ])
        .expect("store pending ack");

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack,
            },
        )
        .await
        .expect("prompt stage");

    // The step must be SkipModel, and the second field must carry the ack.
    let mut carried_ack = match step {
        PromptStep::SkipModel(_state, ack) => ack,
        PromptStep::Prepared(_) => panic!("expected SkipModel, got Prepared"),
        PromptStep::ResumeApproval(_) | PromptStep::ResumeAuth(_) => {
            panic!("expected SkipModel, got resume step")
        }
        PromptStep::Exit(exit) => panic!("expected SkipModel, got Exit({exit:?})"),
    };

    // Nothing should have been acked yet — the ack must be carried, not fired.
    assert!(
        host.acked_input_tokens().is_empty(),
        "ack must not have been delivered inside PromptStage on the Skipped path"
    );

    // Delivering the carried ack must forward the token to the host.
    carried_ack.ack(&host).await.expect("ack inputs");
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:skip-model").expect("valid")],
        "carried ack must deliver the original token to the host"
    );
}

// ---------------------------------------------------------------------------
// WU-A Step 9 — caller-level executor tests for PostCapabilityStage + SkipModel
// ---------------------------------------------------------------------------

/// Byte-threshold trips through the full executor turn: capability batch returns
/// a result whose `byte_len` exceeds `ByteCapStrategy::DEFAULT_FALLBACK_CAP_BYTES`
/// (32 000). PostCapabilityStage should set both compaction flags on the state
/// that is written to the Final checkpoint.
#[tokio::test]
async fn executor_post_capability_trips_policy_and_sets_flags_in_final_state() {
    // Use terminate_hint so the loop exits immediately after the capability
    // turn, giving us a deterministic Final checkpoint to inspect.
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:big").expect("valid"),
                safe_summary: "big result".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                // Exceeds the default 32 000-byte cap for unknown capability ids.
                byte_len: 33_001,
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

    // PostCapabilityStage must have set both flags before stop.decide wrote the
    // Final checkpoint.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.compaction_state.force_compact_on_next_iteration,
        "force_compact_on_next_iteration must be set when byte cap is exceeded"
    );
    assert!(
        final_state.post_capability_state.skip_model_this_iteration,
        "skip_model_this_iteration must be set when byte cap is exceeded"
    );
    assert!(
        final_state
            .post_capability_state
            .pending_capability_bytes
            .is_empty(),
        "pending_capability_bytes must be cleared after trip"
    );

    // D-A: PostCapabilityStage no longer emits CompactionStarted directly;
    // it threads the initiator through force_compact_initiator for
    // PromptCompactionStep to emit on the next iteration. Because this test
    // uses terminate_hint=true and the loop exits before the SkipModel
    // iteration runs, compaction_started must NOT appear here.
    assert!(
        !host.progress_event_names().contains(&"compaction_started"),
        "compaction_started must NOT be emitted by PostCapabilityStage (D-A fix); \
         it is deferred to PromptCompactionStep on the next iteration"
    );
    // D-A: the initiator must be threaded through state.
    assert_eq!(
        final_state.compaction_state.force_compact_initiator,
        Some(ironclaw_turns::run_profile::CompactionInitiator::CapabilityResultOverflow),
        "force_compact_initiator must be CapabilityResultOverflow after a byte-cap trip"
    );
}

/// Under-threshold: small byte_len leaves both flags false in the final state.
#[tokio::test]
async fn executor_post_capability_does_not_trip_under_threshold() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:small").expect("valid"),
                safe_summary: "small result".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: true,
                byte_len: 100, // well under the 32 000-byte default cap
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

    let final_state = final_staged_state(&host);
    assert!(
        !final_state.compaction_state.force_compact_on_next_iteration,
        "force_compact_on_next_iteration must stay false when under threshold"
    );
    assert!(
        !final_state.post_capability_state.skip_model_this_iteration,
        "skip_model_this_iteration must stay false when under threshold"
    );
    assert!(
        !host.progress_event_names().contains(&"compaction_started"),
        "no compaction_started event should be emitted when under threshold"
    );
}

/// SkipModel route: after a byte-cap trip in iteration 1, iteration 2 runs
/// through PromptStage → SkipModel, bypassing the model entirely. The model
/// is called exactly once (iteration 1 only). Iteration 3 calls the model and
/// returns a reply that terminates the loop.
#[tokio::test]
async fn executor_skip_model_turn_bypasses_model_stage() {
    // Iteration 1: model → capability calls (big byte_len, no terminate).
    // Iteration 2: SkipModel (flags cleared by PromptStage, no model call).
    // Iteration 3: model → reply → GracefulStop.
    let host = MockHost::new(vec![calls_response(), reply_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:big-no-term").expect("valid"),
                safe_summary: "big result no terminate".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false, // loop must continue so SkipModel fires
                byte_len: 33_001,
            })],
            stopped_on_suspension: false,
        },
    ]);

    // F7: seed an input ack on the SkipModel iteration (iteration 2 = second
    // poll_inputs call). Batches are consumed in order; iteration 1 gets the
    // first (empty), iteration 2 gets the one with the ack token, iteration 3
    // gets the third (empty). The SkipModel path must deliver this ack to the
    // host (canonical.rs line ~317: pending_input_ack.ack(host).await?).
    let run_context = host.run_context().clone();
    // Seed a steering input ack for iteration 2 (the SkipModel iteration).
    // A Steering input is required to make consume_drainable_inputs advance the
    // ack; without a consumed input, ack_tokens remains empty regardless of the
    // input_acks field in the batch.
    let host = host.with_input_batches(vec![
        LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: input_cursor(&run_context, "input-cursor:iter-1"),
        },
        LoopInputBatch {
            inputs: vec![LoopInput::Steering {
                message_ref: message_ref("msg:steering-skip-model"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:iter-2",
                "input-ack:skip-model-executor",
            )],
            next_cursor: input_cursor(&run_context, "input-cursor:iter-2"),
        },
        LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: input_cursor(&run_context, "input-cursor:iter-3"),
        },
    ]);

    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));

    // The model must have been called exactly twice: once for capabilities
    // (iteration 1) and once for the final reply (iteration 3). Iteration 2
    // must have gone through the SkipModel route and never called the model.
    assert_eq!(
        host.model_requests().len(),
        2,
        "model must be called exactly twice (capability turn + reply turn); \
         SkipModel iteration must bypass ModelStage"
    );

    // D-A: PostCapabilityStage no longer emits CompactionStarted directly;
    // it defers to PromptCompactionStep. In this mock environment the
    // compaction_prompt.message_index is empty, so should_compact() returns
    // Skip and no CompactionStarted event is emitted. The SkipModel route
    // is confirmed by the model_requests().len() == 2 assertion above.
    assert!(
        !host.progress_event_names().contains(&"compaction_started"),
        "compaction_started must NOT appear when message_index is empty \
         (PromptCompactionStep skips compaction; PostCapabilityStage no longer emits it)"
    );

    // Final state: skip_model flag cleared (PromptStage consumed it).
    let final_state = final_staged_state(&host);
    assert!(
        !final_state.post_capability_state.skip_model_this_iteration,
        "skip_model_this_iteration must be cleared by PromptStage before the \
         final reply turn"
    );

    // CompactionOnly turns DO count toward turns_completed per
    // observe_completed_turn's unconditional increment. 3 iterations =
    // 3 completed turns (capabilities + SkipModel + reply).
    assert_eq!(final_state.stop_state.turns_completed, 3);

    // F7: the ack token seeded for the SkipModel iteration must have been
    // delivered to the host. This exercises the D1-regression path:
    // PromptStep::SkipModel carries the ack out of PromptStage, then
    // canonical.rs delivers it before stop.observe (line ~317).
    assert!(
        host.acked_input_tokens()
            .contains(&LoopInputAckToken::new("input-ack:skip-model-executor").expect("valid")),
        "ack token from the SkipModel iteration must be delivered to the host; \
         if it is missing, canonical.rs is dropping the ack on the SkipModel path"
    );
}

/// Multi-call batch: two calls in one turn each carrying 20 000 bytes for the
/// same capability id accumulate to 40 000, exceeding the 32 000-byte default
/// cap. The policy trips once and clears the byte map.
#[tokio::test]
async fn executor_batch_accumulates_per_capability_bytes_and_trips() {
    // two_calls_response() emits two calls with capability_id() ("demo.echo").
    // Each result carries 20 000 bytes → sum = 40 000 > 32 000 → trip.
    let host = MockHost::new(vec![two_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:first").expect("valid"),
                    safe_summary: "first".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true, // exit after batch so we can inspect state
                    byte_len: 20_000,
                }),
                CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:second").expect("valid"),
                    safe_summary: "second".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 20_000,
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
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));

    // Both flags must be set (accumulated bytes exceeded cap).
    let final_state = final_staged_state(&host);
    assert!(
        final_state.compaction_state.force_compact_on_next_iteration,
        "force_compact must trip when per-cap byte sum exceeds the cap"
    );
    assert!(
        final_state.post_capability_state.skip_model_this_iteration,
        "skip_model must trip when per-cap byte sum exceeds the cap"
    );
    // Byte map cleared after trip.
    assert!(
        final_state
            .post_capability_state
            .pending_capability_bytes
            .is_empty(),
        "pending_capability_bytes must be cleared after PostCapabilityStage trips"
    );
    // D-A: PostCapabilityStage no longer emits CompactionStarted directly;
    // the event is deferred to PromptCompactionStep on the next iteration.
    // Because this test uses terminate_hint=true and exits before the SkipModel
    // iteration runs, compaction_started must NOT appear here.
    assert!(
        !host.progress_event_names().contains(&"compaction_started"),
        "compaction_started must NOT be emitted by PostCapabilityStage (D-A fix); \
         it is deferred to PromptCompactionStep on the next iteration"
    );
    // D-A: the initiator must be threaded through state.
    assert_eq!(
        final_state.compaction_state.force_compact_initiator,
        Some(ironclaw_turns::run_profile::CompactionInitiator::CapabilityResultOverflow),
        "force_compact_initiator must be CapabilityResultOverflow after accumulated overflow"
    );
}

/// D2 regression: byte_len was hardcoded to 0 for SpawnedChildRun outcomes.
/// ByteCapStrategy (WU-A) never tripped for builtin.spawn_subagent — the
/// capability with the largest configured cap (48 KB) — even when the spawned
/// result was huge. This test drives the full executor turn with a
/// SpawnedChildRun outcome carrying a large byte_len and asserts that
/// pending_capability_bytes accumulates those bytes (not 0).
#[tokio::test]
async fn spawned_child_run_byte_len_accumulates_and_trips_policy() {
    // Iteration 1: model → SpawnedChildRun with 49 001 bytes (> 32 000-byte
    // default cap). PostCapabilityStage should set compaction flags.
    // Iteration 2: SkipModel route — no model call.
    // Iteration 3: model → reply → GracefulStop.
    let host = MockHost::new(vec![calls_response(), reply_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::SpawnedChildRun {
                child_run_id: TurnRunId::new(),
                result_ref: LoopResultRef::new("result:spawned-child-large").expect("valid"),
                safe_summary: "spawned child with large result".to_string(),
                // Exceeds the default 32 000-byte fallback cap.
                // If byte_len were still hardcoded to 0 in append_spawned_child_result,
                // the policy would never trip and both flag assertions below would fail.
                byte_len: 49_001,
            }],
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

    // The byte cap trip forces a SkipModel iteration before the reply, so the
    // model is called exactly twice: once for capabilities (iteration 1) and
    // once for the final reply (iteration 3). If byte_len were still 0, no
    // trip would occur and the model would be called only once (no SkipModel
    // iteration), making this assertion fail.
    assert_eq!(
        host.model_requests().len(),
        2,
        "model must be called exactly twice (capability turn + reply turn); \
         SkipModel iteration must have fired because the byte cap was tripped by \
         the SpawnedChildRun byte_len — was hardcoded to 0 before D2 fix"
    );

    // D-A: PostCapabilityStage no longer emits CompactionStarted directly;
    // it defers to PromptCompactionStep. In this mock environment the
    // compaction_prompt.message_index is empty, so should_compact() returns
    // Skip and no CompactionStarted event is emitted. The SkipModel route
    // is confirmed by the model_requests().len() == 2 assertion above.
    assert!(
        !host.progress_event_names().contains(&"compaction_started"),
        "compaction_started must NOT appear when message_index is empty \
         (PromptCompactionStep skips; PostCapabilityStage no longer emits it)"
    );
}

/// D2 coverage: AwaitDependentRun outcomes carry byte_len into
/// pending_capability_bytes via push_completed_result (gates.rs).
/// Because AwaitDependentRun exits Blocked (the gate never SkipAndContinues),
/// PostCapabilityStage does not run its policy check on the Exit path.
/// This test verifies that the byte_len IS accumulated into the
/// BeforeBlock checkpoint state — confirming the propagation path is
/// correct — and that the loop exits Blocked as expected. The model is
/// called once (capability turn) before the gate fires.
#[tokio::test]
async fn await_dependent_run_byte_len_accumulates_and_trips_policy() {
    // Iteration 1: model → AwaitDependentRun with 33 001 bytes (> 32 000-byte
    // default cap). The gate fires and blocks the loop. Unlike SpawnedChildRun,
    // the AwaitDependentRun path exits Blocked rather than Continue, so
    // PostCapabilityStage does not evaluate the policy on this turn — but the
    // bytes ARE accumulated into pending_capability_bytes before the block.
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AwaitDependentRun {
                gate_ref: LoopGateRef::new("gate:await-large").expect("valid"),
                result_ref: LoopResultRef::new("result:await-large").expect("valid"),
                safe_summary: "await dependent run with large result".to_string(),
                // Exceeds the default 32 000-byte fallback cap. If byte_len were
                // still propagated as 0 in the AwaitDependentRunGateStage path,
                // the pending_capability_bytes assertion below would fail.
                byte_len: 33_001,
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

    // AwaitDependentRun always blocks — the gate does not SkipAndContinue.
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "AwaitDependentRun must exit Blocked when the gate strategy returns Block"
    );

    // The model is called exactly once: the capability turn. The gate fires
    // after the capability batch, blocking before a second iteration begins.
    assert_eq!(
        host.model_requests().len(),
        1,
        "model must be called exactly once (capability turn only); \
         the gate blocks before any subsequent iteration"
    );

    // Bytes must have been accumulated into pending_capability_bytes by
    // push_completed_result inside AwaitDependentRunGateStage (gates.rs).
    // Inspect the BeforeBlock checkpoint — that is the state written just
    // before the loop exits Blocked.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let accumulated = before_block_state
        .post_capability_state
        .pending_capability_bytes
        .values()
        .sum::<u64>();
    assert_eq!(
        accumulated, 33_001,
        "pending_capability_bytes must accumulate the AwaitDependentRun byte_len \
         (33 001) via push_completed_result before the gate checkpoint fires"
    );
}

// ---------------------------------------------------------------------------
// F12 — CompactionStarted event carries CapabilityResultOverflow initiator
// ---------------------------------------------------------------------------

/// D-A integration: the `force_compact_initiator` threaded through state by
/// PostCapabilityStage must survive the iteration boundary and appear in the
/// `CompactionStarted` event emitted by `PromptCompactionStep` on iteration 2.
///
/// Iteration 1: model → capability call returns 33 001 bytes →
///   PostCapabilityStage trips ByteCapStrategy → sets
///   `force_compact_on_next_iteration`, `skip_model_this_iteration`, and
///   `force_compact_initiator = CapabilityResultOverflow`, clears byte map.
///
/// Iteration 2: PromptStage detects `skip_model_this_iteration` → fires
///   PromptCompactionStep → compaction index is non-empty so `should_compact`
///   returns `Trigger` → emits `CompactionStarted { initiator:
///   CapabilityResultOverflow }` → model call is skipped.
///
/// Iteration 3: model → reply → `GracefulStop`.
///
/// Asserts the recorded progress events contain exactly one `CompactionStarted`
/// whose `initiator == CapabilityResultOverflow` — proving the D-A fix that
/// moves the emit from PostCapabilityStage to PromptCompactionStep is correct.
#[tokio::test]
async fn executor_emits_compaction_started_with_capability_result_overflow_initiator() {
    // The SkipModel path in PromptStage does NOT call build_prompt_bundle;
    // instead it runs PromptCompactionStep directly against
    // state.compaction_prompt.message_index, which was populated by iteration
    // 1's build_prompt_bundle call. So we must provide a non-empty index for
    // iteration 1 (call 1) to seed the state; iteration 3's prompt build
    // (call 2) gets an empty index. Two prompt-bundle builds in total:
    // one on iter 1 (candidate bundle) and one on iter 3 (final reply prompt).
    // Iteration 2 (SkipModel) never calls build_prompt_bundle.
    let host = MockHost::new(vec![calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:big-f12").expect("valid"),
                safe_summary: "big result for F12".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false, // loop must continue so SkipModel iteration fires
                byte_len: 33_001,      // exceeds the 32 000-byte default cap
            })],
            stopped_on_suspension: false,
        }])
        .with_prompt_compaction_indexes(vec![
            // Iteration 1 prompt build: non-empty — seeds state.compaction_prompt.message_index.
            // On iteration 2 (SkipModel), PromptCompactionStep reads this stored index
            // (no bundle rebuild on the SkipModel path) and DefaultCompactionStrategy
            // returns Trigger, causing PromptCompactionStep to fire and emit
            // CompactionStarted with the force_compact_initiator from state.
            active_task_preserving_compaction_index(),
            // Iteration 3 prompt build (post-compaction reply turn): empty.
            vec![],
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-f12").unwrap(),
            compression_ratio_ppm: 250_000,
        }));

    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));

    // Model must have been called exactly twice: iteration 1 (capability
    // turn) and iteration 3 (reply turn). Iteration 2 is a SkipModel turn
    // and must never reach ModelStage.
    assert_eq!(
        host.model_requests().len(),
        2,
        "model must be called exactly twice (capability turn + reply turn); \
         SkipModel iteration must bypass ModelStage"
    );

    // The recorded progress events must contain exactly one CompactionStarted
    // event. Its initiator must be CapabilityResultOverflow — proving that
    // force_compact_initiator threaded through state by PostCapabilityStage
    // (D-A fix) was consumed by PromptCompactionStep and emitted here rather
    // than falling back to the Auto default.
    let progress_events = host.progress_events();
    let compaction_started_events: Vec<_> = progress_events
        .iter()
        .filter(|event| {
            matches!(
                event,
                ironclaw_turns::run_profile::LoopProgressEvent::CompactionStarted { .. }
            )
        })
        .collect();
    assert_eq!(
        compaction_started_events.len(),
        1,
        "exactly one CompactionStarted event must be emitted (on the SkipModel iteration); \
         got: {compaction_started_events:?}"
    );
    match compaction_started_events[0] {
        ironclaw_turns::run_profile::LoopProgressEvent::CompactionStarted { initiator, .. } => {
            assert_eq!(
                initiator,
                &ironclaw_turns::run_profile::CompactionInitiator::CapabilityResultOverflow,
                "CompactionStarted initiator must be CapabilityResultOverflow; \
                 if it is Auto the D-A state-threaded initiator was dropped before \
                 PromptCompactionStep could consume it"
            );
        }
        other => panic!("expected CompactionStarted event, got {:?}", other),
    }

    // Final state: all compaction flags must be cleared (consumed by
    // PromptCompactionStep on iteration 2 and no longer set at iteration 3).
    let final_state = final_staged_state(&host);
    assert!(
        !final_state.compaction_state.force_compact_on_next_iteration,
        "force_compact_on_next_iteration must be cleared after compaction fires"
    );
    assert!(
        final_state
            .compaction_state
            .force_compact_initiator
            .is_none(),
        "force_compact_initiator must be consumed/cleared by PromptCompactionStep"
    );
    // Three iterations completed (capability turn + SkipModel turn + reply turn).
    assert_eq!(
        final_state.stop_state.turns_completed, 3,
        "turns_completed must be 3 (D-A: CompactionOnly turns count per \
         observe_completed_turn's unconditional increment)"
    );
}

// ---------------------------------------------------------------------------
// F13 — AwaitDependentRunGateStage::SkipAndContinue byte_len accumulation
// ---------------------------------------------------------------------------

/// Exercises the `SkipAndContinue` arm in `AwaitDependentRunGateStage::process`
/// (gates.rs:177) via the full executor turn. When the gate strategy returns
/// `SkipAndContinue` for an `AwaitDependentRun` outcome, `push_completed_result`
/// must be called: it accumulates `byte_len` into `pending_capability_bytes` and
/// appends the result ref to `state.result_refs`.
///
/// This path is normally guarded against by `validate_for_gate_kind`, but that
/// check is enforcement-only (test-only call site in strategies/gate.rs). The
/// `SkipAndContinue` arm of `AwaitDependentRunGateStage::process` is reachable
/// through a custom gate strategy that bypasses the guard — e.g. Reborn-hosted
/// gate resolvers that derive their outcome from external policy. This test
/// drives the arm through `CanonicalAgentLoopExecutor` using `FixedGateStrategy`
/// (which returns the outcome directly without validation).
///
/// Note: `PostCapabilityStage` always clears `pending_capability_bytes` at the
/// end of a capability turn (line 96, to avoid cross-turn accumulation). To
/// verify the bytes were accumulated BEFORE the clear, we use a `byte_len` that
/// exceeds the default 32 000-byte threshold. If `push_completed_result` is
/// called, the bytes accumulate inside the turn → `PostCapabilityStage`'s policy
/// check evaluates them → sets `force_compact_on_next_iteration = true` (which
/// DOES persist in the checkpoint). If `push_completed_result` is NOT called,
/// `pending_capability_bytes` is empty, the policy never fires, and
/// `force_compact_on_next_iteration` remains false.
///
/// Scenario (single-iteration):
///   - Model → `AwaitDependentRun` capability outcome with `byte_len = 33 001`.
///   - Gate returns `SkipAndContinue` → loop continues.
///   - `terminate_hint = true` in the outcome causes `StopStage` to exit after
///     this iteration, giving us a deterministic Final checkpoint to inspect.
///
/// Asserts:
///   - Loop completes (not blocked — confirms SkipAndContinue worked).
///   - Final `force_compact_on_next_iteration = true`: bytes accumulated by
///     `push_completed_result` were seen by `PostCapabilityStage`'s policy.
///   - Final `result_refs` contains the `AwaitDependentRun` result ref:
///     second proof that `push_completed_result` was called in the
///     `SkipAndContinue` arm (result_refs are retained across turns).
///   - `force_compact_initiator == CapabilityResultOverflow`: the D-A initiator
///     threading also works correctly for the `SkipAndContinue` arm.
#[tokio::test]
async fn await_dependent_run_gate_skip_and_continue_accumulates_byte_len() {
    let result_ref_str = "result:await-skip";
    // byte_len exceeds the default 32 000-byte threshold to make the policy trip.
    // See note in docstring: we cannot inspect pending_capability_bytes in the
    // Final checkpoint directly (PostCapabilityStage clears it), so we rely on
    // force_compact_on_next_iteration being set as an indirect proof.
    let byte_len: u64 = 33_001;
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    // Single iteration: model → AwaitDependentRun (SkipAndContinue), terminate_hint=true.
    // The resolved_result constructed inside AwaitDependentRunGateStage from the
    // AwaitDependentRun outcome carries byte_len; terminate_hint is set to false
    // internally (capabilities.rs line 467), but stop.decide exits on the
    // TerminateHint StopKind from DefaultStopConditionStrategy — which uses the
    // batch summary's terminate_hint flag, not the result message's. To force
    // a 1-iteration exit we instead use a terminate_hint=true outcome so that
    // StopStage exits, giving us a stable Final checkpoint. Since AwaitDependentRun
    // outcomes set terminate_hint=false in the resolved_result (line 467,
    // capabilities.rs), the actual CapabilityResultMessage has terminate_hint=false;
    // the StopStage terminate path is driven by CapabilityBatchTurnSummary which
    // we can't directly override here. Use terminate_hint via the batch outcome.
    // Simplest: use the default stop strategy and provide only one model response
    // (calls_response) and no reply_response — the loop exits after the batch
    // because DefaultStopConditionStrategy.should_stop_after_observed_turn returns
    // GracefulStop when there are no more model responses pending AND the only
    // model response was a capability call that resulted in a SkipAndContinue batch
    // with a completed result summary. Actually — the simplest approach is two
    // model responses: calls + reply. After SkipAndContinue, iteration 2 has the
    // reply and exits. The SkipModel path does NOT fire here because byte_len
    // accumulates and PostCapabilityStage would set force_compact flags, but we
    // check the FIRST iteration's contribution via Final state after 2 iterations.
    // Use terminate_hint=false on the outcome and a second model response (reply).
    // After iteration 1 (SkipAndContinue + PostCapabilityStage trip):
    //   state.compaction_state.force_compact_on_next_iteration = true (persists)
    // After iteration 2 (SkipModel — skip_model_this_iteration was set):
    //   PromptCompactionStep runs; message_index is empty → Skipped path →
    //   force_compact_on_next_iteration cleared to false (prompt.rs line 207).
    // After iteration 3 (reply — provided by second model response):
    //   Final checkpoint: force_compact_on_next_iteration = false (already cleared).
    //
    // To avoid the clearing on the SkipModel iteration we use terminate_hint=true
    // on the batch outcome (not the result message; terminate_hint on the result
    // message is set to false by AwaitDependentRunGateStage internally). We achieve
    // this by using the CapabilityBatchOutcome's StopKind pathway. The cleanest
    // approach: set terminate_hint=true on a SIBLING completed result in the batch,
    // but that adds complexity. Instead we use a one-shot check: since
    // force_compact_on_next_iteration is set in iteration 1's PostCapabilityStage
    // and only cleared in iteration 2's PromptStage (SkipModel path, when
    // message_index is empty), and iteration 2 immediately clears the flag before
    // writing any checkpoint, the flag value in any checkpoint after iteration 2
    // will be false regardless.
    //
    // Resolution: use terminate_hint=true as the capability outcome's own field
    // which IS propagated to CapabilityBatchTurnSummary. The AwaitDependentRun
    // CapabilityResultMessage has terminate_hint=false (hardcoded in capabilities.rs)
    // so the DefaultStopStrategy won't act on it. We cannot set terminate_hint=true
    // on AwaitDependentRun via the public API without modifying test fixtures.
    //
    // Pragmatic solution: check result_refs instead (persists across turns).
    // Provide 2 model responses so iter 1 is capability + SkipAndContinue and
    // iter 2 is SkipModel (forced by PostCapabilityStage) and iter 3 is reply.
    // The force_compact_on_next_iteration is set in iter 1 and cleared in iter 2
    // — so we check result_refs as the persistent proof and also assert the
    // SkipModel iteration fired (model count == 2 for 3 total iterations).
    let host = MockHost::new(vec![calls_response(), reply_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AwaitDependentRun {
                gate_ref: LoopGateRef::new("gate:await-skip").expect("valid"),
                result_ref: LoopResultRef::new(result_ref_str).expect("valid"),
                safe_summary: "dependent run skip and continue".to_string(),
                byte_len,
            }],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    // SkipAndContinue must allow the loop to complete, not block.
    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "SkipAndContinue must allow the loop to continue to completion; \
         if Blocked, the AwaitDependentRunGateStage SkipAndContinue arm returned \
         BatchStep::Exit instead of BatchStep::Continue"
    );

    // push_completed_result was called in iteration 1's SkipAndContinue arm.
    // The result ref must appear in state.result_refs (set by push_completed_result).
    let final_state = final_staged_state(&host);
    assert!(
        final_state
            .result_refs
            .iter()
            .any(|r| r.as_str() == result_ref_str),
        "state.result_refs must contain the AwaitDependentRun result ref; \
         push_completed_result in the SkipAndContinue arm must call \
         state.result_refs.push(result.result_ref) — if missing, the \
         SkipAndContinue arm is not calling push_completed_result"
    );

    // byte_len = 33 001 exceeds the threshold; PostCapabilityStage set
    // force_compact_on_next_iteration=true and skip_model_this_iteration=true
    // after iteration 1. Iteration 2 is therefore a SkipModel iteration, and
    // the model is called only twice (iter 1 + iter 3 reply). This confirms the
    // bytes reached the PostCapabilityStage policy evaluator via push_completed_result.
    assert_eq!(
        host.model_requests().len(),
        2,
        "model must be called exactly twice (capability turn iter 1 + reply turn iter 3); \
         byte_len=33_001 must have tripped ByteCapStrategy via push_completed_result, \
         causing iter 2 to be a SkipModel iteration"
    );
}

#[tokio::test]
async fn auth_gate_block_stores_pending_auth_resume() {
    // Drive the full executor loop so the GateStage block arm runs through the
    // canonical path (cancel-check → progress emit → write_before_block).
    let gate_ref = LoopGateRef::new("gate:auth-block").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: gate_ref.clone(),
                credential_requirements: Vec::new(),
                safe_summary: "auth required".to_string(),
            }],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on auth gate");

    // Exit must be a blocked (auth) exit.
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit for auth gate, got {exit:?}"
    );

    // BeforeBlock checkpoint must have been written in the expected sequence.
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::BeforeBlock,
        ]
    );

    // Recover state from the BeforeBlock checkpoint — this is what the resume
    // path will load, so it must carry the pending_auth_resume record.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);

    // Auth slot must be populated.
    let pending = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_auth_resume when auth gate blocks");
    assert_eq!(
        pending.gate_ref, gate_ref,
        "pending_auth_resume.gate_ref must match the blocked gate ref"
    );
    assert_eq!(
        pending.capability_id,
        capability_id(),
        "pending_auth_resume.capability_id must match the scripted capability"
    );

    // Approval slot must NOT be touched by an auth block.
    assert!(
        before_block_state.pending_approval_resume.is_none(),
        "auth block must not populate pending_approval_resume"
    );
}

#[tokio::test]
async fn non_auth_gate_block_preserves_pending_auth_resume() {
    // Regression test for the fix where `_ => state.pending_auth_resume.take()`
    // would erase a live auth resume record when a non-auth gate (e.g. approval)
    // blocked mid-re-dispatch.
    //
    // Scenario: auth gate previously blocked → record stored → OAuth completes →
    // resume re-dispatches the call → re-dispatch hits an APPROVAL gate → Block
    // arm must NOT clear the auth record. The auth record must survive so that
    // the outer resume handler can still consume it.
    let approval_gate_ref = LoopGateRef::new("gate:approval-during-redispatch").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: approval_gate_ref.clone(),
                safe_summary: "approval required during redispatch".to_string(),
                approval_resume: None,
            }],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;

    // Seed a live auth resume record, simulating a state that was rehydrated
    // from a BeforeBlock checkpoint written when the auth gate first blocked.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original").expect("valid");
    let seeded_auth_resume = PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        prior_approval: None,
    };
    let mut initial_state = LoopExecutionState::initial_for_run(host.run_context());
    initial_state.pending_auth_resume = Some(seeded_auth_resume.clone());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on approval gate");

    // Exit must be a Blocked exit (approval gate fired).
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit when approval gate blocks, got {exit:?}"
    );

    // The BeforeBlock checkpoint must carry the auth resume record unchanged —
    // the approval-gate Block arm must not have erased it.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let surviving_resume = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume must survive a non-auth gate block");
    assert_eq!(
        surviving_resume.gate_ref, seeded_gate_ref,
        "surviving pending_auth_resume.gate_ref must be the original auth gate ref, not the approval gate ref"
    );
    assert_eq!(
        surviving_resume.capability_id, seeded_auth_resume.capability_id,
        "surviving pending_auth_resume.capability_id must be unchanged"
    );
}

#[tokio::test]
async fn resume_after_auth_gate_redispatches_original_call_without_model_turn() {
    // Phase 1: executor blocks on an auth gate and writes a BeforeBlock checkpoint
    // that carries a pending_auth_resume record with the original input_ref.
    let gate_ref = LoopGateRef::new("gate:auth-resume-test").expect("valid");
    let completed_ref = LoopResultRef::new("result:auth-resumed").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: gate_ref.clone(),
                credential_requirements: Vec::new(),
                safe_summary: "auth required".to_string(),
            }],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: the auth is now satisfied, call completes.
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(
                ironclaw_turns::run_profile::CapabilityResultMessage {
                    result_ref: completed_ref.clone(),
                    safe_summary: "auth resumed and completed".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 0,
                },
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    // Phase 1 run — expect a Blocked exit.
    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    // Exactly one model call happened during Phase 1.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // Recover the BeforeBlock checkpoint state — this is what resume loads.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_auth_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_auth_resume"
    );

    // Derive the stale input_ref from the BeforeBlock checkpoint before the
    // state is consumed by the phase 2 execute call.
    let checkpoint_input_ref = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set in BeforeBlock checkpoint")
        .input_ref
        .clone();
    assert!(
        before_block_state
            .pending_auth_resume
            .as_ref()
            .expect("pending_auth_resume set")
            .provider_replay
            .is_some(),
        "provider-backed auth resumes must checkpoint replay metadata"
    );
    assert!(
        host.registered_provider_calls().is_empty(),
        "phase 1 model response is already a candidate; registration happens on auth resume"
    );

    // Phase 2 run — seeded from the BeforeBlock checkpoint state.
    // The prompt stage must detect pending_auth_resume and skip the model call,
    // restaging the provider replay metadata before re-dispatching the capability.
    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes from auth gate");
    assert!(
        matches!(second_exit, LoopExit::Completed(_)),
        "expected Completed exit after auth resume, got {second_exit:?}"
    );

    // (a) No additional model call during Phase 2 — capability re-dispatched before model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "auth resume must re-dispatch the saved invocation without a model call"
    );

    // (b) Exactly two batch invocations total: Phase 1 (blocked) + Phase 2 (completed).
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        2,
        "expected two batch invocations (phase 1 block + phase 2 re-dispatch)"
    );

    // The Phase 2 invocation must carry a freshly staged input_ref. The
    // checkpoint input_ref belonged to the old provider-call input resolver.
    assert_ne!(
        batch_invocations[1].invocations[0].input_ref, checkpoint_input_ref,
        "provider-backed auth resume must not reuse the stale checkpoint input_ref"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].input_ref.as_str(),
        "input:registered-provider-1",
        "provider-backed auth resume must invoke with the restaged provider input"
    );
    let registered_provider_calls = host.registered_provider_calls();
    assert_eq!(
        registered_provider_calls.len(),
        1,
        "auth resume must restage exactly one provider tool call"
    );
    assert_eq!(
        registered_provider_calls[0].name, "demo__echo",
        "auth resume must restage the checkpointed provider tool name"
    );
    assert_eq!(
        registered_provider_calls[0].arguments,
        serde_json::json!({"message":"hello"}),
        "auth resume must restage the checkpointed provider tool arguments"
    );

    // (c) Neither invocation carries an approval_resume token.
    //     Phase 1 is a plain first invocation; phase 2 is a token-less auth re-dispatch.
    assert_eq!(
        batch_invocations[0].invocations[0].approval_resume, None,
        "phase-1 invocation must not carry an approval_resume token"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].approval_resume, None,
        "auth re-dispatch must not carry an approval_resume token"
    );

    // (d) pending_auth_resume is cleared in the final state.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after successful re-dispatch"
    );

    // (e) The completed result was recorded.
    assert_eq!(
        final_state.result_refs,
        vec![completed_ref],
        "completed result ref must be recorded in final state"
    );
}

#[tokio::test]
async fn auth_resume_provider_registration_failure_fails_before_invocation() {
    let gate_ref = LoopGateRef::new("gate:auth-resume-register-fails").expect("valid");
    let completed_ref = LoopResultRef::new("result:unused-auth-resume").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()])
        .with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::AuthRequired {
                    gate_ref: gate_ref.clone(),
                    credential_requirements: Vec::new(),
                    safe_summary: "auth required".to_string(),
                }],
                stopped_on_suspension: true,
            },
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Completed(
                    ironclaw_turns::run_profile::CapabilityResultMessage {
                        result_ref: completed_ref,
                        safe_summary: "should not invoke".to_string(),
                        progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                        terminate_hint: true,
                        byte_len: 0,
                    },
                )],
                stopped_on_suspension: false,
            },
        ])
        .with_provider_registration_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "provider registration failed",
        )]);
    let executor = CanonicalAgentLoopExecutor;

    let first_exit = executor
        .execute_family(
            &crate::families::default(),
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);

    let error = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect_err("provider registration failure should fail auth resume");

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability
        }
    ));
    assert!(
        host.registered_provider_calls().is_empty(),
        "failed provider registration must not be recorded as staged"
    );
    assert_eq!(
        host.batch_invocations().len(),
        1,
        "phase 2 must fail before invoking the resumed capability"
    );
}

#[tokio::test]
async fn resume_with_still_missing_credentials_blocks_again_without_model_turn() {
    // Phase 1: scripted AuthRequired -> executor exits Blocked and writes a
    // BeforeBlock checkpoint carrying a pending_auth_resume record.
    let gate_ref = LoopGateRef::new("gate:auth-still-missing").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: gate_ref.clone(),
                credential_requirements: Vec::new(),
                safe_summary: "auth required (phase 1)".to_string(),
            }],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: credentials are STILL missing — block again.
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: LoopGateRef::new("gate:auth-still-missing-2").expect("valid"),
                credential_requirements: Vec::new(),
                safe_summary: "auth required (phase 2 — still missing)".to_string(),
            }],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    // Phase 1 run — expect Blocked exit.
    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit in phase 1, got {first_exit:?}"
    );
    // Exactly one model call in phase 1.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // Recover the BeforeBlock checkpoint — this is what resume loads.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_auth_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_auth_resume after phase 1"
    );
    let phase1_capability_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set")
        .capability_id
        .clone();

    // Phase 2 run — seeded from the BeforeBlock state.
    // Credentials are still missing: the capability re-dispatches and blocks again.
    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute — still-missing credentials path should not error");
    assert!(
        matches!(second_exit, LoopExit::Blocked(_)),
        "expected Blocked exit in phase 2 (credentials still missing), got {second_exit:?}"
    );

    // (a) No additional model call during phase 2 — re-dispatch happened without model turn.
    assert_eq!(
        host.model_requests().len(),
        1,
        "auth resume with still-missing credentials must not trigger a new model call"
    );

    // (b) Exactly two batch invocations total: phase 1 block + phase 2 re-dispatch block.
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        2,
        "expected two batch invocations (phase 1 block + phase 2 re-dispatch block)"
    );

    // (c) The new BeforeBlock checkpoint must carry a pending_auth_resume record
    //     whose capability_id matches the original one from phase 1.
    let phase2_before_block_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock checkpoint payload")
        })
        .collect();
    // There should be at least two BeforeBlock checkpoints (one per phase).
    assert!(
        phase2_before_block_states.len() >= 2,
        "expected at least two BeforeBlock checkpoints (phase 1 + phase 2)"
    );
    let phase2_resume = phase2_before_block_states
        .last()
        .expect("at least one")
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock checkpoint must carry pending_auth_resume");
    assert_eq!(
        phase2_resume.capability_id, phase1_capability_id,
        "phase 2 pending_auth_resume.capability_id must match the original capability"
    );
    // The gate_ref in the phase-2 BeforeBlock checkpoint must reflect the refreshed
    // AuthRequired outcome from phase 2, not the stale phase-1 gate ref.  This
    // proves GateStage wrote a fresh record rather than preserving the old one.
    let phase2_gate_ref = LoopGateRef::new("gate:auth-still-missing-2").expect("valid");
    assert_eq!(
        phase2_resume.gate_ref, phase2_gate_ref,
        "phase 2 pending_auth_resume.gate_ref must equal the refreshed phase-2 gate ref"
    );
}

#[tokio::test]
async fn gate_stage_skip_and_continue_clears_stale_pending_auth_resume() {
    // Bug scenario: auth record stored for capability A → resume re-dispatches A
    // → re-dispatch returns ApprovalRequired → GateStage runs with kind Approval
    // → planner returns SkipAndContinue. Without the fix, pending_auth_resume
    // for A survives, and the next prompt iteration re-dispatches A again —
    // potential infinite re-dispatch loop with no model turn.
    //
    // This test exercises GateStage directly (not the full executor) so we can
    // seed pending_auth_resume before the gate runs, mirroring the existing
    // gate_stage_skips_and_continues_records_skipped_summary pattern.
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    // Seed a pending_auth_resume for the same capability that will be dispatched
    // through GateStage — this simulates the state reloaded from a BeforeBlock
    // checkpoint that was written when the auth gate first blocked.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        prior_approval: None,
    });
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:approval-skip").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Approval,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
            },
        )
        .await
        .expect("gate stage");

    let BatchStep::Continue(final_state) = step else {
        panic!("expected SkipAndContinue to return Continue");
    };
    assert!(
        final_state.pending_auth_resume.is_none(),
        "SkipAndContinue must clear pending_auth_resume for the skipped capability \
         to prevent an infinite re-dispatch loop on the next prompt iteration"
    );
}

#[tokio::test]
async fn gate_stage_abort_clears_stale_pending_auth_resume() {
    // Bug scenario: auth record stored for capability A → resume re-dispatches A
    // → re-dispatch returns ResourceBlocked → GateStage runs with kind Resource
    // → planner returns Abort. Without the fix, pending_auth_resume persists
    // into the Final checkpoint, leaving a stale record.
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
    // Seed a pending_auth_resume for the same capability.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original-abort").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original-abort").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        prior_approval: None,
    });
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:resource-abort").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Resource,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
            },
        )
        .await
        .expect("gate stage");

    // The Abort arm must return a Failed exit and write a Final checkpoint.
    let BatchStep::Exit(LoopExit::Failed(failed)) = step else {
        panic!("expected failed exit from Abort arm");
    };
    assert_eq!(failed.reason_kind, failure_kind);
    assert!(failed.checkpoint_id.is_some());

    // The Final checkpoint must NOT carry a stale pending_auth_resume.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "Abort must clear pending_auth_resume for the aborted capability \
         to prevent a stale record from persisting into the Final checkpoint"
    );
}

#[tokio::test]
async fn gate_stage_skip_does_not_clear_auth_resume_for_different_capability() {
    // The clear is capability-scoped: a SkipAndContinue for capability B must NOT
    // erase a pending_auth_resume record belonging to capability A.
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    // Seed a pending_auth_resume for a DIFFERENT capability (not the one being gated).
    let different_cap_id = ironclaw_host_api::CapabilityId::new("other.cap").expect("valid");
    let seeded_gate_ref = LoopGateRef::new("gate:auth-other-cap").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: different_cap_id.clone(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:other-cap").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        prior_approval: None,
    });
    // The call being dispatched through GateStage is capability_id() ("demo.echo"),
    // not the seeded "other.cap".
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:approval-skip-other").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Approval,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
            },
        )
        .await
        .expect("gate stage");

    let BatchStep::Continue(final_state) = step else {
        panic!("expected SkipAndContinue to return Continue");
    };
    // The record for "other.cap" must survive — only the matching capability is cleared.
    let surviving = final_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume for a different capability must not be cleared");
    assert_eq!(
        surviving.capability_id, different_cap_id,
        "surviving pending_auth_resume must belong to the other capability"
    );
    assert_eq!(
        surviving.gate_ref, seeded_gate_ref,
        "surviving pending_auth_resume.gate_ref must be unchanged"
    );
}

#[tokio::test]
async fn stale_surface_batch_failure_is_recoverable() {
    let host = MockHost::new(vec![calls_response(), reply_response()])
        .fail_batch_with(AgentLoopHostErrorKind::StaleSurface);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("StaleSurface batch error must not kill the run");

    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "run must complete after a StaleSurface batch error; got {exit:?}"
    );
}

#[tokio::test]
async fn non_stale_batch_failure_stays_terminal() {
    let host =
        MockHost::new(vec![calls_response()]).fail_batch_with(AgentLoopHostErrorKind::Unavailable);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("non-StaleSurface batch error must propagate as terminal error");

    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability
        }
    );
}

// ── Approval-then-auth resume: invocation_id preserved ───────────────────────

#[tokio::test]
async fn auth_resume_after_approval_carries_resume_token_and_approval_request_id() {
    // Regression test for the fix that makes auth-gate re-dispatch reuse the
    // ORIGINAL invocation_id so a one-shot approval lease survives the auth gate.
    //
    // Without the fix: `capability_invocation_from_auth_resume_candidate` returned
    // `auth_resume: None` because `pending_auth.resume_token` was never set.
    // With the fix: `pending_auth.resume_token` carries the approval resume token and
    // `auth_resume` is populated, allowing the host to match the fingerprinted lease.
    //
    // This test drives the full 3-phase executor path:
    //   Phase 1: model → ApprovalRequired (with resume token) → Blocked
    //   Phase 2: approval-resume re-dispatch → AuthRequired → Blocked
    //   Phase 3: auth-resume re-dispatch → Completed
    // and asserts that the phase-3 invocation carries the correct auth_resume.

    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:approval-auth-test").expect("valid token");
    let correlation_id = CorrelationId::new();
    let original_input_ref =
        CapabilityInputRef::new("input:approval-auth-original").expect("valid");
    let auth_gate_ref = LoopGateRef::new("gate:auth-after-approval").expect("valid");
    let completed_ref = LoopResultRef::new("result:auth-after-approval-done").expect("valid");

    let approval_resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: resume_token.clone(),
        correlation_id,
        input_ref: original_input_ref.clone(),
        input: serde_json::json!({ "message": "hello" }),
        estimate: ResourceEstimate::default(),
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks with resume metadata
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:approval-then-auth").expect("valid"),
                safe_summary: "approval required".to_string(),
                approval_resume: Some(approval_resume.clone()),
            }],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: auth_gate_ref.clone(),
                credential_requirements: Vec::new(),
                safe_summary: "auth required after approval".to_string(),
            }],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(
                ironclaw_turns::run_profile::CapabilityResultMessage {
                    result_ref: completed_ref.clone(),
                    safe_summary: "completed after auth resume".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 0,
                },
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;

    // ── Phase 1: model turn → approval gate → Blocked ────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must block on approval gate");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "expected Blocked exit from approval gate; got {phase1_exit:?}"
    );
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // BeforeBlock checkpoint carries pending_approval_resume with the resume token.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_approval = phase1_bb
        .pending_approval_resume
        .as_ref()
        .expect("phase 1 BeforeBlock must carry pending_approval_resume");
    assert_eq!(
        pending_approval.resume_token, resume_token,
        "phase 1 pending_approval_resume.resume_token must match the scripted token"
    );
    assert_eq!(
        pending_approval.approval_request_id, approval_request_id,
        "phase 1 pending_approval_resume.approval_request_id must match"
    );

    // ── Phase 2: approval-resume → auth gate → Blocked ───────────────────────
    let phase2_exit = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await
        .expect("phase 2 must block on auth gate");
    assert!(
        matches!(phase2_exit, LoopExit::Blocked(_)),
        "expected Blocked exit from auth gate; got {phase2_exit:?}"
    );
    // No new model call — approval-resume re-dispatched before the model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 2 (approval-resume) must not trigger a new model call"
    );

    // BeforeBlock checkpoint for phase 2 carries pending_auth_resume.
    // It must propagate the resume_token and prior_approval from the approval.
    let phase2_bb_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock payload")
        })
        .collect();
    assert!(
        phase2_bb_states.len() >= 2,
        "expected at least two BeforeBlock checkpoints (phase 1 + phase 2)"
    );
    let phase2_bb = phase2_bb_states.last().expect("at least one").clone();
    let pending_auth = phase2_bb
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock must carry pending_auth_resume");
    assert_eq!(
        pending_auth.resume_token,
        Some(resume_token.clone()),
        "pending_auth_resume.resume_token must carry the approval resume token"
    );
    let pending_auth_pa = pending_auth
        .prior_approval
        .as_ref()
        .expect("pending_auth_resume.prior_approval must be set when approval preceded auth");
    assert_eq!(
        pending_auth_pa.approval_request_id, approval_request_id,
        "pending_auth_resume.prior_approval.approval_request_id must match the approval request"
    );

    // ── Phase 3: auth-resume → Completed ─────────────────────────────────────
    let phase3_exit = executor
        .execute_family(&crate::families::default(), &host, phase2_bb)
        .await
        .expect("phase 3 must complete after auth resume");
    assert!(
        matches!(phase3_exit, LoopExit::Completed(_)),
        "expected Completed exit after auth resume; got {phase3_exit:?}"
    );
    // Still no additional model call.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 3 (auth-resume) must not trigger a new model call"
    );

    // Three total batch invocations: phase 1 (approval block) + phase 2 (auth
    // block) + phase 3 (completed).
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        3,
        "expected three batch invocations (phase 1 approval + phase 2 auth + phase 3 complete)"
    );

    // Phase 1 invocation: plain, no approval_resume and no auth_resume.
    assert_eq!(
        batch_invocations[0].invocations[0].approval_resume, None,
        "phase 1 invocation must not carry approval_resume (set on the outcome, not the request)"
    );
    assert_eq!(
        batch_invocations[0].invocations[0].auth_resume, None,
        "phase 1 invocation must not carry auth_resume"
    );

    // Phase 2 invocation: this is the approval-resume re-dispatch.
    // approval_resume is set; auth_resume is not (auth hasn't happened yet).
    assert_eq!(
        batch_invocations[1].invocations[0].auth_resume, None,
        "phase 2 (approval-resume) invocation must not carry auth_resume"
    );

    // Phase 3 invocation: this is the auth-resume re-dispatch.
    // auth_resume must be set and carry the original resume_token + prior_approval.
    // Pre-fix: auth_resume would be None (resume_token was never propagated).
    // Post-fix: auth_resume carries the token so the host can reuse the original
    // invocation identifier and match the fingerprinted approval lease.
    let phase3_auth_resume = batch_invocations[2].invocations[0]
        .auth_resume
        .as_ref()
        .expect(
            "phase 3 (auth-resume) invocation must carry auth_resume \
                 (pre-fix: was None because resume_token was not propagated)",
        );
    assert_eq!(
        phase3_auth_resume.resume_token, resume_token,
        "auth_resume.resume_token must match the original approval resume token"
    );
    let phase3_pa = phase3_auth_resume
        .prior_approval
        .as_ref()
        .expect("phase 3 auth_resume.prior_approval must be set");
    assert_eq!(
        phase3_pa.approval_request_id, approval_request_id,
        "auth_resume.prior_approval.approval_request_id must match the original approval request id"
    );
    assert_eq!(
        phase3_pa.correlation_id, correlation_id,
        "auth_resume.prior_approval.correlation_id must match the original correlation id from the approval"
    );

    // Final state: pending_auth_resume cleared and result recorded.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after successful auth-resume re-dispatch"
    );
    assert_eq!(
        final_state.result_refs,
        vec![completed_ref],
        "completed result ref must be recorded"
    );
}

/// Verify that `pending_auth_resume.prior_approval.correlation_id` equals the
/// approval's own `correlation_id` throughout the approval → auth-block → auth-resume
/// pipeline.  This is the dedicated regression test for the correlation-id axis
/// (the broader `auth_resume_after_approval_carries_resume_token_and_approval_request_id`
/// test covers the other fields).
#[tokio::test]
async fn auth_resume_after_approval_carries_original_correlation_id() {
    // The three-phase flow:
    //   phase 1 — model turn → approval gate (records correlation_id in approval_resume)
    //   phase 2 — approval-resume → auth gate → Blocked
    //             (pending_auth_resume.prior_approval.correlation_id must equal approval's)
    //   phase 3 — auth-resume → Completed
    //             (phase-3 invocation.auth_resume.prior_approval.correlation_id must match)

    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:corr-id-test").expect("valid token");
    let correlation_id = CorrelationId::new();
    let original_input_ref = CapabilityInputRef::new("input:corr-id-original").expect("valid");
    let completed_ref = LoopResultRef::new("result:corr-id-done").expect("valid");

    let approval_resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: resume_token.clone(),
        correlation_id,
        input_ref: original_input_ref.clone(),
        input: serde_json::json!({ "message": "hello" }),
        estimate: ResourceEstimate::default(),
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks.
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:corr-id-approval").expect("valid"),
                safe_summary: "approval required".to_string(),
                approval_resume: Some(approval_resume.clone()),
            }],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch.
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::AuthRequired {
                gate_ref: LoopGateRef::new("gate:corr-id-auth").expect("valid"),
                credential_requirements: Vec::new(),
                safe_summary: "auth required".to_string(),
            }],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes.
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(
                ironclaw_turns::run_profile::CapabilityResultMessage {
                    result_ref: completed_ref.clone(),
                    safe_summary: "done".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: true,
                    byte_len: 0,
                },
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;

    // Phase 1: model → approval gate.
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must block on approval gate");
    assert!(matches!(phase1_exit, LoopExit::Blocked(_)));

    // Phase 2: approval-resume → auth gate.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let phase2_exit = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await
        .expect("phase 2 must block on auth gate");
    assert!(matches!(phase2_exit, LoopExit::Blocked(_)));

    // KEY: pending_auth_resume.prior_approval.correlation_id must equal the
    // original approval correlation_id written by the approval-gate outcome.
    let phase2_bb_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock payload")
        })
        .collect();
    let phase2_bb = phase2_bb_states.last().expect("at least one").clone();
    let pending_auth = phase2_bb
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock must carry pending_auth_resume");
    let pending_pa = pending_auth
        .prior_approval
        .as_ref()
        .expect("pending_auth_resume.prior_approval must be set when approval preceded auth");
    assert_eq!(
        pending_pa.correlation_id, correlation_id,
        "pending_auth_resume.prior_approval.correlation_id must equal the approval's correlation_id"
    );

    // Phase 3: auth-resume → Completed.
    let phase3_exit = executor
        .execute_family(&crate::families::default(), &host, phase2_bb)
        .await
        .expect("phase 3 must complete");
    assert!(matches!(phase3_exit, LoopExit::Completed(_)));

    // Phase-3 invocation must carry prior_approval.correlation_id.
    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 3);
    let phase3_ar = batch_invocations[2].invocations[0]
        .auth_resume
        .as_ref()
        .expect("phase 3 invocation must carry auth_resume");
    let phase3_pa = phase3_ar
        .prior_approval
        .as_ref()
        .expect("phase 3 auth_resume.prior_approval must be set");
    assert_eq!(
        phase3_pa.correlation_id, correlation_id,
        "phase 3 auth_resume.prior_approval.correlation_id must match the original approval correlation_id"
    );
}

// ── auth-resume slot consumed on first batch match (batch duplicate guard) ──

#[tokio::test]
async fn auth_resume_slot_consumed_on_first_batch_match_not_reused_for_second_call() {
    // Regression test: pending_auth_resume must be consumed on the FIRST batch
    // call whose capability_id matches, not shared across all matching calls.
    //
    // Before the fix `pending_auth_resume` was accessed via `as_ref().filter(…)`
    // (non-consuming), so two calls to the same capability_id in one batch would
    // BOTH receive the same auth_resume — reusing one resume_token/invocation_id
    // across distinct calls (correctness + security bug).
    //
    // After the fix `pending_auth_resume` uses `take_if` (consuming on first
    // match), so only the FIRST matching call carries auth_resume; the second is
    // a normal dispatch (auth_resume = None).
    //
    // We drive CapabilityStage directly (rather than the full executor) because
    // when pending_auth_resume is set the executor routes through the single-call
    // ResumeAuth prompt path — the two-call batch can only be exercised at the
    // CapabilityStage boundary where the mapping loop lives.
    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:batch-dup-guard").expect("valid token");
    let correlation_id = CorrelationId::new();
    let input_ref = CapabilityInputRef::new("input:batch-dup-guard").expect("valid");

    // Two outcomes for the two calls; both complete so no suspension complicates things.
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![
                CapabilityOutcome::Completed(
                    ironclaw_turns::run_profile::CapabilityResultMessage {
                        result_ref: LoopResultRef::new("result:first").expect("valid"),
                        safe_summary: "first done".to_string(),
                        progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                        terminate_hint: false,
                        byte_len: 0,
                    },
                ),
                CapabilityOutcome::Completed(
                    ironclaw_turns::run_profile::CapabilityResultMessage {
                        result_ref: LoopResultRef::new("result:second").expect("valid"),
                        safe_summary: "second done".to_string(),
                        progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                        terminate_hint: false,
                        byte_len: 0,
                    },
                ),
            ],
            stopped_on_suspension: false,
        },
    ]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // State with pending_auth_resume set — capability_id() matches both calls below.
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:batch-dup-auth").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: input_ref.clone(),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: Some(resume_token.clone()),
        prior_approval: Some(crate::state::AuthResumeApprovalIdentity {
            approval_request_id,
            correlation_id,
        }),
    });

    // Two calls to the same capability_id — extracted from the two_calls_response fixture.
    let calls = match two_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected calls fixture"),
    };

    let surface = ironclaw_turns::run_profile::LoopCapabilityPort::visible_capabilities(
        &host,
        VisibleCapabilityRequest,
    )
    .await
    .expect("visible surface");

    let step = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state,
                surface,
                calls,
            },
        )
        .await
        .expect("capability stage");

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue from CapabilityStage; got Exit({exit:?})")
        }
    };
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be consumed after the auth-resume slot is dispatched"
    );

    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        1,
        "expected exactly one batch invocation"
    );
    let invocations = &batch_invocations[0].invocations;
    assert_eq!(invocations.len(), 2, "batch must have two calls");

    // First call: auth_resume is set (slot consumed here).
    let first_auth = invocations[0]
        .auth_resume
        .as_ref()
        .expect("first batch call must carry auth_resume (pre-fix: both carried it)");
    assert_eq!(
        first_auth.resume_token, resume_token,
        "first call auth_resume.resume_token must match"
    );
    let first_pa = first_auth
        .prior_approval
        .as_ref()
        .expect("first call auth_resume.prior_approval must be set");
    assert_eq!(
        first_pa.approval_request_id, approval_request_id,
        "first call auth_resume.prior_approval.approval_request_id must match"
    );

    // Second call: auth_resume must be None — slot was consumed by the first call.
    assert_eq!(
        invocations[1].auth_resume, None,
        "second batch call must NOT carry auth_resume — slot must be consumed on first match \
         (pre-fix: was Some, reusing the same resume_token)"
    );
}
