// arch-exempt: large_file, canonical executor regression remains with shared loop fixtures, plan #4088
use std::{collections::VecDeque, sync::Arc};

use ironclaw_host_api::{
    ApprovalRequestId, CorrelationId, DispatchInputIssueCode, ProviderToolName,
};
use ironclaw_turns::{
    CapabilityActivityId, GateResumeDisposition, LoopCancelledReasonKind, LoopCompletionKind,
    LoopDiagnosticRef, LoopExit, LoopFailureKind, LoopGateRef, LoopResultRef, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityAuthResume,
        CapabilityCallCandidate, CapabilityFailureDetail, CapabilityFailureKind,
        CapabilityInputIssue, CapabilityInputRef, CapabilityInputRepair, CapabilityRecoveryHint,
        CapabilityResumeToken, LoopCancelReasonKind, LoopCancellationSignal, LoopCheckpointKind,
        LoopCompactionError, LoopCompactionOutcome, LoopCompactionResponse,
        LoopContextCompactionKind, LoopInput, LoopInputAckToken, LoopInputBatch, LoopInputCursor,
        LoopInterruptKind, LoopModelCapabilityView, LoopProcessRef, LoopProgressEvent,
        LoopRunInfoPort, LoopSafeSummary, LoopSummaryArtifactId,
        MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation,
        ObservationTrust, ParentLoopOutput, PromptMode, ProviderToolCallReplay,
        SameCallRetryConstraint, ToolObservationDetail, ToolObservationStatus,
        VisibleCapabilityRequest, resolution,
    },
};

use crate::state::{
    CapabilityCallSignature, CheckpointKind, DeferredCompactionWatermark, IndexedMessageKind,
    LoopExecutionState, MessageIndexEntry, ModelErrorObservationClass,
    ModelErrorRecoveryObservation, PendingApprovalResume, PendingAuthResume,
    PendingModelRetryDirective, RepeatedCallWarningPhase, RepeatedCallWarningState,
};
use crate::strategies::{
    CapabilityBatchTurnSummary, CapabilityFilter, DefaultCompactionStrategy, GateKind, GateOutcome,
    StopKind, TurnSummary,
};
use crate::test_support::compaction::{
    active_task_preserving_compaction_index, compaction_metadata,
};
use crate::test_support::{
    MockAgentLoopDriverHost as DriverMockHost, MockHostCall, ScenarioScript,
    ScriptedCapabilityCall, ScriptedCapabilityOutcome, ScriptedModelResponse,
};

use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AssistantReplyInput, AssistantReplyStage, BatchStep,
    BudgetInput, BudgetStage, BudgetStep, CanonicalAgentLoopExecutor, CapabilityInput,
    CapabilityStage, DrainInput, ExecutorStage, ExitInput, ExitStage, GateInput, GateStage,
    HostStage, InputStage, InputStep, ModelInput, ModelStage, PendingInputAck, PromptInput,
    PromptStage, PromptStep, StageContext, StopInput, StopStage, StopStep, TurnCompletedStep,
    UserFacingInputDrainMode, consume_drainable_inputs, sanitize_result_ref_suffix,
    synthetic_provider_error_result_ref,
};

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}

mod support;
use support::*;

mod cancellation;
mod failure_matrix;

fn continuation_observation(
    result_ref: &LoopResultRef,
    byte_len: u64,
) -> ModelVisibleToolObservation {
    ModelVisibleToolObservation {
        schema_version: 1,
        status: ToolObservationStatus::Success,
        summary: "Use result_read to continue this child result.".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len,
            preview: Some("first bounded chunk".to_string()),
            total_bytes: Some(byte_len * 2),
            next_offset: Some(byte_len),
            item_count: None,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

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
async fn explanation_prompt_bundle_error_degrades_to_original_failed_exit() {
    let host = MockHost::new(Vec::new()).with_failing_prompt_bundle();
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

    match step {
        BudgetStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::IterationLimit);
            assert!(failed.explanation_message_refs.is_empty());
            assert!(failed.checkpoint_id.is_some());
        }
        _ => panic!("expected iteration-limit failed exit"),
    }
    assert_eq!(
        host.prompt_requests().len(),
        1,
        "failure explanation should attempt one prompt bundle"
    );
    assert!(
        host.model_requests().is_empty(),
        "prompt-bundle failure should not call the explanation model"
    );
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
async fn prompt_stage_circuit_breaker_disables_compaction_after_repeated_ineffective_runs() {
    use ironclaw_turns::run_profile::PromptContextTokenBudget;

    use crate::state::CompactionStrategyState;

    // Threshold is 90 tokens (100 - 10). Each compaction retains a 60-token
    // assistant tail — BELOW the threshold, so measuring right after
    // retain_after_sequence (BUG B2) records every attempt as effective and
    // the breaker never opens. Only the rebuilt bundle shows the injected
    // 40-token summary pushing the prompt back over the threshold
    // (100 >= 90): three REAL ineffective compactions driven through the
    // prompt stage must open the breaker, and the fourth threshold overflow
    // must be suppressed.
    let compacting_run = |summary_seq: u64, user_seq: u64| {
        vec![
            // Candidate bundle: over threshold with an eligible user boundary.
            vec![
                compaction_metadata(summary_seq, LoopContextCompactionKind::Summary, 40),
                compaction_metadata(summary_seq + 1, LoopContextCompactionKind::Assistant, 60),
                compaction_metadata(user_seq, LoopContextCompactionKind::User, 20),
                compaction_metadata(user_seq + 1, LoopContextCompactionKind::Assistant, 60),
            ],
            // Rebuild after compacting through user_seq: the 60-token tail is
            // under the threshold, but summary + tail is over (100 >= 90) —
            // ineffective.
            vec![
                compaction_metadata(user_seq, LoopContextCompactionKind::Summary, 40),
                compaction_metadata(user_seq + 1, LoopContextCompactionKind::Assistant, 60),
            ],
        ]
    };
    let mut prompt_indexes = vec![
        // First run's candidate bundle (no prior summary yet).
        vec![
            compaction_metadata(1, LoopContextCompactionKind::User, 20),
            compaction_metadata(2, LoopContextCompactionKind::Assistant, 60),
            compaction_metadata(3, LoopContextCompactionKind::User, 20),
            compaction_metadata(4, LoopContextCompactionKind::Assistant, 60),
        ],
        // First rebuild: injected summary keeps the prompt over threshold.
        vec![
            compaction_metadata(3, LoopContextCompactionKind::Summary, 40),
            compaction_metadata(4, LoopContextCompactionKind::Assistant, 60),
        ],
    ];
    prompt_indexes.extend(compacting_run(3, 5));
    prompt_indexes.extend(compacting_run(5, 7));
    // Fourth run's candidate bundle: over threshold with an eligible user
    // boundary, so only the open circuit can explain a skip.
    prompt_indexes.push(vec![
        compaction_metadata(7, LoopContextCompactionKind::Summary, 40),
        compaction_metadata(8, LoopContextCompactionKind::Assistant, 60),
        compaction_metadata(9, LoopContextCompactionKind::User, 20),
        compaction_metadata(10, LoopContextCompactionKind::Assistant, 60),
    ]);
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(prompt_indexes)
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 1,
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());

    for completed_compactions in 1..=CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT {
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
            _ => panic!("expected prepared prompt"),
        };
        state = output.state;
        assert_eq!(
            state.compaction_state.consecutive_ineffective_compactions, completed_compactions,
            "each rebuilt prompt stays over threshold, so every completed \
             compaction must count as ineffective"
        );
        assert_eq!(
            state.compaction_state.compaction_circuit_open,
            completed_compactions == CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT,
            "the circuit must open exactly on the third real ineffective compaction"
        );
        assert_eq!(
            host.progress_event_names()
                .iter()
                .filter(|&&name| name == "compaction_started")
                .count(),
            completed_compactions as usize
        );
    }

    // Drive the prompt stage again with the breaker open and the prompt still
    // over threshold: threshold-triggered compaction must NOT run again.
    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state,
                pending_input_ack: PendingInputAck::default(),
            },
        )
        .await
        .expect("prompt stage after breaker opened");
    let output = match step {
        PromptStep::Prepared(output) => output,
        _ => panic!("expected prepared prompt"),
    };
    assert!(output.state.compaction_state.compaction_circuit_open);
    assert_eq!(
        host.progress_event_names()
            .iter()
            .filter(|&&name| name == "compaction_started")
            .count(),
        CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT as usize,
        "an open circuit breaker must stop threshold-triggered compactions for the run"
    );
}

#[tokio::test]
async fn prompt_stage_forced_compaction_bypasses_open_circuit_breaker() {
    use ironclaw_turns::run_profile::PromptContextTokenBudget;

    // BUG B1 regression: force_compact_on_next_iteration (context-overflow
    // recovery via RetryAlteration::ShrinkContext, byte-cap overflow) must
    // run its compaction even with the breaker open — otherwise the same
    // oversized prompt is rebuilt until the retry budget aborts.
    // BUG B3 regression: the forced compaction is judged against the prompt
    // it started from (260 tokens), not the 90-token transcript threshold —
    // shrinking to 140 tokens is effective and resets the counter, even
    // though 140 is still over the threshold.
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(vec![
            // Candidate bundle: 260 tokens with an eligible user boundary.
            vec![
                compaction_metadata(1, LoopContextCompactionKind::Summary, 40),
                compaction_metadata(2, LoopContextCompactionKind::Assistant, 100),
                compaction_metadata(3, LoopContextCompactionKind::User, 20),
                compaction_metadata(4, LoopContextCompactionKind::Assistant, 100),
            ],
            // Rebuild after compacting through seq 3: shrank to 140 tokens.
            vec![
                compaction_metadata(3, LoopContextCompactionKind::Summary, 40),
                compaction_metadata(4, LoopContextCompactionKind::Assistant, 100),
            ],
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 1,
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.compaction_circuit_open = true;
    state.compaction_state.consecutive_ineffective_compactions =
        crate::state::CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT;
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
        _ => panic!("expected prepared prompt"),
    };
    assert_eq!(
        host.progress_event_names()
            .iter()
            .filter(|&&name| name == "compaction_started")
            .count(),
        1,
        "a forced compaction must bypass the open circuit breaker"
    );
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        Some(3)
    );
    assert!(
        !output
            .state
            .compaction_state
            .force_compact_on_next_iteration
    );
    assert_eq!(
        output
            .state
            .compaction_state
            .consecutive_ineffective_compactions,
        0,
        "a forced compaction that shrank the prompt (260 -> 140) is effective \
         against its pre-compaction baseline even though 140 is still over \
         the 90-token transcript threshold"
    );
    assert!(
        output.state.compaction_state.compaction_circuit_open,
        "the breaker is one-way; an effective forced compaction does not close it"
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
async fn prompt_stage_compaction_inference_timeout_returns_to_normal_prompt_path() {
    // Simulates the inner `ModelGatewayBackedSystemInferencePort` deadline
    // firing: it surfaces as `LoopCompactionError::InferenceFailed`, not as
    // a separate outer race in the executor (that duplicate timeout was
    // removed — see `await_compaction_with_cancellation`).
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Err(LoopCompactionError::InferenceFailed {
            safe_summary: LoopSafeSummary::new("compaction deadline exceeded").unwrap(),
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
    };
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
    assert_eq!(host.prompt_requests().len(), 1);
    assert!(host.checkpoint_kinds().is_empty());
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "compaction_failed",
        ]
    );
}

#[tokio::test]
async fn prompt_stage_compaction_security_rejection_returns_to_normal_prompt_path() {
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

    let output = match step {
        PromptStep::Prepared(output) => output,
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
            panic!("unexpected resume step")
        }
        PromptStep::Exit(exit) => panic!("expected prepared prompt, got {exit:?}"),
        PromptStep::SkipModel(_, _) => panic!("unexpected SkipModel"),
    };
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
        host.prompt_requests().len(),
        1,
        "compaction failure should continue with the existing prompt candidate"
    );
    assert!(host.checkpoint_kinds().is_empty());
    assert_eq!(
        host.progress_event_names(),
        vec![
            "prompt_bundle_built",
            "compaction_started",
            "compaction_failed",
        ]
    );
}

#[tokio::test]
async fn compaction_failure_cancellation_skips_explanation_and_returns_cancelled() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_index(vec![compaction_metadata(
            1,
            LoopContextCompactionKind::User,
            10,
        )])
        .with_compaction_result(Err(LoopCompactionError::SecurityRejected {
            safe_summary: LoopSafeSummary::new("injection detected").unwrap(),
        }))
        .cancel_after_compaction_failure();
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
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
        }
        PromptStep::Exit(exit) => panic!("expected cancelled exit, got {exit:?}"),
        PromptStep::Prepared(_)
        | PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_)
        | PromptStep::SkipModel(_, _) => panic!("unexpected prompt step"),
    }
    assert_eq!(
        host.prompt_requests().len(),
        1,
        "compaction cancellation should not request a failure explanation prompt"
    );

    // The Final checkpoint must persist the deferred watermark set by the
    // failure-fallback path, not just the in-memory Cancelled exit.
    let final_state = final_staged_state(&host);
    assert!(
        !final_state.compaction_state.force_compact_on_next_iteration,
        "force_compact_on_next_iteration must be cleared before the Final checkpoint"
    );
    assert_eq!(
        final_state.compaction_state.last_deferred,
        Some(DeferredCompactionWatermark {
            through_seq: 1,
            prompt_fingerprint: final_state.compaction_prompt.fingerprint(),
        }),
        "deferred watermark must persist through the cancellation checkpoint"
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
async fn model_context_overflow_exhaustion_gives_model_one_observation_assisted_attempt() {
    let overflow = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "model request exceeded its context budget",
        )
    };
    let host = MockHost::new(vec![reply_response()]).with_model_errors(vec![
        overflow(),
        overflow(),
        overflow(),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("context-overflow observation should let the model recover");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("context overflowed; use the available context and continue")
    }));
}

