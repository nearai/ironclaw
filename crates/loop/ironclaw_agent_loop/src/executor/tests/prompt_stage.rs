use super::*;

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
async fn prompt_stage_compacts_candidate_emits_redaction_once_then_rebuilds_final_bundle() {
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
            redacted_leak_count: 2,
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
        .process(ctx, PromptInput { state })
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
        PromptStep::SkipModel(_) => panic!("unexpected SkipModel"),
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
            "compaction_leak_detected",
            "compaction_completed",
            "checkpoint_written",
            "prompt_bundle_built",
        ]
    );
    assert!(matches!(
        host.progress_events().as_slice(),
        [
            _,
            _,
            LoopProgressEvent::CompactionLeakDetected {
                reason_kind,
                redacted_leak_count: 2,
                ..
            },
            _,
            _,
            _
        ] if reason_kind.as_str() == "redacted"
    ));
}

#[tokio::test]
async fn prompt_stage_compacts_eviction_through_latest_safe_tool_result_once() {
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(vec![
            vec![
                compaction_metadata(4, LoopContextCompactionKind::Assistant, 10),
                compaction_metadata(9, LoopContextCompactionKind::ToolResult, 10),
            ],
            vec![compaction_metadata(
                4,
                LoopContextCompactionKind::Assistant,
                10,
            )],
        ])
        .with_recent_window_truncation(LoopContextWindowTruncation {
            omitted_through_sequence: 3,
            omitted_through_kind: LoopContextCompactionKind::ToolResult,
        })
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-window-eviction").unwrap(),
            compression_ratio_ppm: 250_000,
            redacted_leak_count: 0,
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 1,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let step = PromptStage
        .process(
            ctx,
            PromptInput {
                state: LoopExecutionState::initial_for_run(host.run_context()),
            },
        )
        .await
        .expect("prompt stage");

    let output = match step {
        PromptStep::Prepared(output) => output,
        PromptStep::Exit(exit) => panic!("expected prepared prompt, got {exit:?}"),
        PromptStep::ResumeApproval(_)
        | PromptStep::ResumeAuth(_)
        | PromptStep::ResumeExternalTool(_)
        | PromptStep::SkipModel(_) => panic!("expected prepared prompt"),
    };
    let requests = host.compaction_requests();
    assert_eq!(requests.len(), 1, "the watermark must trigger exactly once");
    assert_eq!(requests[0].drop_through_seq, 9);
    assert_eq!(requests[0].mode, LoopCompactionMode::WindowEviction);
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        Some(9)
    );
    let initiator = host
        .progress_events()
        .into_iter()
        .find_map(|event| match event {
            LoopProgressEvent::CompactionStarted { initiator, .. } => Some(initiator),
            _ => None,
        });
    assert_eq!(
        initiator,
        Some(ironclaw_loop_contracts::CompactionInitiator::WindowEviction)
    );
}

#[tokio::test]
async fn prompt_stage_does_not_retry_deferred_eviction_watermark_on_unchanged_prompt() {
    let index = vec![compaction_metadata(
        4,
        LoopContextCompactionKind::Assistant,
        10,
    )];
    let host = MockHost::new(Vec::new())
        .with_prompt_compaction_indexes(vec![index.clone(), index])
        .with_recent_window_truncation(LoopContextWindowTruncation {
            omitted_through_sequence: 3,
            omitted_through_kind: LoopContextCompactionKind::ToolResult,
        })
        .with_compaction_outcome(Ok(LoopCompactionOutcome::Deferred {
            safe_summary: LoopSafeSummary::new("compaction deferred until transcript stabilizes")
                .unwrap(),
        }));
    let family = family_with_compaction_strategy(DefaultCompactionStrategy {
        deadline_ms: 1,
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let first = PromptStage
        .process(
            ctx,
            PromptInput {
                state: LoopExecutionState::initial_for_run(host.run_context()),
            },
        )
        .await
        .expect("first prompt stage");
    let first_state = match first {
        PromptStep::Prepared(output) => output.state,
        _ => panic!("expected prepared prompt"),
    };
    let second = PromptStage
        .process(ctx, PromptInput { state: first_state })
        .await
        .expect("second prompt stage");
    let second_state = match second {
        PromptStep::Prepared(output) => output.state,
        _ => panic!("expected prepared prompt"),
    };

    assert_eq!(host.compaction_requests().len(), 1);
    assert!(
        !second_state
            .compaction_state
            .force_compact_on_next_iteration
    );
    assert_eq!(second_state.compaction_state.force_compact_initiator, None);
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
        .process(ctx, PromptInput { state })
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
        PromptStep::SkipModel(_) => panic!("unexpected SkipModel"),
    }
    assert_eq!(host.prompt_requests().len(), 1);
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.progress_event_names(),
        vec!["prompt_bundle_built", "checkpoint_written"]
    );
}

