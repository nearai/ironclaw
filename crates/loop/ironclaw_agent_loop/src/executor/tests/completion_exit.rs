use super::*;

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
    // The single model call is the best-effort failure explanation (§5a.2);
    // no assistant reply was finalized.
    assert_eq!(
        host.model_requests().len(),
        1,
        "only the best-effort failure-explanation call may be issued"
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
async fn no_progress_exit_remains_typed_failure_when_driver_nudges_enabled() {
    // Driver nudges do not add a second recovery mechanism after the terminal
    // warning turn. The available model reply is consumed only by the
    // failure-explanation call (§5a.2).
    let host = MockHost::new(vec![reply_response_with_text(
        "explanation after no progress",
    )])
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

    // Exactly one model call — the failure explanation — and the run still
    // fails with the typed no-progress category.
    assert_eq!(host.model_requests().len(), 1);
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
    .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
        resolutions: vec![resolution::completed(
            result_ref.clone(),
            "wrote file".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
async fn completion_nudge_skips_confirmation_with_quoted_literal_ending_in_colon() {
    let confirmation = "Done — routine **QA Recurring Telegram** is active and scheduled to run every minute, sending the current Toronto time to Telegram with messages beginning:\n\n> QA recurring tick:";
    let host = MockHost::new(vec![
        reply_response_with_text(confirmation),
        reply_response_with_text("duplicate confirmation one"),
        reply_response_with_text("duplicate confirmation two"),
    ])
    .with_driver_nudges_enabled();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.finalized_assistant_messages(),
        vec![confirmation.to_string()],
        "a quoted literal is completed content, not an unfinished narration"
    );
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(final_staged_state(&host).completion_nudges_used, 0);
}

#[tokio::test]
async fn scheduled_question_gets_bounded_nudge_then_completes_with_answer() {
    let host = MockHost::new(vec![
        reply_response_with_text("Which repository should I inspect?"),
        reply_response_with_text("Inspected nearai/ironclaw. No blocking failures found."),
    ])
    .with_driver_nudges_enabled()
    .with_scheduled_trigger_origin();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.prompt_requests().len(), 2);
    assert_eq!(final_staged_state(&host).completion_nudges_used, 1);
    assert!(
        host.prompt_requests()[1]
            .inline_messages
            .iter()
            .any(|message| message.safe_body.as_str().contains("Finish the task now"))
    );
}

#[tokio::test]
async fn scheduled_empty_reply_fails_through_canonical_executor() {
    let host =
        MockHost::new(vec![reply_response_with_text("   \n")]).with_scheduled_trigger_origin();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let LoopExit::Failed(failed) = exit else {
        panic!("expected empty scheduled output to fail, got {exit:?}");
    };
    assert_eq!(failed.reason_kind, LoopFailureKind::InvalidModelOutput);
    let summary = failed
        .safe_summary
        .expect("empty scheduled output must carry typed failure detail");
    assert_eq!(summary.category(), "invalid_model_output");
    assert_eq!(
        summary.detail(),
        Some("model returned an empty assistant response")
    );
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["   \n".to_string()]
    );
    assert_eq!(final_staged_state(&host).completion_nudges_used, 0);
}

#[tokio::test]
async fn interactive_question_remains_a_normal_completion() {
    let host = MockHost::new(vec![reply_response_with_text(
        "Which repository should I inspect?",
    )])
    .with_driver_nudges_enabled();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(final_staged_state(&host).completion_nudges_used, 0);
}

#[tokio::test]
async fn scheduled_answer_with_internal_question_completes_without_nudge() {
    let answer = "Did the deployment pass? Yes. All required checks passed.";
    let host = MockHost::new(vec![reply_response_with_text(answer)])
        .with_driver_nudges_enabled()
        .with_scheduled_trigger_origin();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.finalized_assistant_messages(),
        vec![answer.to_string()]
    );
    assert_eq!(final_staged_state(&host).completion_nudges_used, 0);
}