#[tokio::test]
async fn model_budget_approval_required_with_gate_ref_blocks_resource_gate() {
    let gate_ref = LoopGateRef::new("gate:budget-test-approval").expect("gate ref");
    let host = MockHost::new(vec![reply_response()]).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetApprovalRequired,
            "budget approval required",
        )
        .with_gate_ref(gate_ref.clone()),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Blocked(blocked) => {
            assert_eq!(blocked.kind, ironclaw_turns::LoopBlockedKind::Resource);
            assert_eq!(blocked.gate_ref, gate_ref);
            assert_eq!(blocked.blocked_activity_id, None);
        }
        other => panic!("expected budget approval to block, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeBlock
        ]
    );
    assert!(host.progress_event_names().contains(&"gate_blocked"));
    let blocked_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert_eq!(blocked_state.last_gate, Some(gate_ref));
}

#[tokio::test]
async fn model_budget_approval_required_without_gate_ref_fails_diagnostics_not_recovery() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetApprovalRequired,
            "budget approval required",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("budget approval without gate evidence must fail closed");

    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::BudgetApprovalRequired,
            safe_summary: LoopSafeSummary::new("budget approval required").expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: None,
        }
    );
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel]
    );
    assert!(!host.progress_event_names().contains(&"gate_blocked"));
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
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

/// A tool-using run must count the usage from EVERY model response, including
/// the capability-call turn — not just the final assistant reply. Regression
/// for usage/cost being dropped on the `CapabilityCalls` branch: the executor
/// now accumulates before branching on the model output.
#[tokio::test]
async fn cumulative_usage_counts_capability_call_and_reply_turns() {
    use ironclaw_turns::run_profile::{LoopModelResponse, LoopModelUsage};

    let result_ref = LoopResultRef::new("result:done").expect("valid");
    // Turn 1 is a capability call carrying its own usage; turn 2 is the reply.
    let calls_usage = LoopModelUsage {
        input_tokens: 100,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    let reply_usage = LoopModelUsage {
        input_tokens: 40,
        output_tokens: 20,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    let calls = LoopModelResponse {
        usage: Some(calls_usage),
        ..calls_response()
    };
    let reply = LoopModelResponse {
        usage: Some(reply_usage),
        ..reply_response()
    };
    let host = MockHost::new(vec![calls, reply]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref,
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
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
            // Both turns' usage is summed: had the capability turn been dropped,
            // this would be only the reply's 40/20.
            assert_eq!(
                completed.model_usage,
                Some(LoopModelUsage {
                    input_tokens: 140,
                    output_tokens: 20,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            );
        }
        other => panic!("expected completed, got {other:?}"),
    }
}

#[tokio::test]
async fn reply_admission_rendered_flag_stays_false_when_context_suppresses_control_message() {
    let result_ref = LoopResultRef::new("result:done").expect("valid");
    let host = MockHost::new(vec![reply_response(), calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
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
    assert_eq!(
        host.model_requests().len(),
        4,
        "invalid model output should attempt one best-effort explanation call"
    );
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
        // Multi-line, all provider-transcript-artifact lines: a replayed-history
        // echo (still rejected under the multi-line artifact rule).
        reply_response_with_text(
            "Previous tool event: demo__echo was invoked.\nTool result from demo__echo: hi",
        ),
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
    let host =
        MockHost::new(Vec::new()).with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        }]);
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
                        no_change_signatures: Vec::new(),
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
        activity_id: ironclaw_turns::CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: capability_id(),
        input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            provider_turn_id: oversized,
            provider_call_id: "call/with space".to_string(),
            provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
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
async fn exit_stage_no_progress_fails_when_nudge_disabled() {
    // Production default: the driver-specific nudge gate is off, so a no-progress
    // stop produces a typed `NoProgressDetected` failure with a Final checkpoint —
    // NOT a canned "I stopped" reply finalized as a completed turn. The failed
    // branch attaches a best-effort failure explanation (§5a.2); with no model
    // response available the explanation fails soft and the typed failure still
    // carries the honest category the product layer renders deterministically.
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
            // Final checkpoint is mandatory for the failed exit to validate
            // through `verify_failure_evidence` (parity with the Aborted arm).
            assert!(failed.checkpoint_id.is_some());
            assert!(
                failed.explanation_message_refs.is_empty(),
                "a failed explanation model call must fail soft with no refs"
            );
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
    // The single model call is the best-effort failure explanation (§5a.2),
    // never the nudge: the nudge gate is off, so the nudge counter stays 0 —
    // and no assistant reply was finalized.
    assert_eq!(
        host.model_requests().len(),
        1,
        "only the best-effort failure-explanation call may be issued"
    );
    assert_eq!(
        final_staged_state(&host).final_answer_nudges_used,
        0,
        "nudge gate disabled must not attempt a nudge"
    );
    assert!(
        host.finalized_assistant_messages().is_empty(),
        "no assistant reply is finalized when the explanation call fails"
    );
}

#[tokio::test]
async fn no_progress_explanation_cancellation_returns_cancelled_before_final_checkpoint() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![AgentLoopHostError::new(
        AgentLoopHostErrorKind::Cancelled,
        "cancelled during no-progress explanation",
    )]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = ExitStage
        .process(
            ctx,
            ExitInput {
                state,
                kind: StopKind::NoProgressDetected,
            },
        )
        .await;

    assert!(
        matches!(result, Err(AgentLoopExecutorError::Cancelled)),
        "cancellation during the no-progress explanation must propagate before failure finalization: {result:?}"
    );
    assert!(
        host.checkpoint_kinds().is_empty(),
        "cancellation must not write a Final checkpoint for a failed no-progress exit"
    );
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test]
async fn no_progress_nudge_synthesizes_reply_when_gate_enabled() {
    // Gate ON + a model reply queued for the tool-free nudge call: the
    // no-progress exit should issue ONE tool-free model call and finalize the
    // synthesized reply instead of the canned fallback.
    let host = MockHost::new(vec![reply_response_with_text("Here is the final answer.")])
        .with_driver_nudges_enabled();
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

    // Exactly one tool-free model call was issued (the nudge), with an empty
    // capability view so the provider gets no tools.
    let requests = host.model_requests();
    assert_eq!(requests.len(), 1, "nudge should issue one model call");
    assert_eq!(
        requests[0]
            .capability_view
            .as_ref()
            .map(|v| v.visible_capability_ids.len()),
        Some(0),
        "nudge model call must be tool-free (empty capability view)"
    );
    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected completed exit with synthesized reply, got {other:?}"),
    }
}

#[tokio::test]
async fn no_progress_skips_nudge_when_gate_disabled() {
    // Gate OFF: no tool-free nudge call is issued and the no-progress stop
    // terminates as a typed failure (production default) — not a canned reply,
    // not a completed turn. The available model reply is consumed by the
    // failure-explanation call (§5a.2) and referenced from the failed exit.
    let host = MockHost::new(vec![reply_response_with_text(
        "explanation after no progress",
    )]);
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

    // Exactly one model call — the failure explanation, not a nudge (the nudge
    // counter stays 0 and the run still FAILS; a successful nudge would have
    // completed the run instead).
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        final_staged_state(&host).final_answer_nudges_used,
        0,
        "no nudge attempt when the gate is disabled"
    );
    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert_eq!(
                failed.explanation_message_refs.len(),
                1,
                "the failure explanation must be referenced from the failed exit"
            );
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_iteration_limit_nudges_to_completed_when_gate_enabled() {
    // Gate ON at the iteration-limit boundary: instead of failing closed, issue
    // one tool-free nudge and complete with the synthesized reply.
    let host = MockHost::new(vec![reply_response_with_text(
        "Final answer from budget nudge.",
    )])
    .with_driver_nudges_enabled();
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

    assert_eq!(
        host.model_requests().len(),
        1,
        "budget nudge should issue one model call"
    );
    assert!(
        matches!(step, BudgetStep::Exit(LoopExit::Completed(_))),
        "budget nudge should complete, not fail closed"
    );
}

#[tokio::test]
async fn nudge_respects_one_shot_cap() {
    // With the cap already spent, the no-progress exit must not issue another
    // NUDGE model call and terminates as a typed failure (no canned reply).
    // The failed branch still runs its failure-explanation call (§5a.2), which
    // consumes the scripted reply as the explanation message.
    let host = MockHost::new(vec![reply_response_with_text(
        "explanation after capped nudge",
    )])
    .with_driver_nudges_enabled();
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.final_answer_nudges_used = 1;

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

    // The nudge cap held: the counter did not advance, the run FAILED (a
    // successful nudge would have completed it), and the single model call is
    // the failure explanation.
    assert_eq!(
        final_staged_state(&host).final_answer_nudges_used,
        1,
        "capped nudge must not attempt another nudge"
    );
    assert_eq!(host.model_requests().len(), 1);
    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert_eq!(failed.explanation_message_refs.len(), 1);
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
}

#[tokio::test]
async fn completion_nudge_lets_model_use_tools_to_finish_after_trailing_off() {
    // The task-flip proof, end-to-end through the real loop: the model reads,
    // then trails off announcing a write it never performs (reply ends with ':').
    // WITH the gate on, the loop re-enters with the FULL tool surface + the
    // completion-nudge directive; the model then executes its write tool and
    // gives a real closing answer — the run completes having produced the
    // artifact it was trailing off on.
    let result_ref = LoopResultRef::new("result:file-written").expect("valid");
    let host = MockHost::new(vec![
        // Turn 1: trails off — "let me write the file:" with no tool call.
        reply_response_with_text("Here are the recommendations, let me write them to the file:"),
        // Turn 2 (the nudged retry): actually invokes the write tool.
        calls_response(),
        // Turn 3: real closing answer.
        reply_response_with_text("Done — wrote the recommendations to the output file."),
    ])
    .with_driver_nudges_enabled()
    .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
        resolutions: vec![resolution::completed(
            result_ref.clone(),
            "wrote file".to_string(),
            ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )],
        stopped_on_suspension: false,
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "expected completed exit, got {exit:?}"
    );
    // The write tool ran exactly once — on the nudged retry (it could not have
    // run on the trailed-off turn 1, which emitted no tool call).
    assert_eq!(
        host.batch_invocations().len(),
        1,
        "the completion nudge must let the model execute its write tool"
    );
    // Three prompt-driven model calls: trail-off, tool call, closing answer.
    let prompt_requests = host.prompt_requests();
    assert_eq!(
        prompt_requests.len(),
        3,
        "expected trail-off + nudged retry + close"
    );
    assert!(prompt_requests[0].inline_messages.is_empty());
    assert!(
        prompt_requests[1]
            .inline_messages
            .iter()
            .any(|m| m.safe_body.as_str().contains("Finish the task now")),
        "the nudged retry must carry the completion-nudge directive"
    );
    assert_eq!(final_staged_state(&host).completion_nudges_used, 1);
}

#[tokio::test]
async fn completion_nudge_disabled_leaves_trailed_off_run_without_tool_use() {
    // Control: the SAME trailed-off trajectory with the gate OFF (production
    // default). The run ends right after the trail-off — the write tool is never
    // reached, so the artifact is never produced (the failure the nudge fixes).
    let host = MockHost::new(vec![
        reply_response_with_text("Here are the recommendations, let me write them to the file:"),
        // Present but must NOT be consumed — the loop must not continue.
        calls_response(),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.prompt_requests().len(),
        1,
        "without the nudge the run ends after the trailed-off turn"
    );
    assert_eq!(
        host.batch_invocations().len(),
        0,
        "without the nudge no tool runs — the artifact is never written"
    );
    assert_eq!(final_staged_state(&host).completion_nudges_used, 0);
}

#[tokio::test]
async fn completion_nudge_skipped_on_clean_reply() {
    // Gate ON but the first reply is a clean, complete answer (does not trail
    // off): no nudge fires, a single prompt-driven model call, graceful
    // completion. Guards against regressing correct short answers.
    let host = MockHost::new(vec![reply_response_with_text(
        "Here is the complete answer.",
    )])
    .with_driver_nudges_enabled();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.prompt_requests().len(),
        1,
        "a clean, complete reply must not trigger a completion nudge"
    );
    assert_eq!(
        final_staged_state(&host).completion_nudges_used,
        0,
        "no completion nudge issued for a clean reply"
    );
}

#[tokio::test]
async fn no_progress_nudge_model_failure_falls_back_to_failed_exit() {
    // Gate ON but the nudge's OWN model call fails (non-cancel host error). The
    // nudge is best-effort: it must NOT bork the run — the no-progress exit
    // falls back to the typed failed exit instead of propagating the failure.
    let host = MockHost::new(Vec::new())
        .with_driver_nudges_enabled()
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "nudge model call failed",
        )]);
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
        .expect("nudge model failure must not propagate out of the exit stage");

    // Two model calls: the nudge attempt (scripted error) and the best-effort
    // failure-explanation attempt (§5a.2, script exhausted → fails soft).
    assert_eq!(
        host.model_requests().len(),
        2,
        "nudge attempts one model call before failing open; the failed branch \
         then attempts its best-effort failure explanation"
    );
    assert!(
        matches!(exit, LoopExit::Failed(_)),
        "nudge model failure must fall back to the typed no-progress failure, got {exit:?}"
    );
}

#[tokio::test]
async fn budget_nudge_model_failure_falls_back_to_failed_exit() {
    // Gate ON at the iteration-limit boundary, but the nudge's model call fails.
    // The budget stage must fall through to its normal explained-failure exit
    // (also fail-open) rather than propagating the nudge host error.
    let host = MockHost::new(Vec::new())
        .with_driver_nudges_enabled()
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "nudge model call failed",
        )]);
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
        .expect("nudge model failure must not propagate out of the budget stage");

    assert_eq!(
        host.model_requests().len(),
        2,
        "budget path attempts the failed nudge model call plus its normal failure explanation"
    );
    assert!(
        matches!(step, BudgetStep::Exit(LoopExit::Failed(_))),
        "budget nudge model failure must fall back to the failed exit"
    );
}

#[tokio::test]
async fn nudge_model_cancellation_propagates() {
    // A cancellation surfaced during the nudge model call MUST propagate (the
    // run is being cancelled), unlike other host failures which fall open.
    let host = MockHost::new(Vec::new())
        .with_driver_nudges_enabled()
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Cancelled,
            "cancelled during nudge",
        )]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = ExitStage
        .process(
            ctx,
            ExitInput {
                state,
                kind: StopKind::NoProgressDetected,
            },
        )
        .await;

    assert!(
        matches!(result, Err(AgentLoopExecutorError::Cancelled)),
        "nudge cancellation must propagate, got {result:?}"
    );
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
    let final_state = host
        .staged_payloads()
        .into_iter()
        .find(|request| request.kind == LoopCheckpointKind::Final)
        .map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                checkpoint_kind_from_host(request.kind),
            )
            .expect("final checkpoint payload")
        })
        .expect("final checkpoint");
    assert!(
        final_state
            .recent_failure_kinds
            .iter()
            .any(|kind| *kind == LoopFailureKind::CapabilityProtocolError),
        "final failed checkpoint must carry the terminal failure kind for evidence validation"
    );
}

