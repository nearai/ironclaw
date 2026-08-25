use super::*;

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
async fn recovery_event_append_failure_stops_before_model_retry() {
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model gateway unavailable",
        )])
        .with_failing_progress_port();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("recovery cannot proceed without its durable numerator event");

    match error {
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Checkpoint,
            kind: AgentLoopHostErrorKind::Unavailable,
            safe_summary,
            reason_kind: None,
            detail: None,
        } => assert_eq!(safe_summary.as_str(), "progress sink unavailable"),
        other => panic!("expected checkpoint diagnostics, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "the failed recovery append must gate the retry side effect"
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
    // Three before-model checkpoints: one per iteration from the spine, plus
    // the follow-up drain's own cursor checkpoint (written so the drained
    // input can be acked — and become model-visible — before the next prompt
    // is built).
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::Final,
        ]
    );
    assert_eq!(final_staged_state(&host).stop_state.turns_completed, 2);
}

#[tokio::test]
async fn reply_only_drains_steering_arriving_at_exit_boundary() {
    // Regression: a `Steering` input that arrives while the run's FINAL model
    // call is in flight is only observable at the reply-only exit boundary —
    // the steering drain at iteration start has already run. The follow-up
    // drain must consume it and force one more iteration (so the model sees
    // the message), rather than completing the run and stranding the input
    // unconsumed at the queue head.
    let host = MockHost::new(vec![reply_response(), reply_response()]);
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![
        LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: input_cursor(&run_context, "input-cursor:no-input"),
        },
        LoopInputBatch {
            inputs: vec![LoopInput::Steering {
                message_ref: message_ref("msg:late-steering"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-late-steering",
                "input-ack:after-late-steering",
            )],
            next_cursor: input_cursor(&run_context, "input-cursor:after-late-steering"),
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
        host.model_requests().len(),
        2,
        "the late steering input must force one more model iteration"
    );
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:after-late-steering").expect("valid")]
    );
    // Ordering, not just occurrence: the forced extra iteration must build its
    // prompt AFTER the drain's durable cursor checkpoint and ack — the ack is
    // what makes the queued row model-visible, so an iteration forced with the
    // ack still pending would feed the model a prompt without the message.
    assert_eq!(
        host.events(),
        vec![
            "build_prompt_bundle".to_string(),
            "checkpoint:before_model".to_string(),
            "checkpoint:before_model".to_string(),
            "ack_inputs".to_string(),
            "build_prompt_bundle".to_string(),
            "checkpoint:before_model".to_string(),
            "checkpoint:final".to_string(),
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
async fn input_stage_steering_drain_acks_eagerly_after_cursor_checkpoint() {
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
                mode: UserFacingInputDrainMode::Steering,
            },
        )
        .await
        .expect("input stage");

    match step {
        InputStep::Continue { state, drained } => {
            assert!(drained);
            assert_eq!(
                state.input_cursor,
                input_cursor(&run_context, "input-cursor:after-user")
            );
            // The drain stage acks the consumed input itself — after writing a
            // durable checkpoint of the advanced cursor — so the queued
            // transcript row is model-visible before this iteration's prompt
            // is built. The ack lifecycle never leaves this stage.
            assert_eq!(
                host.acked_input_tokens(),
                vec![LoopInputAckToken::new("input-ack:after-user").expect("valid")]
            );
            assert_eq!(
                host.events(),
                vec![
                    "checkpoint:before_model".to_string(),
                    "ack_inputs".to_string(),
                ]
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

/// The production caller — not the pure consume helper — owns the sequence a
/// settled subagent result depends on: `InputStage::process` writes a
/// `BeforeModel` checkpoint of the advanced cursor and only THEN acks, and the
/// ack is what flips the queued transcript row to `Submitted` (model-visible).
/// A `GateResolved` parked right behind the settled input pins the other half:
/// the barrier stops the drain, so its ack token must never be acked and the
/// cursor must not run past it. Both user-facing drain modes reach the settled
/// input, so both are driven here.
#[tokio::test]
async fn input_stage_drains_subagent_settled_ahead_of_a_barrier_in_both_modes() {
    for mode in [
        UserFacingInputDrainMode::Steering,
        UserFacingInputDrainMode::FollowUp,
    ] {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::SubagentSettled {
                    child_run_id: TurnRunId::new(),
                    message_ref: message_ref("msg:child-result-1"),
                },
                LoopInput::GateResolved {
                    gate_ref: LoopGateRef::new("gate:blocks-the-drain").expect("valid gate ref"),
                },
            ],
            input_acks: vec![
                input_ack(
                    &run_context,
                    "input-cursor:after-settled",
                    "input-ack:settled",
                ),
                input_ack(&run_context, "input-cursor:after-gate", "input-ack:gate"),
            ],
            next_cursor: input_cursor(&run_context, "input-cursor:after-gate"),
        }]);
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host: &host,
        };
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let step = InputStage
            .process(ctx, DrainInput { state, mode })
            .await
            .expect("input stage");

        match step {
            InputStep::Continue { state, drained } => {
                assert!(drained, "settled results must drain in {mode:?}");
                assert_eq!(
                    state.input_cursor,
                    input_cursor(&run_context, "input-cursor:after-settled"),
                    "the cursor stops at the barrier in {mode:?}"
                );
            }
            InputStep::Exit(exit) => panic!("expected continue in {mode:?}, got {exit:?}"),
        }

        assert_eq!(
            host.acked_input_tokens(),
            vec![LoopInputAckToken::new("input-ack:settled").expect("valid ack token")],
            "only the settled input is acked in {mode:?}; the gate stays queued"
        );
        assert_eq!(
            host.checkpoint_kinds(),
            vec![LoopCheckpointKind::BeforeModel],
            "the advanced cursor is checkpointed in {mode:?}"
        );
        let staged_before_model = host
            .staged_payloads()
            .into_iter()
            .find(|request| request.kind == LoopCheckpointKind::BeforeModel)
            .unwrap_or_else(|| panic!("no BeforeModel checkpoint payload staged in {mode:?}"));
        let staged_state = LoopExecutionState::from_checkpoint_payload(
            &staged_before_model.payload,
            CheckpointKind::BeforeModel,
        )
        .expect("checkpoint payload");
        // `checkpoint_kinds()` proves only the checkpoint's kind, and the
        // final `state.input_cursor` above proves only the in-memory cursor —
        // neither proves what cursor value was actually persisted. Decode the
        // staged payload bytes (what the host would durably journal) and
        // assert the cursor inside it directly, so a regression that
        // checkpoints a stale cursor and only later advances the in-memory
        // one cannot pass silently.
        assert_eq!(
            staged_state.input_cursor,
            input_cursor(&run_context, "input-cursor:after-settled"),
            "the persisted checkpoint cursor must be the advanced cursor in {mode:?}"
        );
        assert_eq!(
            host.events(),
            vec![
                "checkpoint:before_model".to_string(),
                "ack_inputs".to_string(),
            ],
            "the checkpoint must be durable before the ack in {mode:?}"
        );
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

/// A settled subagent result must be consumed by the real drain call site — in
/// BOTH user-facing modes — before the drain stops at a barrier input. Placing
/// a `GateResolved` right behind it is the case a predicate-only test cannot
/// see: it proves the settled input was consumed (cursor advanced by exactly
/// one, one ack token returned) rather than merely classified as drainable.
#[test]
fn consume_drainable_inputs_drains_subagent_settled_ahead_of_a_barrier_in_both_modes() {
    for mode in [
        UserFacingInputDrainMode::Steering,
        UserFacingInputDrainMode::FollowUp,
    ] {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context();
        let mut state = LoopExecutionState::initial_for_run(run_context);
        let before_cursor = state.input_cursor.clone();
        let batch = LoopInputBatch {
            inputs: vec![
                LoopInput::SubagentSettled {
                    child_run_id: TurnRunId::new(),
                    message_ref: message_ref("msg:child-result-1"),
                },
                LoopInput::GateResolved {
                    gate_ref: LoopGateRef::new("gate:blocks-the-drain").expect("valid gate ref"),
                },
            ],
            input_acks: vec![
                input_ack(
                    run_context,
                    "input-cursor:after-settled",
                    "input-ack:settled",
                ),
                input_ack(run_context, "input-cursor:after-gate", "input-ack:gate"),
            ],
            next_cursor: before_cursor.clone(),
        };

        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, mode, &mut state).expect("consume inputs");

        assert!(drained, "settled results must drain in {mode:?}");
        assert!(
            cancelled_reason_kind.is_none(),
            "no cancellation in {mode:?}"
        );
        assert_eq!(
            ack_tokens,
            vec![LoopInputAckToken::new("input-ack:settled").expect("valid ack token")],
            "only the settled input is consumed in {mode:?}; the gate is a barrier"
        );
        assert_eq!(
            state.input_cursor,
            input_cursor(run_context, "input-cursor:after-settled"),
            "the cursor must advance past the settled input in {mode:?}"
        );
        assert_ne!(
            state.input_cursor, before_cursor,
            "the cursor must not stay put in {mode:?}"
        );
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
            assert!(state.recent_output_token_counts.is_empty());
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "done".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
    use ironclaw_loop_contracts::{LoopModelResponse, LoopModelUsage};

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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref,
                "done".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "done".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