#[test]
fn trailing_off_detection_keeps_bare_unfinished_colon_but_accepts_markdown_quote() {
    assert!(super::super::reply_trailed_off("Let me write the file:"));
    assert!(!super::super::reply_trailed_off(
        "The message will begin:\n\n> QA recurring tick:"
    ));
}

#[tokio::test]
async fn consumed_iteration_warning_falls_back_to_failed_exit() {
    let host = MockHost::new(vec![reply_response_with_text("iteration explanation")]);
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
    assert!(
        state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::iteration_limit(state.iteration))
    );
    state.terminal_warning_state.mark_delivered();
    state.terminal_warning_state.clear_active();

    let step = BudgetStage
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage should finalize the exhausted warning path");

    assert_eq!(
        host.model_requests().len(),
        1,
        "the consumed warning falls through to one best-effort failure explanation"
    );
    assert!(
        matches!(step, BudgetStep::Exit(LoopExit::Failed(_))),
        "an exhausted warning must preserve the typed failed exit"
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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "stopped batch completed".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
async fn typed_structured_result_terminalizes_suppressed_schedule_without_reply_refs() {
    let host = MockHost::new(Vec::new()).with_suppressed_scheduled_context();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.stop_state.structured_result_recorded = true;
    state
        .result_refs
        .push(LoopResultRef::new("result:prior-tool").expect("result ref"));
    state
        .result_refs
        .push(LoopResultRef::new("result:nothing-to-report").expect("result ref"));
    state
        .assistant_refs
        .push(message_ref("msg:intermediate-progress"));

    let exit = completed_exit(&host, state, None).expect("completed exit");

    let LoopExit::Completed(completed) = exit else {
        panic!("expected completed exit");
    };
    assert_eq!(
        completed.completion_kind,
        LoopCompletionKind::NothingToReport
    );
    assert!(completed.reply_message_refs.is_empty());
    assert!(completed.result_refs.is_empty());
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test]
async fn successful_suppression_result_appends_host_authored_outcome_without_provider_replay() {
    let host = MockHost::new(Vec::new()).with_suppressed_scheduled_context();
    let call = CapabilityCallCandidate {
        activity_id: CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: ironclaw_host_api::ids::CapabilityId::new(
            ironclaw_host_api::prepared_context::STRUCTURED_RESULT_CAPABILITY_ID,
        )
        .expect("capability id"),
        input_ref: CapabilityInputRef::new("input:structured-result").expect("input ref"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
    };
    let result = CapabilityResultMessage {
        result_ref: LoopResultRef::new("result:typed-nothing-to-report").expect("result ref"),
        safe_summary: "structured result recorded".to_string(),
        progress: ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
        terminate_hint: true,
        byte_len: 31,
        output_digest: None,
        model_observation: None,
    };

    append_capability_result_ref(&host, &call, &result)
        .await
        .expect("append result evidence");

    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].provider_call, None);
    assert_eq!(
        appended[0].intrinsic_outcome,
        Some(CapabilityResultIntrinsicOutcome::NothingToReport)
    );
}

#[tokio::test]
async fn typed_nothing_to_report_honors_cancellation_after_final_checkpoint() {
    let host = MockHost::new(Vec::new())
        .with_suppressed_scheduled_context()
        .cancel_after_checkpoint(LoopCheckpointKind::Final);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.stop_state.structured_result_recorded = true;
    state
        .result_refs
        .push(LoopResultRef::new("result:nothing-to-report-cancelled").expect("result ref"));

    let exit = ExitStage
        .process(
            ctx,
            ExitInput {
                state,
                kind: StopKind::GracefulStop,
            },
        )
        .await
        .expect("exit stage");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test]
async fn silent_text_has_no_special_meaning_outside_typed_completion() {
    let host = MockHost::new(vec![reply_response_with_text("[SILENT]")]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(
        exit,
        LoopExit::Completed(ref completed)
            if completed.completion_kind == LoopCompletionKind::FinalReply
    ));
    assert_eq!(host.finalized_assistant_messages(), vec!["[SILENT]"]);
    assert!(host.single_invocations().is_empty());
    assert!(host.batch_invocations().is_empty());
}