#[tokio::test]
async fn stopped_on_suspension_completed_outcome_still_appends_result() {
    let result_ref = LoopResultRef::new("result:stopped-completed").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "stopped batch completed".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
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
                        no_change_signatures: Vec::new(),
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:done").expect("valid"),
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new("gate:approval").expect("valid"),
                    "approval required".to_string(),
                    None,
                )
                .resolution,
            ],
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
    };
    let completed_ref = LoopResultRef::new("result:approval-resumed").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!(
                        "gate:approval-{}",
                        approval_resume.approval_request_id
                    ))
                    .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "approval resumed".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
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
    // correlation_id is observability-only post-§5.3 Stage 2 flip: the executor
    // reconstructs the approval identity from the `gate:approval-{id}` ref and
    // regenerates a fresh correlation_id, so it is NOT byte-stable with the
    // original. The authoritative correlation is reconstituted host-side from the
    // replay payload (§5.3 Stage 2a-ii). Assert it is present, not equal.
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
    // The replayed invocation carries the byte-stable approval identity
    // (request id, resume token, input ref) reconstructed from the gate ref.
    // correlation_id is observability-only post-flip and regenerated, so the
    // full struct is NOT equal to the original — assert the stable fields.
    let replayed_resume = batch_invocations[1].invocations[0]
        .approval_resume
        .as_ref()
        .expect("resume metadata");
    assert_eq!(
        replayed_resume.approval_request_id,
        approval_resume.approval_request_id
    );
    assert_eq!(replayed_resume.resume_token, approval_resume.resume_token);
    assert_eq!(replayed_resume.input_ref, approval_resume.input_ref);
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

/// Focused regression for gates.rs:85 — `GateStage` must stamp
/// `disposition: None` on the initial (blocking) `PendingApprovalResume`
/// checkpoint.  A denial has not yet occurred at block time; writing any
/// non-`None` disposition here would short-circuit the capability stage
/// incorrectly on the very next resume, before any user deny action.
#[tokio::test]
async fn approval_gate_before_block_checkpoint_disposition_is_none() {
    let approval_resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("resume-token:disposition-none-test")
            .expect("valid token"),
        correlation_id: CorrelationId::new(),
        input_ref: CapabilityInputRef::new("input:disposition-none-test").expect("valid"),
    };
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!(
                        "gate:approval-{}",
                        approval_resume.approval_request_id
                    ))
                    .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
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

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_resume = before_block_state
        .pending_approval_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_approval_resume");

    // Key regression assertion: disposition must be None at the first-block
    // checkpoint.  A non-None value here means GateStage pre-stamped a denial
    // that hasn't happened yet, which would cause the capability stage to
    // incorrectly short-circuit on the very next resume.
    assert_eq!(
        pending_resume.disposition, None,
        "pending_approval_resume.disposition must be None at the initial BeforeBlock checkpoint \
         (no denial has occurred yet — GateStage must not pre-stamp a disposition)"
    );
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
                auth_resume: None,
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
                auth_resume: None,
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new("gate:approval").expect("valid"),
                    "approval required".to_string(),
                    None,
                )
                .resolution,
                resolution::completed(
                    completed_ref.clone(),
                    "parallel call completed".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: Vec::new(),
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::completed(
                    LoopResultRef::new("result:second").expect("valid"),
                    "second".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
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
async fn prompt_reuses_current_visible_surface_without_refetching() {
    let host = MockHost::new(vec![reply_response()])
        .with_current_default_visible_surface()
        .with_failing_visible_capabilities();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.visible_capability_request_count(), 0);
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
            detail: None,
        }
    );
}

#[tokio::test]
async fn model_budget_accounting_failure_preserves_kind_without_model_retry() {
    let accounting_error = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetAccountingFailed,
            "resource accounting storage is unavailable",
        )
    };
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        accounting_error(),
        accounting_error(),
        accounting_error(),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("accounting outages must remain typed host failures");

    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::BudgetAccountingFailed,
            safe_summary: LoopSafeSummary::new("resource accounting storage is unavailable")
                .expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: None,
        }
    );
}

/// A typed stale surface is model-fixable-by-rebuild: an iteration-scoped retry
/// rebuilds the capability surface and prompt bundle, so a surface refreshed
/// mid-iteration no longer hard-borks the run invisible to the model.
#[tokio::test]
async fn model_stale_surface_retries_iteration_with_fresh_bundle_and_completes() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::StaleSurface,
            "model request surface version does not match the host-built prompt bundle",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("typed stale surface must be recoverable");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(
        host.prompt_requests().len(),
        2,
        "stale-surface retry must rebuild the host prompt bundle"
    );
}

#[tokio::test]
async fn model_invalid_request_kinds_are_terminal_without_retry() {
    for kind in [
        AgentLoopHostErrorKind::InvalidInvocation,
        AgentLoopHostErrorKind::Invalid,
    ] {
        let host =
            MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
                kind,
                "model request is deterministically invalid",
            )]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let error = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect_err("deterministic invalid model requests must terminate");

        assert!(matches!(
            error,
            AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                stage: HostStage::Model,
                kind: actual_kind,
                ..
            } if actual_kind == kind
        ));
        assert_eq!(
            host.model_requests().len(),
            1,
            "{kind:?} must not consume a retry"
        );
        assert_eq!(
            host.prompt_requests().len(),
            1,
            "{kind:?} must not rebuild the prompt for a deterministic failure"
        );
    }
}

/// When the stale-request retry budget is exhausted the run fails gracefully
/// with the precise `model_stale_request` category — not a terminal
/// `HostUnavailableWithDiagnostics` that collapses to a generic
/// model-unavailable failure.
#[tokio::test]
async fn model_stale_request_exhaustion_fails_with_stale_request_category() {
    let stale = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::StaleSurface,
            "model request surface version does not match the host-built prompt bundle",
        )
    };
    let host = MockHost::new(Vec::new()).with_model_errors(vec![stale(), stale(), stale()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("stale-request exhaustion must fail gracefully, not hard-bork");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
            let summary = failed.safe_summary.expect("stale-request failure summary");
            assert_eq!(summary.category(), "model_stale_request");
        }
        other => panic!("expected stale-request failed exit, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        3,
        "stale-request retries are bounded by the per-class budget"
    );
}

/// A model-path `Unauthorized` host error terminates immediately with the
/// pinned, user-actionable `model_credentials_unavailable` category (fix the
/// key/permissions) instead of a generic model-unavailable failure, and is
/// never silently retried.
#[tokio::test]
async fn model_unauthorized_fails_with_credentials_category_without_retry() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unauthorized,
            "model access was unauthorized",
        )
        .with_detail("HTTP 401 from provider"),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("unauthorized model errors must fail gracefully with a precise category");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
            let summary = failed.safe_summary.expect("unauthorized failure summary");
            assert_eq!(summary.category(), "model_credentials_unavailable");
            assert_eq!(summary.detail(), Some("HTTP 401 from provider"));
        }
        other => panic!("expected unauthorized failed exit, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "unauthorized model errors must not be silently retried"
    );
}

/// Model-path checkpoint/transcript host error kinds terminate with their
/// precise failure kinds and categories instead of the generic host-stage
/// unavailability collapse.
#[tokio::test]
async fn model_checkpoint_and_transcript_kinds_fail_with_precise_categories() {
    for (kind, expected_failure_kind, expected_category) in [
        (
            AgentLoopHostErrorKind::CheckpointRejected,
            LoopFailureKind::CheckpointRejected,
            "checkpoint_rejected",
        ),
        (
            AgentLoopHostErrorKind::TranscriptWriteFailed,
            LoopFailureKind::TranscriptWriteFailed,
            "transcript_write_failed",
        ),
    ] {
        let host = MockHost::new(Vec::new()).with_model_errors(vec![AgentLoopHostError::new(
            kind,
            "model stage host persistence failed",
        )]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .unwrap_or_else(|error| {
                panic!("model-path {kind:?} must fail gracefully, got hard error {error:?}")
            });

        match exit {
            LoopExit::Failed(failed) => {
                assert_eq!(
                    failed.reason_kind, expected_failure_kind,
                    "kind for {kind:?}"
                );
                let summary = failed.safe_summary.expect("failure summary");
                assert_eq!(
                    summary.category(),
                    expected_category,
                    "category for {kind:?}"
                );
            }
            other => panic!("expected {kind:?} failed exit, got {other:?}"),
        }
        assert_eq!(
            host.model_requests().len(),
            1,
            "{kind:?} model errors must not be silently retried"
        );
    }
}

#[tokio::test]
async fn model_invalid_output_exhaustion_gives_model_structured_repair_attempt() {
    let host = MockHost::new(vec![reply_response()]).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidOutput,
            "model returned an empty assistant response",
        ),
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidOutput,
            "model returned an empty assistant response",
        ),
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidOutput,
            "model returned an empty assistant response",
        ),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].inline_messages.iter().any(|message| {
        let body = message.safe_body.as_str();
        body.contains("invalid_output") && body.contains("empty_assistant_response")
    }));
}

#[tokio::test]
async fn model_error_observation_attempt_is_bounded_before_terminal_failure() {
    let invalid = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidOutput,
            "model returned an empty assistant response",
        )
    };
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        invalid(),
        invalid(),
        invalid(),
        invalid(),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("bounded observation retry should end in a typed failure");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::InvalidModelOutput);
            let summary = failed.safe_summary.expect("model failure summary");
            assert_eq!(summary.category(), "model_invalid_output");
            assert_eq!(
                summary.detail(),
                Some("model returned an empty assistant response")
            );
        }
        other => panic!("expected invalid-model-output failed exit, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 4);
    assert_eq!(
        host.model_requests()
            .iter()
            .flat_map(|request| &request.inline_messages)
            .filter(|message| message
                .safe_body
                .as_str()
                .contains("model error observation"))
            .count(),
        1,
        "the exhausted class gets exactly one observation-assisted attempt"
    );
}

#[tokio::test]
async fn model_retry_transition_survives_checkpoint_reload_before_retry() {
    let content_filtered = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContentFiltered,
            "model completion was filtered",
        )
    };
    let host = Arc::new(
        MockHost::new(Vec::new())
            .with_model_errors(vec![content_filtered(), content_filtered()])
            .crash_after_checkpoint_progress(LoopCheckpointKind::BeforeModel),
    );
    let crashed_host = Arc::clone(&host);
    let crash = match tokio::spawn(async move {
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host: crashed_host.as_ref(),
        };
        let state = LoopExecutionState::initial_for_run(crashed_host.run_context());
        ModelStage
            .process(
                ctx,
                ModelInput {
                    state,
                    messages: Vec::new(),
                    inline_messages: Vec::new(),
                    surface_version: surface_version(),
                    capability_view: LoopModelCapabilityView {
                        visible_capability_ids: Vec::new(),
                    },
                },
            )
            .await
    })
    .await
    {
        Err(join_error) => join_error,
        Ok(_) => panic!("scripted worker crash must stop before the retry"),
    };
    assert!(crash.is_panic());

    let restored = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeModel);
    assert!(
        restored
            .recovery_state
            .observation_attempted_for(ModelErrorObservationClass::ContentFiltered)
    );
    assert_eq!(
        restored.pending_model_error_observation,
        Some(ModelErrorRecoveryObservation::content_filtered())
    );

    // Simulate a new worker loading the last committed BeforeModel payload.
    // The same provider failure must now abort instead of granting a second
    // observation-assisted attempt.
    let family = crate::families::default();
    let exit = CanonicalAgentLoopExecutor
        .execute_family(&family, host.as_ref(), restored)
        .await
        .expect("reloaded retry state should fail through the typed exit");

    assert!(matches!(
        exit,
        LoopExit::Failed(ref failed) if failed.reason_kind == LoopFailureKind::ModelError
    ));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("provide a policy compliant alternative")
    }));
}

#[tokio::test]
async fn invalid_output_repair_directive_survives_checkpoint_reload_before_retry() {
    let host = Arc::new(
        MockHost::new(vec![reply_response()])
            .with_model_errors(vec![AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidOutput,
                "model returned an empty assistant response",
            )])
            .crash_after_checkpoint_progress(LoopCheckpointKind::BeforeModel),
    );
    let crashed_host = Arc::clone(&host);
    let crash = match tokio::spawn(async move {
        let family = crate::families::default();
        ModelStage
            .process(
                StageContext {
                    planner: family.planner(),
                    host: crashed_host.as_ref(),
                },
                ModelInput {
                    state: LoopExecutionState::initial_for_run(crashed_host.run_context()),
                    messages: Vec::new(),
                    inline_messages: Vec::new(),
                    surface_version: surface_version(),
                    capability_view: LoopModelCapabilityView {
                        visible_capability_ids: Vec::new(),
                    },
                },
            )
            .await
    })
    .await
    {
        Err(join_error) => join_error,
        Ok(_) => panic!("scripted worker crash must stop before the retry"),
    };
    assert!(crash.is_panic());

    let restored = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeModel);
    assert_eq!(
        restored.pending_model_retry_directive,
        Some(PendingModelRetryDirective::RepairInvalidOutput)
    );

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), host.as_ref(), restored)
        .await
        .expect("reloaded repair directive should reach the retry request");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("previous model response was empty or structurally invalid")
    }));
}

#[tokio::test]
async fn retry_transition_checkpoint_failure_stops_before_second_model_call() {
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContentFiltered,
            "model completion was filtered",
        )])
        .fail_checkpoint_on_occurrence(LoopCheckpointKind::BeforeModel, 2);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("retry-transition checkpoint failure must stop the run");

    assert!(matches!(
        error,
        AgentLoopExecutorError::CheckpointFailed {
            stage: CheckpointKind::BeforeModel
        }
    ));
    assert_eq!(host.model_requests().len(), 1);
}