#[tokio::test]
async fn model_context_overflow_exhaustion_gives_model_one_observation_assisted_attempt() {
    let overflow = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContextOverflow,
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
async fn prompt_stage_host_unavailable_on_visible_capabilities_propagates_error() {
    let host = MockHost::new(Vec::new()).with_failing_visible_capabilities();
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage.process(ctx, PromptInput { state }).await;
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

    let result = PromptStage.process(ctx, PromptInput { state }).await;
    let error = match result {
        Ok(_) => panic!("prompt bundle failure should propagate"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Prompt,
            kind: AgentLoopHostErrorKind::Unavailable,
            ..
        }
    ));
}

#[tokio::test]
async fn prompt_stage_preserves_policy_denied_kind_from_prompt_bundle() {
    let host =
        MockHost::new(Vec::new()).with_prompt_bundle_failure(AgentLoopHostErrorKind::PolicyDenied);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage.process(ctx, PromptInput { state }).await;
    let error = match result {
        Ok(_) => panic!("policy denial must stop prompt construction"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Prompt,
            kind: AgentLoopHostErrorKind::PolicyDenied,
            ..
        }
    ));
}

#[tokio::test]
async fn prompt_stage_maps_cancelled_prompt_bundle_error_to_cancelled() {
    let host =
        MockHost::new(Vec::new()).with_prompt_bundle_failure(AgentLoopHostErrorKind::Cancelled);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage.process(ctx, PromptInput { state }).await;

    assert!(matches!(result, Err(AgentLoopExecutorError::Cancelled)));
}

#[tokio::test]
async fn prompt_stage_redacts_rejected_prompt_error_summary() {
    let secret = concat!("ghp_", "012345678901234567890123456789012345");
    let host = MockHost::new(Vec::new()).with_prompt_bundle_error(AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("prompt construction rejected token {secret}"),
    ));
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = PromptStage.process(ctx, PromptInput { state }).await;
    let error = match result {
        Ok(_) => panic!("rejected prompt error summary should propagate safely"),
        Err(error) => error,
    };

    match error {
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Prompt,
            kind: AgentLoopHostErrorKind::Unavailable,
            safe_summary,
            detail: Some(detail),
            ..
        } => {
            assert_eq!(
                safe_summary,
                ironclaw_loop_contracts::LoopSafeSummary::tool_failure_details_redacted()
            );
            assert!(detail.contains("prompt construction rejected token"));
            assert!(detail.contains("[redacted]"));
            assert!(!detail.contains(secret));
        }
        other => panic!("expected sanitized prompt diagnostics, got {other:?}"),
    }
}

#[tokio::test]
async fn failure_explanation_prompt_is_inline_only_and_context_free() {
    // Vehicle: iteration-limit abort at limit 0 (see
    // `failed_exit_finalizes_explanation_and_failed_exit_refs_partial_first`
    // for why the limit is 0 and where the warning turn lands).
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "The run stopped after hitting the iteration limit.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
            "result:limit-2",
        )]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(message_ref("msg:partial-work"));

    let exit = executor
        .execute_family(&family_with_iteration_limit(0), &host, state)
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
    // Vehicle: iteration-limit abort at limit 0 (see
    // `failed_exit_finalizes_explanation_and_failed_exit_refs_partial_first`
    // for why the limit is 0). The scripted `Internal` error must land on the
    // failure-explanation call, not on the pre-termination warning turn —
    // otherwise the run degrades to `ModelError` and this test would no longer
    // pin "the explanation call failing preserves the original failure kind".
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Internal,
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
            "result:limit-3",
        )]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let partial_ref = message_ref("msg:partial-before-error");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(partial_ref.clone());

    let exit = executor
        .execute_family(&family_with_iteration_limit(0), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::IterationLimit);
            assert_eq!(failed.explanation_message_refs, vec![partial_ref]);
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 2);
    assert!(host.finalized_assistant_messages().is_empty());
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
        .process(ctx, PromptInput { state })
        .await
        .expect("prompt stage");

    let returned_state = match step {
        PromptStep::SkipModel(state) => *state,
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