#[tokio::test]
async fn reply_that_mentions_silent_is_delivered_normally() {
    let reply = "The literal marker [SILENT] is documented here.";
    let host =
        MockHost::new(vec![reply_response_with_text(reply)]).with_suppressed_scheduled_context();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(
        exit,
        LoopExit::Completed(ref completed)
            if completed.completion_kind == LoopCompletionKind::FinalReply
    ));
    assert_eq!(host.finalized_assistant_messages(), vec![reply]);
    assert!(host.single_invocations().is_empty());
    assert!(host.batch_invocations().is_empty());
}

#[tokio::test]
async fn silent_text_is_visible_with_a_pending_external_tool_resume() {
    let host = MockHost::new(Vec::new()).with_suppressed_scheduled_context();
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_external_tool_resume = Some(PendingExternalToolResume {
        gate_ref: LoopGateRef::new("gate:pending-external-tool").expect("gate ref"),
        capability_id: capability_id(),
        activity_id: CapabilityActivityId::new(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:pending-external-tool").expect("input ref"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        disposition: None,
    });
    let step = AssistantReplyStage
        .process(
            ctx,
            AssistantReplyInput {
                state,
                reply: ironclaw_loop_contracts::AssistantReply {
                    content: "[SILENT]".to_string(),
                },
            },
        )
        .await
        .expect("assistant reply stage");

    let TurnCompletedStep::Continue { state, .. } = step else {
        panic!("pending resume must prevent a nothing-to-report exit");
    };
    assert!(state.pending_external_tool_resume.is_some());
    assert_eq!(host.finalized_assistant_messages(), vec!["[SILENT]"]);
    assert!(host.single_invocations().is_empty());
    assert!(host.batch_invocations().is_empty());
}

#[tokio::test]
async fn terminate_hint_after_batch_completes_without_extra_model_call() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:done").expect("valid"),
                "done".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
            ironclaw_loop_contracts::LoopProgressEvent::CapabilityBatchCompleted {
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
async fn terminate_hint_counts_only_visible_invoked_calls() {
    let host = MockHost::new(vec![mixed_surface_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:visible").expect("valid"),
                "visible call completed".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
            .map(|_| ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    FailureKind::OperationFailed,
                    "filesystem discovery failed".to_string(),
                    diagnostic_failure_detail("filesystem discovery failed"),
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
    let recovery_sequences = host
        .progress_events()
        .into_iter()
        .filter_map(|event| match event {
            LoopProgressEvent::FailureRecovered {
                sequence,
                stage: LoopRecoveryStage::Capability,
                class: LoopRecoveryClass::Capability(FailureKind::OperationFailed),
                disposition: LoopRecoveryDisposition::ModelVisible,
            } => Some(sequence),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        recovery_sequences,
        vec![1, 2, 3],
        "each directly model-visible tool failure must contribute one ordered numerator event"
    );
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
            .map(|_| ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![
                    resolution::failed(
                        FailureKind::OperationFailed,
                        "first discovery failed".to_string(),
                        diagnostic_failure_detail("first discovery failed"),
                    ),
                    resolution::failed(
                        FailureKind::OperationFailed,
                        "second discovery failed".to_string(),
                        diagnostic_failure_detail("second discovery failed"),
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
async fn repeated_non_provider_replayable_failures_do_not_trigger_no_progress_stop() {
    let host = MockHost::new(vec![calls_response(), calls_response(), calls_response()])
        .with_batch_outcomes(
            (0..3)
                .map(|_| ironclaw_host_api::resolution::ResolutionBatch {
                    resolutions: vec![resolution::failed(
                        FailureKind::OperationFailed,
                        "non-replayable capability failed".to_string(),
                        diagnostic_failure_detail("non-replayable capability failed"),
                    )],
                    stopped_on_suspension: false,
                })
                .collect(),
        );
    let executor = CanonicalAgentLoopExecutor;
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    assert!(
        state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::iteration_limit(3))
    );
    state.terminal_warning_state.mark_delivered();
    state.terminal_warning_state.clear_active();

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