#[tokio::test]
async fn unsupported_checkpointed_model_observation_stops_before_model_call() {
    let host = MockHost::new(vec![reply_response()]);
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    let mut observation = ModelErrorRecoveryObservation::content_filtered();
    observation.schema_version += 1;
    state.pending_model_error_observation = Some(observation);
    let payload = serde_json::to_vec(&state).expect("checkpoint state serializes");
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
            .expect("checkpoint state reloads before semantic validation");

    let error = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), &host, restored)
        .await
        .expect_err("unsupported observation must fail prompt construction");

    assert!(matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "model-error observation control text was invalid"
        }
    ));
    assert!(host.model_requests().is_empty());
}

#[tokio::test]
async fn model_content_filter_gives_model_one_rephrase_attempt() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContentFiltered,
            "model completion was filtered",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("content-filter observation should let the model rephrase");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("provide a policy compliant alternative")
    }));

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
        state.pending_model_error_observation
            == Some(ModelErrorRecoveryObservation::content_filtered())
    }));
    assert!(
        final_staged_state(&host)
            .pending_model_error_observation
            .is_none()
    );
}

#[tokio::test]
async fn model_unrecoverable_host_error_carries_detail_to_executor_error() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::CredentialUnavailable,
            "model credentials are unavailable",
        )
        .with_detail("HTTP 404 model not found"),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("unrecoverable model errors stop before a loop exit");

    match error {
        AgentLoopExecutorError::HostUnavailableWithDiagnostics { detail, .. } => {
            assert_eq!(detail.as_deref(), Some("HTTP 404 model not found"));
        }
        other => panic!("expected HostUnavailableWithDiagnostics, got {other:?}"),
    }
}

#[tokio::test]
async fn capability_abort_finalizes_explanation_and_failed_exit_refs_partial_first() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "The run stopped after a capability failure.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::failed("permanent")]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let partial_ref = message_ref("msg:partial-work");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(partial_ref.clone());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
            assert_eq!(
                failed.explanation_message_refs,
                vec![partial_ref, message_ref("msg:assistant")]
            );
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 2);
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["The run stopped after a capability failure.".to_string()]
    );
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    let explanation_capability_view = requests[1]
        .capability_view
        .as_ref()
        .expect("failure explanation model request must suppress tools");
    assert!(
        explanation_capability_view
            .visible_capability_ids
            .is_empty()
    );
    assert!(requests[1].surface_version.is_none());
    assert!(requests[1].model_preference.is_none());
}

#[tokio::test]
async fn failure_explanation_prompt_is_inline_only_and_context_free() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "The run stopped after a capability failure.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::failed("permanent")]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(message_ref("msg:partial-work"));

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Failed(_)));
    let requests = host.prompt_requests();
    assert_eq!(requests.len(), 2);
    let explanation = &requests[1];
    assert_eq!(explanation.mode, PromptMode::TextOnly);
    assert_eq!(explanation.max_messages, Some(0));
    assert!(explanation.context_cursor.is_none());
    assert!(explanation.surface_version.is_none());
    assert!(explanation.capability_view.is_none());
    assert!(explanation.checkpoint_state_ref.is_none());
    assert_eq!(explanation.inline_messages.len(), 2);
}

#[tokio::test]
async fn explanation_model_error_degrades_to_original_failed_exit() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Internal,
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::failed("permanent")]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let partial_ref = message_ref("msg:partial-before-error");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(partial_ref.clone());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
            assert_eq!(failed.explanation_message_refs, vec![partial_ref]);
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 2);
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test(start_paused = true)]
async fn model_error_abort_skips_explanation_and_carries_partial_refs() {
    // Availability-class model errors retry on the deep availability budget
    // before aborting; paused time fast-forwards the backoff sleeps.
    let abort_call_count = crate::strategies::DefaultRecoveryStrategy::default()
        .max_model_availability_attempts as usize
        + 1;
    let script = ScenarioScript {
        model_responses: (0..abort_call_count)
            .map(|_| ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Internal,
            })
            .collect(),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let partial_ref = message_ref("msg:model-partial");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(partial_ref.clone());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
            assert_eq!(failed.explanation_message_refs, vec![partial_ref]);
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(
        host.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::StreamModel))
            .count(),
        abort_call_count
    );
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test(start_paused = true)]
async fn availability_budget_above_old_executor_guard_reaches_strategy_abort() {
    // Regression pin: the model stage's retry guard is derived from the
    // composed recovery strategy, not a hard-coded executor constant. A
    // configured availability budget larger than the old `MAX_MODEL_RETRIES`
    // (16) must still reach the strategy's own Abort — with its failure
    // category and diagnostics — instead of falling through the loop to a
    // diagnostic-free generic ModelError exit.
    let attempts = 20u32;
    let family = crate::families::default_with_overrides(
        crate::families::FamilyOverrides::default().set_model_availability_attempts(attempts),
    );
    let diagnostic_ref = LoopDiagnosticRef::new("diag:model-outage").expect("valid");
    // Exactly attempts+1 scripted failures: the final one is the call where
    // the strategy's budget is exhausted and it aborts. Any extra call would
    // hit the mock's script-exhausted Internal fallback and change the
    // observed failure category.
    let host = MockHost::new(Vec::new()).with_model_errors(
        (0..=attempts)
            .map(|_| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable")
                    .with_diagnostic_ref(diagnostic_ref.clone())
            })
            .collect(),
    );
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
            assert_eq!(
                failed
                    .safe_summary
                    .as_ref()
                    .map(|summary| summary.category()),
                Some("model_unavailable"),
                "abort must carry the strategy's failure category"
            );
            assert_eq!(
                failed.diagnostic_ref,
                Some(diagnostic_ref),
                "abort must carry the model error's diagnostic ref"
            );
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        (attempts + 1) as usize,
        "the loop must allow exactly the configured availability budget"
    );
}

#[tokio::test]
async fn cancellation_before_explanation_skips_explanation_model_call() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "This explanation should not be requested.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::failed("permanent")]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder()
        .script(script)
        .cancel_after_capability_batch(LoopCancellationSignal {
            reason_kind: LoopCancelReasonKind::UserRequested,
            requested_at: chrono::Utc::now(),
        })
        .build();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(host.model_call_count(), 1);
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test]
async fn exit_stage_aborted_cancellation_skips_explanation_and_returns_cancelled() {
    let host = MockHost::new(vec![reply_response()]);
    host.request_cancellation(LoopCancelReasonKind::UserRequested);
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
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
            },
        )
        .await
        .expect("exit stage");

    match exit {
        LoopExit::Cancelled(cancelled) => {
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
        }
        other => panic!("expected cancelled exit, got {other:?}"),
    }
    assert!(host.prompt_requests().is_empty());
    assert!(host.model_requests().is_empty());
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn cancellation_during_explanation_model_call_propagates_cancelled() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Cancelled,
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::failed("permanent")]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = executor
        .execute_family(&crate::families::default(), &host, state)
        .await;

    assert!(
        matches!(result, Err(AgentLoopExecutorError::Cancelled)),
        "in-flight cancellation during the explanation call must propagate as Cancelled, not produce a Failed exit: {result:?}"
    );
    assert!(host.finalized_assistant_messages().is_empty());
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:visible").expect("valid"),
                "visible call completed".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
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
            .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    error_kind,
                    "temporary failure".to_string(),
                    None,
                )],
                stopped_on_suspension: false,
            }])
            .with_single_outcomes(vec![resolution::completed(
                LoopResultRef::new("result:retry").expect("valid"),
                "retry completed".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::denied(
                    ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    "provider call denied".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        }])
        .with_single_outcomes(vec![resolution::completed(
            LoopResultRef::new("result:policy-retry").expect("valid"),
            "policy retry completed".to_string(),
            ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            true,
            0,
            None,
            None,
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::spawned_process(
                LoopProcessRef::new("process:alpha").expect("valid"),
            )],
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::spawned_child_run(
                TurnRunId::new(),
                result_ref,
                "spawned child completed".to_string(),
                0,
                None,
            )],
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

/// Post-§5.3 Stage 2 flip: an unsafe strategy safe_summary no longer terminates
/// the run. The mapping redacts it to the `SafeSummary::placeholder()` value
/// (`safe_summary_or_placeholder`) before it reaches the model, so the unsafe
/// content (here a filesystem path) still never reaches the model — but the run
/// continues, appending a result whose summary is the redacted placeholder
/// rather than erroring. The redaction guarantee is met by the placeholder.
#[tokio::test]
async fn spawned_child_run_redacts_unsafe_safe_summary_to_placeholder() {
    let result_ref = LoopResultRef::new("result:spawned-child").expect("valid");
    // A second (reply) turn lets the run complete after the SpawnedChildRun
    // result is appended with the redacted summary.
    let host = MockHost::new(vec![calls_response(), reply_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::spawned_child_run(
                TurnRunId::new(),
                result_ref,
                "/Users/alice/.ssh/id_rsa".to_string(),
                0,
                None,
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("unsafe summary is redacted to a placeholder, not terminated");

    assert!(matches!(exit, LoopExit::Completed(_)));
    // The result is appended, but with the redacted placeholder summary — the
    // original unsafe path never reaches the model.
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(
        appended[0].safe_summary,
        ironclaw_host_api::SafeSummary::placeholder().as_str()
    );
    assert_ne!(appended[0].safe_summary, "/Users/alice/.ssh/id_rsa");
}

#[tokio::test]
async fn completed_provider_call_appends_provider_replay_metadata() {
    let result_ref = LoopResultRef::new("result:provider-call").expect("valid");
    let safe_summary = "a".repeat(300);
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                safe_summary.clone(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
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
    assert_eq!(provider_call.replay.provider_turn_id, "turn_1");
    assert_eq!(provider_call.replay.provider_call_id, "call_1");
    assert_eq!(
        provider_call.replay.provider_tool_name.as_str(),
        "demo__echo"
    );
    assert_eq!(provider_call.capability_id, capability_id());
    assert_eq!(
        provider_call.replay.arguments,
        serde_json::json!({"message":"hello"})
    );
    assert_eq!(
        provider_call.replay.response_reasoning.as_deref(),
        Some("response reasoning")
    );
    assert_eq!(
        provider_call.replay.reasoning.as_deref(),
        Some("call reasoning")
    );
    assert_eq!(provider_call.replay.signature.as_deref(), Some("sig-1"));
    let model_observation = appended[0]
        .model_observation
        .as_ref()
        .expect("model-visible observation");
    assert_eq!(
        model_observation.schema_version,
        MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION
    );
    assert_eq!(model_observation.status, ToolObservationStatus::Success);
    assert_eq!(model_observation.summary, safe_summary);
    assert!(matches!(
        &model_observation.detail,
        ToolObservationDetail::ResultReference {
            result_ref: observed_ref,
            byte_len: 0,
            preview: None,
            total_bytes: None,
            next_offset: None,
            item_count: None,
        } if observed_ref == result_ref.as_str()
    ));
    assert!(model_observation.artifacts.is_empty());
    assert!(model_observation.recovery.is_none());
    assert_eq!(
        model_observation.trust,
        ObservationTrust::UntrustedToolOutput
    );
}

#[tokio::test]
async fn denied_provider_call_appends_failure_tool_result_for_replay() {
    let result_ref = LoopResultRef::new("result:provider-call").expect("valid");
    let host = MockHost::new(vec![provider_two_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    result_ref.clone(),
                    "provider call completed".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    true,
                    0,
                    None,
                    None,
                ),
                resolution::denied(
                    ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    "provider call denied".to_string(),
                )
                .resolution,
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
    // Post-§5.3 Stage 2 flip: deny reasons map to the closed `DenyReason` set.
    // The loop reason `empty_surface` is not a `DenyReason` variant, so it maps
    // to `policy_denied` via the `deny_reason_from_kind` fallback.
    assert_eq!(
        appended[1].safe_summary,
        "capability denied with policy_denied: provider call denied"
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
    assert_eq!(denied_provider_call.replay.provider_turn_id, "turn_1");
    assert_eq!(denied_provider_call.replay.provider_call_id, "call_2");
    assert_eq!(
        denied_provider_call.replay.provider_tool_name.as_str(),
        "demo__echo"
    );
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::InvalidInput,
                "provider arguments failed schema validation".to_string(),
                Some(CapabilityFailureDetail::InvalidInput {
                    issues: vec![CapabilityInputIssue {
                        path: "file_path".to_string(),
                        code: DispatchInputIssueCode::MissingRequired,
                        expected: Some("required field".to_string()),
                        received: None,
                        schema_path: Some("required".to_string()),
                    }],
                }),
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
            assert_eq!(issues[0].code, DispatchInputIssueCode::MissingRequired);
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
async fn repeated_capability_failures_do_not_trip_no_progress_and_run_can_recover() {
    // PR3: Blocked/failed tool calls are NOT counted as no-progress — failures
    // route through recovery and are bounded by the budget/iteration limit, not
    // the no-progress escape. Three failed batches do not fire NoProgressDetected;
    // once the model recovers with a reply the run completes normally, and the
    // tool-error results stay in the transcript (work isn't lost).
    let host = MockHost::new(vec![
        provider_calls_response(),
        provider_calls_response(),
        provider_calls_response(),
        reply_response(),
    ])
    .with_batch_outcomes(
        (0..3)
            .map(|_| ironclaw_host_api::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    CapabilityFailureKind::OperationFailed,
                    "filesystem discovery failed".to_string(),
                    None,
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
        other => panic!("expected recovered completion after blocked failures, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        4,
        "blocked failures must not fire no-progress or an explanation failure call before recovered reply"
    );
    assert_eq!(host.batch_invocations().len(), 3);
    assert_eq!(host.appended_result_refs().len(), 3);
    assert_eq!(
        final_staged_state(&host)
            .stop_state
            .trailing_no_progress_results,
        0,
        "Blocked/failed results must not count toward the no-progress escape"
    );
}

#[tokio::test]
async fn repeated_multi_call_failures_do_not_trip_no_progress_and_run_can_recover() {
    // PR3 (multi-call variant): batches where every call fails are not counted as
    // no-progress; the run recovers and completes once the model replies.
    let host = MockHost::new(vec![
        provider_two_calls_response(),
        provider_two_calls_response(),
        provider_two_calls_response(),
        reply_response(),
    ])
    .with_batch_outcomes(
        (0..3)
            .map(|_| ironclaw_host_api::ResolutionBatch {
                resolutions: vec![
                    resolution::failed(
                        CapabilityFailureKind::OperationFailed,
                        "first discovery failed".to_string(),
                        None,
                    ),
                    resolution::failed(
                        CapabilityFailureKind::OperationFailed,
                        "second discovery failed".to_string(),
                        None,
                    ),
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
        other => {
            panic!("expected recovered completion after multi-call blocked failures, got {other:?}")
        }
    }
    assert_eq!(
        host.model_requests().len(),
        4,
        "multi-call blocked failures must not fire no-progress or an explanation failure call before recovered reply"
    );
    assert_eq!(host.batch_invocations().len(), 3);
    assert_eq!(host.appended_result_refs().len(), 6);
    assert_eq!(
        final_staged_state(&host)
            .stop_state
            .trailing_no_progress_results,
        0,
        "Blocked/failed results must not count toward the no-progress escape"
    );
}

#[tokio::test]
async fn completed_output_digest_is_recorded_into_seen_capability_output_digests() {
    // PR2 plumbing: the executor must record a completed result's `output_digest`
    // into the checkpointed `seen_capability_output_digests` ring. Asserted through
    // the executor (not the state helper) so the single production wiring line in
    // `append_completed_capability_result` cannot silently regress while it is still
    // inert — nothing reads the ring until output-aware detection lands in a later
    // change, so a behavior-only test would stay green even if the push were removed.
    let digest = ironclaw_turns::run_profile::ContentDigest(4242);
    let result_ref = LoopResultRef::new("result:digest-recorded").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "completed with digest".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                Some(digest),
                None,
            )],
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

    let recorded: Vec<_> = final_staged_state(&host)
        .seen_capability_output_digests
        .iter()
        .map(|observation| observation.output_digest)
        .collect();
    assert!(
        recorded.contains(&digest),
        "executor must record the completed result's output_digest into the ring; got {recorded:?}"
    );
}

#[tokio::test]
async fn repeated_non_provider_replayable_failures_do_not_trigger_no_progress_stop() {
    let host = MockHost::new(vec![calls_response(), calls_response(), calls_response()])
        .with_batch_outcomes(
            (0..3)
                .map(|_| ironclaw_host_api::ResolutionBatch {
                    resolutions: vec![resolution::failed(
                        CapabilityFailureKind::OperationFailed,
                        "non-replayable capability failed".to_string(),
                        None,
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
    assert_eq!(
        host.model_requests().len(),
        4,
        "iteration limit should attempt one best-effort explanation call"
    );
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
            .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    error_kind,
                    safe_summary.to_string(),
                    None,
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
        assert_eq!(provider_call.replay.provider_turn_id, "turn_1");
        assert_eq!(provider_call.replay.provider_call_id, "call_1");
        assert_eq!(
            provider_call.replay.provider_tool_name.as_str(),
            "demo__echo"
        );
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::OutputTooLarge,
                long_summary,
                None,
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_) => {
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big").expect("valid"),
                "big result".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                33_001,
                None,
                None,
            )],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:small").expect("valid"),
                "small result".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                100,
                None,
                None,
            )],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big-no-term").expect("valid"),
                "big result no terminate".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                33_001,
                None,
                None,
            )],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    true,
                    20_000,
                    None,
                    None,
                ),
                resolution::completed(
                    LoopResultRef::new("result:second").expect("valid"),
                    "second".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    true,
                    20_000,
                    None,
                    None,
                ),
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

/// Post-§5.3 Stage 2 flip: the structured `ModelVisibleToolObservation` no
/// longer rides the channel for an AwaitDependentRun outcome. The mapping
/// collapses it to a `SafeSummary` preview and the executor sets
/// `model_observation: None`; `append_capability_result_ref` then re-synthesizes
/// a Success observation FROM the summary. So the appended result no longer
/// carries the exact structured `ResultReference` observation the fixture
/// scripted — it carries a synthesized success observation whose summary is the
/// AwaitDependentRun safe_summary. This asserts that collapse, not the old
/// structured-object equality.
#[tokio::test]
async fn await_dependent_run_preserves_model_observation_for_replay() {
    let result_ref =
        LoopResultRef::new("result:await-dependent-preserved-observation").expect("valid");
    // The structured observation still travels on the outcome, but the mapping
    // drops it in favor of the safe_summary preview (below).
    let observation = continuation_observation(&result_ref, 4_096);
    let awaited_summary = "awaited child completed".to_string();
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::await_dependent_run(
                    LoopGateRef::new("gate:await-dependent-preserved-observation").expect("valid"),
                    result_ref,
                    awaited_summary.clone(),
                    4_096,
                    Some(observation),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &crate::families::default(),
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Blocked(_)));
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    // The appended result carries the safe_summary preview...
    assert_eq!(appended[0].safe_summary, awaited_summary);
    // ...and the child's staged observation caption is forwarded (#6287 IronLoop):
    // a Success `ResultReference` observation carrying the caption as its summary
    // and pointing at the staged child result, so the resumed parent can
    // `result_read` it — NOT the bare synthesized success observation the append
    // path falls back to when the consumer drops the observation.
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("the forwarded child observation caption must survive to the parent");
    assert_eq!(observation.status, ToolObservationStatus::Success);
    assert_eq!(
        observation.summary, "Use result_read to continue this child result.",
        "the forwarded observation summary is the staged caption, not the bare safe_summary"
    );
    match &observation.detail {
        ToolObservationDetail::ResultReference {
            result_ref,
            byte_len,
            preview,
            ..
        } => {
            assert_eq!(result_ref, "result:await-dependent-preserved-observation");
            assert_eq!(*byte_len, 4_096);
            // The full inline first-look preview content stays host-owned this
            // stage; only the caption + staged result ref are forwarded.
            assert!(preview.is_none());
        }
        other => panic!("expected a forwarded ResultReference observation, got {other:?}"),
    }
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::spawned_child_run(
                TurnRunId::new(),
                LoopResultRef::new("result:spawned-child-large").expect("valid"),
                "spawned child with large result".to_string(),
                49_001,
                None,
            )],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::await_dependent_run(
                    LoopGateRef::new("gate:await-large").expect("valid"),
                    LoopResultRef::new("result:await-large").expect("valid"),
                    "await dependent run with large result".to_string(),
                    33_001,
                    None,
                )
                .resolution,
            ],
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
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big-f12").expect("valid"),
                "big result for F12".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                33_001,
                None,
                None,
            )],
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

#[tokio::test]
async fn executor_continues_after_forced_compaction_rejection_from_tool_result_overflow() {
    let host = MockHost::new(vec![
        calls_response(),
        reply_response_with_text("final answer"),
    ])
    .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
        resolutions: vec![resolution::completed(
            LoopResultRef::new("result:big-compaction-rejected").expect("valid"),
            "large search result".to_string(),
            ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            false,
            33_001,
            None,
            None,
        )],
        stopped_on_suspension: false,
    }])
    .with_prompt_compaction_indexes(vec![active_task_preserving_compaction_index(), vec![]])
    .with_compaction_outcome(Err(LoopCompactionError::SecurityRejected {
        safe_summary: LoopSafeSummary::new("injection detected").unwrap(),
    }));

    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "compaction rejection after successful tool execution must not fail the run: {exit:?}"
    );
    assert_eq!(
        host.model_requests().len(),
        2,
        "the loop should continue to the post-tool reply model turn"
    );
    let progress_events = host.progress_event_names();
    assert!(
        progress_events.contains(&"compaction_failed"),
        "the failed compaction should still be reported: {progress_events:?}"
    );
    assert!(
        host.finalized_assistant_messages()
            .iter()
            .any(|message| message.contains("final answer")),
        "the post-tool assistant reply should still finalize"
    );
}

/// `GateOutcome::validate_for_gate_kind` is the owning contract for every gate
/// stage. A custom strategy cannot use `SkipAndContinue` to discard an
/// `AwaitDependentRun` suspension and report a normal completion; that would
/// orphan the child-run relationship and hide a planner bug.
#[tokio::test]
async fn await_dependent_run_gate_skip_and_continue_fails_as_driver_bug() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::await_dependent_run(
                    LoopGateRef::new("gate:await-skip").expect("valid"),
                    LoopResultRef::new("result:await-skip").expect("valid"),
                    "dependent run skip and continue".to_string(),
                    33_001,
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::DriverBug);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("invalid AwaitDependentRun skip must fail as DriverBug, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "a driver contract violation must stop before another model turn"
    );
}

/// An external-tool suspension cannot be silently discarded by a custom gate
/// strategy. Drive the full executor so the assertion covers the GateStage
/// caller path, not only `GateOutcome::validate_for_gate_kind`.
#[tokio::test]
async fn external_tool_gate_skip_and_continue_fails_as_driver_bug() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    LoopGateRef::new("gate:external-tool-skip").expect("valid"),
                    "external tool skip and continue".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::DriverBug);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("invalid ExternalTool skip must fail as DriverBug, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "a driver contract violation must stop before another model turn"
    );
}

#[tokio::test]
async fn auth_gate_block_stores_pending_auth_resume() {
    // Drive the full executor loop so the GateStage block arm runs through the
    // canonical path (cancel-check → progress emit → write_before_block).
    let gate_ref = LoopGateRef::new("gate:auth-block").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
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
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    approval_gate_ref.clone(),
                    "approval required during redispatch".to_string(),
                    None,
                )
                .resolution,
            ],
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
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
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
async fn external_tool_gate_block_stores_pending_external_tool_resume() {
    // Driving the full loop, an ExternalToolPending outcome must block the run
    // and checkpoint a pending_external_tool_resume record (so resume can
    // re-dispatch the parked client-tool call).
    let gate_ref = LoopGateRef::new("gate:external_tool-block").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    gate_ref.clone(),
                    "awaiting client tool output".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on external tool gate");
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit for external tool gate, got {exit:?}"
    );

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending = before_block_state
        .pending_external_tool_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_external_tool_resume");
    assert_eq!(pending.gate_ref, gate_ref);
    assert_eq!(pending.capability_id, capability_id());
    // External-tool blocks must not touch the auth/approval slots.
    assert!(before_block_state.pending_auth_resume.is_none());
    assert!(before_block_state.pending_approval_resume.is_none());
}

#[tokio::test]
async fn parallel_batch_records_completed_results_before_external_tool_block() {
    let completed_ref = LoopResultRef::new("result:parallel-external-completed").expect("valid");
    let external_gate_ref = LoopGateRef::new("gate:external-tool-parallel").expect("valid");
    let host = MockHost::new(vec![two_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    completed_ref.clone(),
                    "parallel call completed".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::external_tool_pending(
                    external_gate_ref.clone(),
                    "awaiting client tool output".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on external tool gate");

    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit for external tool gate, got {exit:?}"
    );
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].result_ref, completed_ref);
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert_eq!(before_block_state.result_refs, vec![completed_ref]);
    let pending = before_block_state
        .pending_external_tool_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_external_tool_resume");
    assert_eq!(pending.gate_ref, external_gate_ref);
}

#[tokio::test]
async fn resume_after_external_tool_gate_redispatches_without_model_turn() {
    // Phase 1: the client tool call parks (ExternalToolPending). Phase 2: resume
    // re-dispatches the parked call WITHOUT a model turn, restaging the provider
    // tool call so the host decorator can complete it from the catalog output.
    let gate_ref = LoopGateRef::new("gate:external_tool-resume").expect("valid");
    let completed_ref = LoopResultRef::new("result:external-tool-resumed").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    gate_ref.clone(),
                    "awaiting client tool output".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "external tool output".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on external tool gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    assert_eq!(host.model_requests().len(), 1);

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_external_tool_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_external_tool_resume"
    );

    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes from external tool gate");
    assert!(
        matches!(second_exit, LoopExit::Completed(_)),
        "expected Completed exit after external tool resume, got {second_exit:?}"
    );

    // No additional model call: the parked call is re-dispatched before the model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "external tool resume must re-dispatch without a model call"
    );
    // Two batch invocations: phase 1 (park) + phase 2 (complete).
    assert_eq!(host.batch_invocations().len(), 2);
    // Resume re-registered the provider tool call so the decorator re-binds and
    // completes from the catalog.
    assert_eq!(
        host.registered_provider_calls().len(),
        1,
        "external tool resume must restage exactly one provider tool call"
    );
}

#[tokio::test]
async fn resume_after_auth_gate_redispatches_original_call_without_model_turn() {
    // Phase 1: executor blocks on an auth gate and writes a BeforeBlock checkpoint
    // that carries a pending_auth_resume record with the original input_ref.
    let gate_ref = LoopGateRef::new("gate:auth-resume-test").expect("valid");
    let completed_ref = LoopResultRef::new("result:auth-resumed").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: the auth is now satisfied, call completes.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "auth resumed and completed".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
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
    let parked_activity_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set")
        .activity_id_for_resume();
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
    assert_eq!(
        batch_invocations[0].invocations[0].activity_id, parked_activity_id,
        "auth gate must park the original provider activity identity"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].activity_id, parked_activity_id,
        "provider-backed auth resume must re-dispatch with the parked activity identity"
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
        registered_provider_calls[0].name.as_str(),
        "demo__echo",
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
            ironclaw_host_api::ResolutionBatch {
                resolutions: vec![
                    resolution::auth_required(
                        gate_ref.clone(),
                        Vec::new(),
                        "auth required".to_string(),
                        None,
                    )
                    .resolution,
                ],
                stopped_on_suspension: true,
            },
            ironclaw_host_api::ResolutionBatch {
                resolutions: vec![resolution::completed(
                    completed_ref,
                    "should not invoke".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    true,
                    0,
                    None,
                    None,
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
async fn auth_resume_provider_activity_remap_fails_before_invocation() {
    let gate_ref = LoopGateRef::new("gate:auth-resume-activity-remap").expect("valid");
    let completed_ref = LoopResultRef::new("result:unused-auth-resume-remap").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref,
                "should not invoke".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        },
    ]);
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
    let parked_activity_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("auth resume checkpointed")
        .activity_id_for_resume();
    let remapped_activity_id = loop {
        let candidate = CapabilityActivityId::new();
        if candidate != parked_activity_id {
            break candidate;
        }
    };
    host.set_provider_registration_activity_remap(remapped_activity_id);

    let error = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect_err("provider activity remap should fail auth resume");

    assert!(
        matches!(
            error,
            AgentLoopExecutorError::PlannerContract { detail }
                if detail.contains("provider replay no longer matches")
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        host.registered_provider_calls().len(),
        1,
        "phase 2 should restage the provider call before rejecting identity drift"
    );
    assert_eq!(
        host.batch_invocations().len(),
        1,
        "phase 2 must fail before invoking the remapped resumed capability"
    );
}

#[tokio::test]
async fn resume_with_still_missing_credentials_blocks_again_without_model_turn() {
    // Phase 1: scripted AuthRequired -> executor exits Blocked and writes a
    // BeforeBlock checkpoint carrying a pending_auth_resume record.
    let gate_ref = LoopGateRef::new("gate:auth-still-missing").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required (phase 1)".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: credentials are STILL missing — block again.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:auth-still-missing-2").expect("valid"),
                    Vec::new(),
                    "auth required (phase 2 — still missing)".to_string(),
                    None,
                )
                .resolution,
            ],
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
    // → re-dispatch blocks again → GateStage runs → planner returns
    // SkipAndContinue. Without the fix, pending_auth_resume for A survives, and
    // the next prompt iteration re-dispatches A again — potential infinite
    // re-dispatch loop with no model turn.
    //
    // Driven through an Auth gate: SkipAndContinue is only a valid outcome for
    // Auth/Resource gates now that GateStage enforces
    // `GateOutcome::validate_for_gate_kind` (§5a.1) — an Approval-gate skip
    // fails the run as DriverBug (covered by the executor failure matrix).
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
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-skip-stale").expect("valid");

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
                auth_resume: None,
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
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
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
                auth_resume: None,
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
    // Driven through an Auth gate (a valid skip kind) since GateStage now
    // enforces `GateOutcome::validate_for_gate_kind` (§5a.1).
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
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    // The call being dispatched through GateStage is capability_id() ("demo.echo"),
    // not the seeded "other.cap".
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-skip-other").expect("valid");

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
                auth_resume: None,
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
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks with resume metadata
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{approval_request_id}"))
                        .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    auth_gate_ref.clone(),
                    Vec::new(),
                    "auth required after approval".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "completed after auth resume".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
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
    assert!(
        phase2_bb.pending_approval_resume.is_none(),
        "phase 2 auth gate must fold prior approval into pending_auth_resume and clear pending_approval_resume"
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
        phase3_auth_resume.resume_token.as_ref(),
        Some(&resume_token),
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
    // correlation_id is observability-only post-§5.3 Stage 2 flip: it is
    // regenerated when the approval identity is reconstructed from the gate ref,
    // so prior_approval.correlation_id is NOT byte-stable with the original (the
    // authoritative correlation is reconstituted host-side from the replay
    // payload, §5.3 Stage 2a-ii). The dedicated correlation axis lives in
    // `auth_resume_after_approval_carries_original_correlation_id`; here we only
    // assert prior_approval is present and carries a correlation_id.
    let _ = phase3_pa.correlation_id;

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

/// Verify that `pending_auth_resume.prior_approval` is present and carries a
/// correlation_id throughout the approval → auth-block → auth-resume pipeline.
///
/// Post-§5.3 Stage 2 flip, correlation_id is observability-only: the approval
/// identity is reconstructed from the `gate:approval-{id}` ref and a fresh
/// correlation_id is minted at the executor boundary, so prior_approval's
/// correlation_id is NOT byte-stable with the approval's original one. The
/// authoritative correlation is reconstituted host-side from the replay payload
/// (§5.3 Stage 2a-ii). This test therefore asserts the weaker present-and-valid
/// contract on prior_approval; the byte-stable fields (request id, resume token)
/// are covered by `auth_resume_after_approval_carries_resume_token_and_approval_request_id`.
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
    let auth_gate_resume_token =
        CapabilityResumeToken::new("resume-token:corr-id-auth-gate").expect("valid token");
    let correlation_id = CorrelationId::new();
    let original_input_ref = CapabilityInputRef::new("input:corr-id-original").expect("valid");
    let completed_ref = LoopResultRef::new("result:corr-id-done").expect("valid");

    let approval_resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: resume_token.clone(),
        correlation_id,
        input_ref: original_input_ref.clone(),
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{approval_request_id}"))
                        .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:corr-id-auth").expect("valid"),
                    Vec::new(),
                    "auth required".to_string(),
                    Some(CapabilityAuthResume {
                        resume_token: Some(auth_gate_resume_token),
                        disposition: None,
                        prior_approval: None,
                    }),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                true,
                0,
                None,
                None,
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
    assert_eq!(
        pending_auth.resume_token.as_ref(),
        Some(&resume_token),
        "pending_auth_resume.resume_token must preserve the original approval invocation token"
    );
    let pending_pa = pending_auth
        .prior_approval
        .as_ref()
        .expect("pending_auth_resume.prior_approval must be set when approval preceded auth");
    // correlation_id is observability-only and regenerated at the executor
    // boundary post-flip, so it is NOT equal to the approval's original. Assert
    // prior_approval is present and its request id is byte-stable instead.
    assert_eq!(
        pending_pa.approval_request_id, approval_request_id,
        "pending_auth_resume.prior_approval.approval_request_id must equal the approval's request id"
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
    assert_eq!(
        phase3_ar.resume_token.as_ref(),
        Some(&resume_token),
        "phase 3 auth_resume.resume_token must preserve the original approval invocation token"
    );
    let phase3_pa = phase3_ar
        .prior_approval
        .as_ref()
        .expect("phase 3 auth_resume.prior_approval must be set");
    // correlation_id is observability-only post-flip and regenerated, so it does
    // NOT match the original; assert the byte-stable request id instead.
    assert_eq!(
        phase3_pa.approval_request_id, approval_request_id,
        "phase 3 auth_resume.prior_approval.approval_request_id must match the original approval request id"
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
    let host =
        MockHost::new(Vec::new()).with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::completed(
                    LoopResultRef::new("result:second").expect("valid"),
                    "second done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

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
        activity_id: CapabilityActivityId::new(),
        prior_approval: Some(crate::state::AuthResumeApprovalIdentity {
            approval_request_id,
            correlation_id,
        }),
        disposition: None,
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
        first_auth.resume_token.as_ref(),
        Some(&resume_token),
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

// ── Resume-origin Backend failure must not die as scope_mismatch ─────────────

/// Regression test for the terminal `scope_mismatch` / `HostUnavailable` failure
/// that surfaces when a capability's approval-resume dispatch returns a transient
/// `Backend` error.
///
/// # Bug (capabilities.rs, pre-fix)
///
/// 1. Executor dispatches the capability with `approval_resume` set (batch path).
/// 2. Host returns `Failed(Backend)`.
/// 3. `handle_capability_error` clears `state.pending_approval_resume` BEFORE
///    asking the planner for a recovery outcome.
/// 4. Planner returns `RecoveryOutcome::Retry`.
/// 5. Retry dispatch calls `invoke_capability(…, None)` — `approval_resume` is
///    dropped.
/// 6. MockHost `invoke_capability` has no scripted outcome → returns
///    `Err(Internal, "single script exhausted")` → `capability_host_error` →
///    `AgentLoopExecutorError::HostUnavailable { stage: Capability }`.
///    In production the host would instead fail with `ScopeMismatch` because the
///    original run's `input_ref` has no approval context to validate against.
///
/// # Fix (Part C-sub-A)
///
/// When the failure originated from an approval-resume dispatch, intercept
/// `RecoveryOutcome::Retry` and redirect it to `ToolErrorResult` instead.
/// The model sees the real backend error and the user can re-approve — no retry
/// of the side effect, no scope_mismatch.
///
/// # What this test asserts (observable, not implementation detail)
///
/// - **Pre-fix (RED)**: `execute_family` on Phase 2 returns
///   `Err(AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })`
///   — the run terminally dies.
/// - **Post-fix (GREEN)**: `execute_family` on Phase 2 returns `Ok(LoopExit::Completed(_))`
///   — the model sees the backend error as a tool result, issues a final reply,
///   and the run completes cleanly.
#[tokio::test]
async fn resume_origin_backend_failure_does_not_die_as_scope_mismatch() {
    let cap1_request_id = ApprovalRequestId::new();
    let cap1_resume_token =
        CapabilityResumeToken::new("resume-token:sm-test-cap1").expect("valid token");
    let cap1_correlation_id = CorrelationId::new();
    let cap1_input_ref =
        CapabilityInputRef::new("input:run-original:sm-cap1-uuid").expect("valid input ref");

    let cap1_approval_resume = CapabilityApprovalResume {
        approval_request_id: cap1_request_id,
        resume_token: cap1_resume_token,
        correlation_id: cap1_correlation_id,
        input_ref: cap1_input_ref.clone(),
    };

    // Phase 1 model response: issues cap1 with original-run input_ref.
    let cap1_model_response = ironclaw_turns::run_profile::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: cap1_input_ref,
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    };

    // Batch outcomes:
    //   [0] Phase 1: cap1 → ApprovalRequired → gate blocked.
    //   [1] Phase 2: cap1 approval-resume → Failed(Backend) — the bug trigger.
    let batch_outcomes = vec![
        // [0] cap1 → ApprovalRequired (gate blocked).
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{cap1_request_id}")).expect("valid"),
                    "cap1 needs approval".to_string(),
                    Some(cap1_approval_resume),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // [1] cap1 approval-resume → Backend failure.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::Backend,
                "transient backend error during cap1 resume".to_string(),
                None,
            )],
            stopped_on_suspension: false,
        },
    ];

    // Phase 1: one model turn (cap1); Phase 2: after Backend→ToolErrorResult
    // the loop continues and needs a model turn to issue the final reply.
    let host = MockHost::new(vec![
        cap1_model_response, // Phase 1: issues cap1
        reply_response(),    // Phase 2 (post-fix): model sees tool error, issues reply
    ])
    .with_batch_outcomes(batch_outcomes);
    // Deliberately NO single_outcomes: pre-fix, the retry would consume one and
    // get `Err(Internal)` → HostUnavailable.  Post-fix, no retry is attempted.

    let executor = CanonicalAgentLoopExecutor;

    // ── Phase 1: cap1 → ApprovalRequired → Blocked ───────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must succeed (blocks on cap1 gate)");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "phase 1 must block on cap1 approval gate; got {phase1_exit:?}"
    );

    // Recover the BeforeBlock checkpoint state to use as Phase 2 input.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        phase1_bb.pending_approval_resume.is_some(),
        "phase 1 BeforeBlock must carry pending_approval_resume"
    );
    assert_eq!(
        phase1_bb
            .pending_approval_resume
            .as_ref()
            .unwrap()
            .approval_request_id,
        cap1_request_id,
        "phase 1 BeforeBlock pending_approval_resume must be for cap1"
    );

    // ── Phase 2: approve cap1 → approval-resume → Backend failure ─────────────
    //
    // BUG TRIGGER: the batch returns Failed(Backend) for the approval-resume
    // dispatch.  On unfixed code:
    //   handle_capability_error clears pending_approval_resume at L637 BEFORE
    //   recovery decides → retry fires with None → invoke_capability has no
    //   scripted outcome → Err(Internal) → HostUnavailable.
    //
    // After fix (Part C-sub-A):
    //   Resume-origin Backend failure is intercepted before any retry → surfaced
    //   as ToolErrorResult → loop continues → model issues final reply → Done.
    let phase2_result = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await;

    // Primary assertion: the run must NOT die as HostUnavailable.
    // Pre-fix this panics; post-fix it passes.
    let phase2_exit = phase2_result.expect(
        "REGRESSION: resume-origin Backend failure must not kill the run as HostUnavailable. \
         Pre-fix: handle_capability_error clears pending_approval_resume BEFORE recovery \
         decides to retry → retry fires invoke_capability with approval_resume=None → \
         single script exhausted → HostUnavailable (in production: ScopeMismatch). \
         Fix: intercept Retry for resume-origin failures and surface as ToolErrorResult \
         so the model can re-approve without a terminal run death.",
    );

    // Post-fix: the model saw the tool error, issued a final reply → Completed.
    assert!(
        matches!(phase2_exit, LoopExit::Completed(_)),
        "phase 2 must complete the run after Backend→ToolErrorResult; got {phase2_exit:?}"
    );

    // No single invoke_capability calls should have been made: the C-sub-A guard
    // prevents the retry dispatch entirely for resume-origin failures.
    assert!(
        host.single_invocations().is_empty(),
        "no single invoke_capability call must be made for a resume-origin Backend failure \
         (retry is suppressed to avoid double-exec)"
    );
}

/// Regression test for the terminal `scope_mismatch` / `HostUnavailable` failure
/// that surfaces when a capability's **auth-resume** dispatch returns a transient
/// `Backend` error.
///
/// # Bug (capabilities.rs, pre-fix)
///
/// 1. Phase 1: executor dispatches the capability; host returns `AuthRequired`.
///    GateStage stores `pending_auth_resume` in the BeforeBlock checkpoint.
/// 2. Phase 2: executor detects `pending_auth_resume` in the prompt stage,
///    re-dispatches the capability via `invoke_capability_batch(auth_resume=…)`.
/// 3. Host returns `Failed(Backend)` for the re-dispatch.
/// 4. `handle_capability_error` clears `state.pending_auth_resume` at ~L667
///    BEFORE asking the planner for a recovery outcome.
/// 5. Planner returns `RecoveryOutcome::Retry`.
/// 6. Retry calls `invoke_capability(…)` (single, non-batch) with no auth context.
///    MockHost has no scripted single outcome → `Err(Internal, "single script
///    exhausted")` → `capability_host_error` → `HostUnavailable { stage: Capability }`.
///    In production the product adapter would instead fail with `ScopeMismatch`
///    because the original run's `input_ref` has no auth context to validate against.
///
/// # Fix (Part C-sub-A extended to auth-resume)
///
/// Before clearing `pending_auth_resume`, snapshot whether this failure is
/// auth-resume-origin (`captured_auth_resume_origin`).  When `is_resume_origin`
/// is true (either approval- or auth-resume origin), intercept
/// `RecoveryOutcome::Retry` and redirect it to `ToolErrorResult` instead.
/// The model sees the real backend error and the user can re-authenticate —
/// no retry of the side effect, no scope_mismatch.
///
/// # What this test asserts (observable, not implementation detail)
///
/// - **Pre-fix (RED)**: Phase 2 `execute_family` returns
///   `Err(AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })`
///   — the run terminally dies.
/// - **Post-fix (GREEN)**: Phase 2 returns `Ok(LoopExit::Completed(_))` — the
///   model sees the backend error as a tool result, issues a final reply, and
///   the run completes cleanly.
#[tokio::test]
async fn auth_resume_origin_backend_failure_does_not_die_as_scope_mismatch() {
    let cap1_input_ref =
        CapabilityInputRef::new("input:run-original:auth-sm-cap1-uuid").expect("valid input ref");

    // Phase 1 model response: issues cap1 with original-run input_ref.
    // (No provider_replay — this is a non-provider-backed auth resume, so
    // Phase 2 reuses the stored input_ref directly via
    // pending_auth_resume_staged_input_candidate.)
    let cap1_model_response = ironclaw_turns::run_profile::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: cap1_input_ref,
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    };

    // Batch outcomes:
    //   [0] Phase 1: cap1 → AuthRequired → gate blocked.
    //   [1] Phase 2: cap1 auth-resume → Failed(Backend) — the bug trigger.
    let batch_outcomes = vec![
        // [0] Phase 1: cap1 → AuthRequired (gate blocked).
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:auth-sm-test-cap1").expect("valid"),
                    Vec::new(),
                    "cap1 needs auth".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // [1] Phase 2: cap1 auth-resume → Backend failure.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::Backend,
                "transient backend error during cap1 auth-resume".to_string(),
                None,
            )],
            stopped_on_suspension: false,
        },
    ];

    // Phase 1: one model turn (cap1); Phase 2: after Backend→ToolErrorResult
    // the loop continues and needs a model turn to issue the final reply.
    // (Auth-resume Phase 2 skips the model for the capability re-dispatch but
    // needs a model call AFTER the error is surfaced for the final reply.)
    let host = MockHost::new(vec![
        cap1_model_response, // Phase 1: issues cap1
        reply_response(),    // Phase 2 (post-fix): model sees tool error, issues reply
    ])
    .with_batch_outcomes(batch_outcomes);
    // Deliberately NO single_outcomes: pre-fix, the retry would consume one and
    // get `Err(Internal)` → HostUnavailable.  Post-fix, no retry is attempted.

    let executor = CanonicalAgentLoopExecutor;

    // ── Phase 1: cap1 → AuthRequired → Blocked ──────────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must succeed (blocks on cap1 auth gate)");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "phase 1 must block on cap1 auth gate; got {phase1_exit:?}"
    );

    // Recover the BeforeBlock checkpoint state to use as Phase 2 input.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        phase1_bb.pending_auth_resume.is_some(),
        "phase 1 BeforeBlock must carry pending_auth_resume"
    );
    assert_eq!(
        phase1_bb
            .pending_auth_resume
            .as_ref()
            .unwrap()
            .capability_id,
        capability_id(),
        "phase 1 BeforeBlock pending_auth_resume must be for cap1"
    );

    // ── Phase 2: OAuth completes → auth-resume → Backend failure ────────────
    //
    // BUG TRIGGER: the batch returns Failed(Backend) for the auth-resume
    // dispatch.  On unfixed code:
    //   handle_capability_error clears pending_auth_resume at ~L667 BEFORE
    //   recovery decides → retry fires with None → invoke_capability has no
    //   scripted outcome → Err(Internal) → HostUnavailable.
    //
    // After fix (Part C-sub-A extended to auth-resume):
    //   Auth-resume-origin Backend failure is intercepted before any retry →
    //   surfaced as ToolErrorResult → loop continues → model issues final
    //   reply → Done.
    let phase2_result = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await;

    // Primary assertion: the run must NOT die as HostUnavailable.
    // Pre-fix this panics; post-fix it passes.
    let phase2_exit = phase2_result.expect(
        "REGRESSION: auth-resume-origin Backend failure must not kill the run as \
         HostUnavailable. Pre-fix: handle_capability_error clears pending_auth_resume \
         BEFORE recovery decides to retry → retry fires invoke_capability with \
         auth_resume=None → single script exhausted → HostUnavailable (in production: \
         ScopeMismatch). Fix: intercept Retry for auth-resume-origin failures and \
         surface as ToolErrorResult so the model can re-auth without a terminal run death.",
    );

    // Post-fix: the model saw the tool error, issued a final reply → Completed.
    assert!(
        matches!(phase2_exit, LoopExit::Completed(_)),
        "phase 2 must complete the run after Backend→ToolErrorResult; got {phase2_exit:?}"
    );

    // No single invoke_capability calls should have been made: the C-sub-A guard
    // prevents the retry dispatch entirely for auth-resume-origin failures.
    assert!(
        host.single_invocations().is_empty(),
        "no single invoke_capability call must be made for an auth-resume-origin Backend \
         failure (retry is suppressed to avoid double-exec)"
    );
}

/// Caller-level regression test for the approval-deny short-circuit in
/// `CapabilityStage::process`.
///
/// When a run resumes from a user-DENIED approval gate (i.e.
/// `pending_approval_resume` is set and `disposition = Some(Denied)`), the
/// executor must NOT re-dispatch the parked capability (re-dispatch → re-block
/// → infinite loop). It must:
///
/// 1. Return `TurnCompletedStep::Continue` (loop proceeds, not Blocked/Exit).
/// 2. Clear `pending_approval_resume` so the next iteration prompts the model
///    normally.
/// 3. Append a model-visible gate-declined failure observation with
///    `status = Error` and `same_call_retry = Forbidden` (via
///    `generic_failure_recovery` mapping).
/// 4. Issue zero batch-invoke calls to the capability host.
#[tokio::test]
async fn capability_stage_denied_approval_resume_surfaces_gate_declined_failure_and_continues() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let mut calls = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider calls fixture"),
    };
    let provider_replay = calls[0].provider_replay.clone();
    let denied_input_ref = calls[0].input_ref.clone();
    let denied_surface_version = calls[0].surface_version.clone();
    let denied_activity_id = CapabilityActivityId::new();
    calls[0].activity_id = denied_activity_id;

    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-test").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new(denied_activity_id.to_string())
            .expect("valid token"),
        activity_id: denied_activity_id,
        correlation_id: CorrelationId::new(),
        surface_version: denied_surface_version,
        input_ref: denied_input_ref,
        effective_capability_ids: vec![capability_id()],
        provider_replay,
        disposition: Some(GateResumeDisposition::Denied),
    });

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

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied approval resume, got Exit: {exit:?}")
        }
    };

    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared after surfacing the deny failure"
    );
    assert!(
        host.batch_invocations().is_empty(),
        "denied approval resume must not dispatch any capability batch invocations"
    );
    assert!(
        host.progress_events().iter().any(|event| matches!(
            event,
            LoopProgressEvent::CapabilityActivityFailed {
                activity_id,
                capability_id: emitted_capability_id,
                reason_kind: CapabilityFailureKind::GateDeclined,
                ..
            } if *activity_id == denied_activity_id && *emitted_capability_id == capability_id()
        )),
        "denied approval resume must emit a persistent failed capability activity"
    );
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        1,
        "one model-visible result ref must be appended for the approval denial"
    );
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("model_observation must be Some for an gate-declined failure");
    assert_eq!(observation.status, ToolObservationStatus::Error);
    assert_eq!(
        observation.summary, "Capability declined by user.",
        "observation summary must describe the gate-declined failure"
    );
    let recovery = observation
        .recovery
        .as_ref()
        .expect("recovery must be present");
    assert_eq!(recovery.same_call_retry, SameCallRetryConstraint::Forbidden);
}

#[tokio::test]
async fn capability_stage_denied_auth_resume_surfaces_gate_declined_failure_and_continues() {
    // Use a provider call fixture (provider_replay set) so the observation is
    // actually appended to appended_result_refs by
    // append_capability_safe_summary_ref_with_observation.
    let host =
        MockHost::new(Vec::new()).with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::GateDeclined,
                "auth gate denied by user".to_string(),
                None,
            )],
            stopped_on_suspension: false,
        }]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Build state with pending_auth_resume carrying Denied disposition,
    // matching the capability_id() from provider_calls_response.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-test").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:deny-test")
            .expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: Some(
            CapabilityResumeToken::new(denied_activity_id.to_string()).expect("valid token"),
        ),
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    // Use provider_calls_response so provider_replay is set, enabling the
    // model-visible observation to be written to appended_result_refs.
    let mut calls = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider calls fixture"),
    };
    calls[0].activity_id = denied_activity_id;

    let mut current_surface =
        ironclaw_turns::run_profile::LoopCapabilityPort::visible_capabilities(
            &host,
            VisibleCapabilityRequest,
        )
        .await
        .expect("visible surface");
    current_surface.descriptors.clear();
    current_surface.callable_capability_ids = Some(Vec::new());

    let step = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state,
                surface: current_surface,
                calls,
            },
        )
        .await
        .expect("capability stage");

    // 1. Must return Continue (loop proceeds, not Blocked or Failed).
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied auth resume, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared after the denied-resume path.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after surfacing the deny failure"
    );

    // 3. The denial crosses the canonical capability port as a typed terminal
    // auth resume even though the blocked capability has disappeared from the
    // current surface. The production host uses this request only to
    // terminalize the exact durable invocation; it never dispatches the
    // capability provider.
    let batches = host.batch_invocations();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].invocations.len(), 1);
    let auth_resume = batches[0].invocations[0]
        .auth_resume
        .as_ref()
        .expect("denied auth resume reaches the capability lifecycle");
    assert_eq!(
        auth_resume.disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied)
    );
    assert!(auth_resume.resume_token.is_none());
    // 4. One model-visible observation appended with GateDeclined error + Forbidden retry.
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        1,
        "exactly one model-visible result ref must be appended for the deny failure"
    );
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("model_observation must be Some for an gate-declined failure");
    assert_eq!(
        observation.status,
        ToolObservationStatus::Error,
        "observation status must be Error"
    );
    assert_eq!(
        observation.summary, "Capability declined by user.",
        "observation summary must describe the gate-declined failure"
    );
    let recovery = observation
        .recovery
        .as_ref()
        .expect("recovery must be present");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "gate-declined failure must map to Forbidden retry constraint (model must not retry)"
    );
}

#[tokio::test]
async fn auth_gate_without_resume_token_records_activity_id_for_denial_failure() {
    let mut calls = match calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected capability call fixture"),
    };
    let blocked_activity_id = calls[0].activity_id;

    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:hook-auth-tokenless").expect("valid"),
                    Vec::new(),
                    "hook requested auth".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: the denied resume crosses the port as a typed terminal
        // auth resume; the host terminalizes it as a gate-declined failure.
        ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::failed(
                CapabilityFailureKind::GateDeclined,
                "auth gate denied by user".to_string(),
                None,
            )],
            stopped_on_suspension: false,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let phase1 = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state: LoopExecutionState::initial_for_run(host.run_context()),
                surface: ironclaw_turns::run_profile::LoopCapabilityPort::visible_capabilities(
                    &host,
                    VisibleCapabilityRequest,
                )
                .await
                .expect("visible surface"),
                calls: calls.clone(),
            },
        )
        .await
        .expect("phase 1 must block on auth gate");
    assert!(
        matches!(phase1, TurnCompletedStep::Exit(LoopExit::Blocked(_))),
        "tokenless auth gate must block the turn"
    );

    let mut blocked_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending = blocked_state
        .pending_auth_resume
        .as_ref()
        .expect("tokenless auth gate must create pending auth resume");
    assert_eq!(
        pending.resume_token, None,
        "hook-style tokenless auth gates should remain tokenless"
    );
    assert_eq!(
        pending.activity_id, blocked_activity_id,
        "tokenless auth gate must persist the call activity id for later terminal events"
    );

    blocked_state
        .pending_auth_resume
        .as_mut()
        .expect("pending auth resume")
        .disposition = Some(ironclaw_turns::GateResumeDisposition::Denied);

    let phase2 = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state: blocked_state,
                surface: ironclaw_turns::run_profile::LoopCapabilityPort::visible_capabilities(
                    &host,
                    VisibleCapabilityRequest,
                )
                .await
                .expect("visible surface"),
                calls: std::mem::take(&mut calls),
            },
        )
        .await
        .expect("phase 2 must surface declined auth gate");
    let final_state = match phase2 {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied tokenless auth gate, got Exit: {exit:?}")
        }
    };

    assert!(
        final_state.pending_auth_resume.is_none(),
        "denied tokenless auth resume must be cleared"
    );
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        2,
        "phase 2 must send exactly one more batch: the typed denial terminalization"
    );
    assert_eq!(batches[1].invocations.len(), 1);
    let denied_resume = batches[1].invocations[0]
        .auth_resume
        .as_ref()
        .expect("denied tokenless resume reaches the capability lifecycle");
    assert_eq!(
        denied_resume.disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied)
    );
    assert!(
        denied_resume.resume_token.is_none(),
        "hook-style tokenless auth gates should remain tokenless"
    );
    // The parked activity id must travel on the typed denial so the host
    // terminalizes the ORIGINAL durable invocation (the loop no longer
    // synthesizes a CapabilityActivityFailed event for it).
    assert_eq!(
        batches[1].invocations[0].activity_id, blocked_activity_id,
        "denied tokenless auth gate must terminalize the original activity id"
    );
}

/// Regression test for the auth-deny partition fix.
///
/// When a resumed run carries `pending_auth_resume` with `disposition = Some(Denied)`
/// and the parallel batch contains BOTH the denied capability (X = `capability_id()`)
/// AND an unrelated capability (Y = `other_capability_id()`), only X must receive
/// an gate-declined failure. Y must be dispatched normally and its outcome must
/// appear in the result refs. The loop must continue (not exit), and
/// `pending_auth_resume` must be cleared.
#[tokio::test]
async fn capability_stage_denied_auth_resume_only_fails_matching_call_remaining_dispatched() {
    let y_result_ref = LoopResultRef::new("result:y-outcome").expect("valid");

    // The MockHost surface normally contains only capability_id() ("demo.echo").
    // Add other_capability_id() ("demo.list") so that call Y is treated as
    // visible rather than denied-by-surface.
    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_turns::run_profile::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            // Order matches invocation order: X's typed denial terminalizes
            // first, then Y's normal dispatch completes.
            resolutions: vec![
                resolution::failed(
                    CapabilityFailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    None,
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "list done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_auth_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-multi-test").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:deny-x")
            .expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    // Build a batch with two calls:
    //   call X: capability_id() ("demo.echo")  — matches denied pending_auth_resume
    //   call Y: other_capability_id() ("demo.list") — unrelated, must proceed normally
    //
    // Use provider_replay on X so the gate-declined failure observation is
    // written to appended_result_refs (same pattern as the single-call test).
    let calls = vec![
        // call X — denied
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:x-denied")
                .expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_turns::run_profile::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // call Y — unrelated, must dispatch normally
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:y-unrelated")
                .expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop must proceed, not exit.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after partial auth-deny, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared — the denial was consumed.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after the denied call was surfaced"
    );

    // 3. One batch carries BOTH calls: X as a typed terminal denial (the host
    // terminalizes the durable invocation, never dispatches the provider) and
    // Y as a plain dispatch.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry the typed denial for X and the dispatch for Y"
    );
    assert_eq!(
        batches[0].invocations.len(),
        2,
        "batch must contain X's denial terminalization and Y's dispatch"
    );
    let x_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == capability_id())
        .expect("X's invocation must be present");
    let x_resume = x_invocation
        .auth_resume
        .as_ref()
        .expect("X must carry the typed denied auth resume");
    assert_eq!(
        x_resume.disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied)
    );
    assert!(x_resume.resume_token.is_none());
    let y_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == other_capability_id())
        .expect("Y's invocation must be present");
    assert!(
        y_invocation.auth_resume.is_none(),
        "Y must dispatch as a plain invocation"
    );

    // 4. Two result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        2,
        "expected two appended result refs: one gate-declined failure for X, one Completed for Y"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let auth_failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = auth_failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // The Completed result for Y must be present (no model observation on a Completed outcome).
    let y_completed = appended
        .iter()
        .find(|r| r.result_ref == y_result_ref)
        .expect("Y's completed result ref must be appended");
    assert!(
        y_completed.model_observation.is_none()
            || y_completed
                .model_observation
                .as_ref()
                .is_some_and(|o| o.status != ToolObservationStatus::Error),
        "Y's result must not be an gate-declined failure"
    );

    // 5. final_state.result_refs must carry BOTH refs so the next model prompt sees them.
    //    A regression that appends to the host but forgets to update result_refs would still
    //    pass all checks above — this assertion catches that gap.
    assert!(
        final_state
            .result_refs
            .contains(&auth_failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
}

#[tokio::test]
async fn capability_stage_denied_auth_resume_only_fails_matching_activity_when_capability_repeats()
{
    let y_result_ref = LoopResultRef::new("result:same-cap-y-outcome").expect("valid");
    let host =
        MockHost::new(Vec::new()).with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            // Order matches invocation order: the denied activity's typed
            // terminalization first, then the surviving call's completion.
            resolutions: vec![
                resolution::failed(
                    CapabilityFailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    None,
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "same capability second call done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let denied_activity_id = CapabilityActivityId::new();
    let surviving_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-same-cap").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:deny-same-cap-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    let calls = vec![
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:x-same-cap-denied").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x_same_cap".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: surviving_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:y-same-cap-survives").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        },
    ];

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

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after same-capability partial auth deny, got Exit: {exit:?}")
        }
    };

    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry the denied activity's terminalization and the surviving call"
    );
    assert_eq!(batches[0].invocations.len(), 2);
    let denied_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.activity_id == denied_activity_id)
        .expect("denied activity's invocation must be present");
    assert_eq!(
        denied_invocation
            .auth_resume
            .as_ref()
            .expect("denied activity carries the typed denied auth resume")
            .disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied)
    );
    let surviving_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.activity_id == surviving_activity_id)
        .expect("surviving activity's invocation must be present");
    assert_eq!(surviving_invocation.capability_id, capability_id());
    assert!(
        surviving_invocation.auth_resume.is_none(),
        "the surviving same-capability call must dispatch as a plain invocation"
    );
    // The typed batch above already pins that ONLY the parked activity carries
    // the denied resume (the host terminalizes the durable invocation; the
    // loop no longer synthesizes a CapabilityActivityFailed event for it).
    // The model-visible gate-declined observation lands for X alone:
    let appended = host.appended_result_refs();
    let declined_observations: Vec<_> = appended
        .iter()
        .filter_map(|entry| entry.model_observation.as_ref())
        .filter(|observation| observation.status == ToolObservationStatus::Error)
        .collect();
    assert_eq!(
        declined_observations.len(),
        1,
        "only the parked activity should receive the gate-declined failure"
    );
    assert_eq!(
        declined_observations[0].summary, "Capability declined by user.",
        "the gate-declined failure must be the model-visible declined observation"
    );
    assert!(final_state.pending_auth_resume.is_none());
    assert!(final_state.result_refs.contains(&y_result_ref));
}

/// Regression test: partition + sizing invariant with 1 denied + 2 remaining calls.
///
/// When the denied auth-resume batch contains one denied call (X) and TWO
/// unrelated remaining calls (Y and Z), the partition must place both Y and Z
/// into the fall-through batch.  `invoke_capability_batch` must receive exactly
/// 2 invocations, return 2 outcomes, and the loop must continue.
///
/// This exercises the `outcomes.len() == invocations.len()` invariant under
/// more than one remaining call, which the 1+1 sibling test above does not cover.
#[tokio::test]
async fn capability_stage_denied_auth_resume_one_denied_two_remaining_all_dispatched() {
    let y_result_ref = LoopResultRef::new("result:y-multi").expect("valid");
    let z_result_ref = LoopResultRef::new("result:z-multi").expect("valid");
    let z_capability_id = ironclaw_host_api::CapabilityId::new("demo.write").expect("valid cap id");

    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            // Y: demo.list
            ironclaw_turns::run_profile::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
            // Z: demo.write
            ironclaw_turns::run_profile::CapabilityDescriptorView {
                capability_id: z_capability_id.clone(),
                provider: None,
                runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                safe_name: "demo_write".to_string(),
                safe_description: "demo write capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            // Order matches invocation order: X's typed denial terminalizes
            // first, then Y and Z complete.
            resolutions: vec![
                resolution::failed(
                    CapabilityFailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    None,
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "list done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::completed(
                    z_result_ref.clone(),
                    "write done".to_string(),
                    ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // pending_auth_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-1plus2").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:deny-x-1plus2")
            .expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    // Three calls: X (denied), Y (unrelated), Z (unrelated).
    let calls = vec![
        // X — matches denied pending_auth_resume; provider_replay set so the
        // gate-declined failure observation is written to appended_result_refs.
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:x-denied-1plus2",
            )
            .expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_turns::run_profile::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // Y — unrelated, must dispatch normally.
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:y-unrelated-1plus2",
            )
            .expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
        // Z — unrelated, must dispatch normally.
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: z_capability_id.clone(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:z-unrelated-1plus2",
            )
            .expect("valid"),
            effective_capability_ids: vec![z_capability_id.clone()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop proceeds with both Y and Z completed.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after 1-denied + 2-remaining, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after the denied call was surfaced"
    );

    // 3. One batch carries all three: X as a typed terminal denial plus the
    //    plain dispatches for Y and Z. This validates the
    //    outcomes.len() == invocations.len() invariant for >1 remaining.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry X's denial terminalization plus Y and Z"
    );
    assert_eq!(
        batches[0].invocations.len(),
        3,
        "batch must contain X's denial terminalization and the Y/Z dispatches"
    );
    let x_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == capability_id())
        .expect("X's invocation must be present");
    assert_eq!(
        x_invocation
            .auth_resume
            .as_ref()
            .expect("X carries the typed denied auth resume")
            .disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied)
    );
    let dispatched_ids: std::collections::HashSet<_> = batches[0]
        .invocations
        .iter()
        .filter(|invocation| invocation.auth_resume.is_none())
        .map(|invocation| invocation.capability_id.clone())
        .collect();
    let expected_ids: std::collections::HashSet<_> =
        [other_capability_id(), z_capability_id.clone()]
            .into_iter()
            .collect();
    assert_eq!(
        dispatched_ids, expected_ids,
        "the plain dispatches must be exactly Y (demo.list) and Z (demo.write)"
    );

    // 4. Three result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    //    - one Completed result for Z
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        3,
        "expected 3 appended result refs: gate-declined failure for X, Completed for Y and Z"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let auth_failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = auth_failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // Both Y and Z Completed results must be present.
    assert!(
        appended.iter().any(|r| r.result_ref == y_result_ref),
        "Y's completed result ref must be appended"
    );
    assert!(
        appended.iter().any(|r| r.result_ref == z_result_ref),
        "Z's completed result ref must be appended"
    );

    // 5. final_state.result_refs must carry all three refs.
    assert!(
        final_state
            .result_refs
            .contains(&auth_failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
    assert!(
        final_state.result_refs.contains(&z_result_ref),
        "final_state.result_refs must contain Z's completed result ref"
    );
}

/// When a resumed run carries `pending_approval_resume` with
/// `disposition = Some(Denied)` and the parallel batch contains BOTH the
/// denied capability (X = `capability_id()`) AND an unrelated capability
/// (Y = `other_capability_id()`), only X must receive a gate-declined
/// failure. Y must be dispatched normally and its outcome must appear in
/// the result refs. The loop must continue, and `pending_approval_resume`
/// must be cleared.
#[tokio::test]
async fn capability_stage_denied_approval_resume_only_fails_matching_call_remaining_dispatched() {
    let y_result_ref = LoopResultRef::new("result:approval-y-outcome").expect("valid");

    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_turns::run_profile::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                y_result_ref.clone(),
                "list done".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_approval_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-multi").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("00000000-0000-0000-0000-000000000043")
            .expect("valid"),
        activity_id: denied_activity_id,
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:approval-deny-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    // Build a batch with two calls:
    //   call X: capability_id() ("demo.echo")  — matches denied pending_approval_resume
    //   call Y: other_capability_id() ("demo.list") — unrelated, must proceed normally
    let calls = vec![
        // call X — denied
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:x-approval-denied").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_turns::run_profile::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // call Y — unrelated, must dispatch normally
        ironclaw_turns::run_profile::CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: CapabilityInputRef::new("input:y-approval-unrelated").expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop must proceed, not exit.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after partial approval-deny, got Exit: {exit:?}")
        }
    };

    // 2. pending_approval_resume must be cleared — the denial was consumed.
    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared after the denied call was surfaced"
    );

    // 3. Exactly one batch invocation containing only call Y (not X).
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "exactly one batch invocation must occur for the remaining (non-denied) call Y"
    );
    let batch_ids: Vec<_> = batches[0]
        .invocations
        .iter()
        .map(|inv| &inv.capability_id)
        .collect();
    assert!(
        batch_ids.iter().all(|id| **id == other_capability_id()),
        "the batch must contain only call Y (other_capability_id), not call X"
    );
    assert_eq!(
        batch_ids.len(),
        1,
        "batch must contain exactly one invocation (call Y)"
    );

    // 4. Two result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        2,
        "expected two appended result refs: one gate-declined failure for X, one Completed for Y"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // The Completed result for Y must be present.
    let y_completed = appended
        .iter()
        .find(|r| r.result_ref == y_result_ref)
        .expect("Y's completed result ref must be appended");
    assert!(
        y_completed.model_observation.is_none()
            || y_completed
                .model_observation
                .as_ref()
                .is_some_and(|o| o.status != ToolObservationStatus::Error),
        "Y's result must not be an gate-declined failure"
    );

    // 5. final_state.result_refs must carry BOTH refs.
    assert!(
        final_state.result_refs.contains(&failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
}

/// No-match variant of the denied-approval short-circuit test.
///
/// When `pending_approval_resume` is set with `disposition = Some(Denied)` for
/// capability X but the model emits *only* calls for capability Y (no X in
/// `visible_calls`), the executor must:
///
/// 1. NOT surface X as an gate-declined failure (X is absent from the batch).
/// 2. Dispatch Y normally and record its outcome.
/// 3. Clear `pending_approval_resume` after processing the batch.
/// 4. Return `TurnCompletedStep::Continue` (loop proceeds).
///
/// This ensures the stale denied state is always evicted even when the model
/// does not reproduce the denied call in the next turn.
#[tokio::test]
async fn capability_stage_denied_approval_resume_no_matching_call_dispatches_unrelated_normally() {
    let y_result_ref = LoopResultRef::new("result:approval-no-match-y").expect("valid");

    // Only capability Y is needed in the batch outcome; X is never submitted.
    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_turns::run_profile::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::ResolutionBatch {
            resolutions: vec![resolution::completed(
                y_result_ref.clone(),
                "list done no-match".to_string(),
                ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_approval_resume for capability X = capability_id(), Denied.
    // The model will emit ONLY capability Y calls — X is absent from visible_calls.
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-no-match").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("00000000-0000-0000-0000-000000000099")
            .expect("valid"),
        activity_id: CapabilityActivityId::new(),
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:approval-deny-no-match-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
    });

    // The model emits ONLY call Y (other_capability_id); no X in this batch.
    let calls = vec![ironclaw_turns::run_profile::CapabilityCallCandidate {
        activity_id: ironclaw_turns::CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: other_capability_id(),
        input_ref: CapabilityInputRef::new("input:y-no-match-approval").expect("valid"),
        effective_capability_ids: vec![other_capability_id()],
        provider_replay: None,
    }];

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

    // 1. Must return Continue — the loop proceeds.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue when denied X is absent from the batch, got Exit: {exit:?}")
        }
    };

    // 2. X must NOT appear as a failure — it was not in the visible batch.
    let appended = host.appended_result_refs();
    assert!(
        !appended.iter().any(|r| r
            .model_observation
            .as_ref()
            .is_some_and(|o| o.summary == "Capability declined by user.")),
        "X must NOT be surfaced as an gate-declined failure when it is absent from visible_calls"
    );

    // 3. Y dispatches normally: exactly one batch invocation containing Y.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "exactly one batch invocation must occur for call Y"
    );
    assert!(
        batches[0]
            .invocations
            .iter()
            .all(|inv| inv.capability_id == other_capability_id()),
        "the batch must contain only call Y"
    );

    // 4. Y's result ref is present in appended refs and final_state.
    assert!(
        appended.iter().any(|r| r.result_ref == y_result_ref),
        "Y's completed result ref must be appended"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );

    // 5. pending_approval_resume is cleared after the batch — stale denied
    //    state must not survive into the next iteration.
    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared even when the denied capability X was absent \
         from the model's batch"
    );
}
