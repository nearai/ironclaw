use super::*;

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

    assert!(matches!(step, BudgetStep::Exit(LoopExit::Failed(_))));
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn budget_stage_hard_stops_on_wall_clock_limit_without_a_warning_turn() {
    let host = MockHost::new(Vec::new());
    let family = family_with_budget_strategy(crate::strategies::DefaultBudgetStrategy {
        wall_clock_limit: Some(std::time::Duration::from_secs(60)),
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state
        .budget_ledger
        .set_run_started_at_for_test(Some(chrono::Utc::now() - chrono::Duration::seconds(120)));

    let step = BudgetStage
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage");

    match step {
        BudgetStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(
                failed.reason_kind,
                ironclaw_loop_contracts::LoopFailureKind::WallClockLimit
            );
        }
        other => panic!("an exhausted wall clock must hard-stop the run, got {other:?}"),
    }
    // Hard stop: final checkpoint only, no model-visible warning iteration.
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn budget_stage_rearms_wall_clock_accounting_on_checkpoints_without_a_start() {
    let host = MockHost::new(Vec::new());
    let family = family_with_budget_strategy(crate::strategies::DefaultBudgetStrategy {
        wall_clock_limit: Some(std::time::Duration::from_secs(3600)),
        ..Default::default()
    });
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    // An older checkpoint predating wall-clock accounting deserializes with
    // no start; the stage re-arms from resume time instead of failing.
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.budget_ledger.set_run_started_at_for_test(None);

    let step = BudgetStage
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage");

    match step {
        BudgetStep::Continue { state } => {
            assert!(
                state.budget_ledger.run_started_at().is_some(),
                "the stage must re-arm the start so the limit binds from resume"
            );
        }
        other => panic!("a re-armed run must continue, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_stage_hard_stops_when_call_budgets_are_exhausted() {
    let host = MockHost::new(Vec::new());
    let family = family_with_compaction_strategy(DefaultCompactionStrategy::default());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let policy = host
        .run_context()
        .resolved_run_profile
        .resource_budget_policy
        .clone();

    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state
        .budget_ledger
        .set_model_calls_made_for_test(policy.max_model_calls);
    match BudgetStage
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage")
    {
        BudgetStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(
                failed.reason_kind,
                ironclaw_loop_contracts::LoopFailureKind::ModelCallLimit
            );
        }
        other => panic!("an exhausted model-call budget must hard-stop, got {other:?}"),
    }

    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(policy.max_capability_invocations);
    match BudgetStage
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage")
    {
        BudgetStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(
                failed.reason_kind,
                ironclaw_loop_contracts::LoopFailureKind::CapabilityInvocationLimit
            );
        }
        other => panic!("an exhausted capability budget must hard-stop, got {other:?}"),
    }
}

/// A single model turn must not dispatch more calls than the remaining
/// per-run capability-invocation budget allows. Before the fix,
/// `CapabilityStage` admitted and dispatched the whole `visible_calls` batch
/// and charged the full length in one shot, so a 4-call batch with only 2
/// invocations remaining would dispatch (and charge) all 4 — silently
/// exceeding `ResourceBudgetPolicy.max_capability_invocations`, which
/// `BudgetStage` only checks between outer-loop iterations. This pins: the
/// batch is trimmed to the remaining allowance at admission, the trimmed
/// tail is never dispatched but still gets a paired model-visible blocked
/// result, the counter lands exactly at the cap, and the next `BudgetStage`
/// iteration hard-stops the run through the existing
/// `CapabilityInvocationLimit` exit.
#[tokio::test]
async fn capability_stage_trims_batch_to_remaining_capability_budget() {
    let first_ref = LoopResultRef::new("result:budget-trim-first").expect("valid");
    let second_ref = LoopResultRef::new("result:budget-trim-second").expect("valid");
    let make_call = |index: u32| CapabilityCallCandidate {
        activity_id: CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: capability_id(),
        input_ref: CapabilityInputRef::new(format!("input:budget-trim-{index}")).expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            provider_turn_id: "turn_budget".to_string(),
            provider_call_id: format!("call_{index}"),
            provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
            arguments: serde_json::json!({ "message": format!("call-{index}") }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }),
    };
    let calls = vec![make_call(1), make_call(2), make_call(3), make_call(4)];

    let host = MockHost::new(Vec::new()).with_single_outcomes(vec![
        resolution::completed(
            first_ref.clone(),
            "first".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        ),
        resolution::completed(
            second_ref.clone(),
            "second".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        ),
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let policy = host
        .run_context()
        .resolved_run_profile
        .resource_budget_policy
        .clone();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    // Only 2 invocations remain before the run hits its cap.
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(policy.max_capability_invocations - 2);

    let surface = ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
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

    // Exactly 2 host dispatches: the over-budget tail was never sent to the
    // host, whatever its outcome would have been.
    assert_eq!(host.single_invocations().len(), 2);
    assert!(host.batch_invocations().is_empty());

    match step {
        TurnCompletedStep::Continue { state, .. } => {
            // Charged only for the 2 dispatched calls, landing exactly at
            // the cap (not beyond it).
            assert_eq!(
                state.budget_ledger.capability_invocations_made(),
                policy.max_capability_invocations
            );

            // 4 tool results total: 2 real completions plus 2 blocked
            // results naming the exhausted budget — tool_use/tool_result
            // pairing holds for the whole model-emitted batch.
            let appended = host.appended_result_refs();
            assert_eq!(appended.len(), 4);
            let blocked_count = appended
                .iter()
                .filter(|request| {
                    request
                        .safe_summary
                        .contains("capability-invocation budget")
                })
                .count();
            assert_eq!(blocked_count, 2);

            // The next BudgetStage iteration hard-stops the run through the
            // existing capability-invocation-limit exit — no new exit path
            // was introduced.
            match BudgetStage
                .process(ctx, BudgetInput { state: *state })
                .await
                .expect("budget stage")
            {
                BudgetStep::Exit(LoopExit::Failed(failed)) => {
                    assert_eq!(
                        failed.reason_kind,
                        LoopFailureKind::CapabilityInvocationLimit
                    );
                }
                other => panic!("an exhausted capability budget must hard-stop, got {other:?}"),
            }
        }
        TurnCompletedStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn stale_surface_batch_releases_unlaunched_invocation_budget() {
    let result = |suffix: &str| {
        resolution::completed(
            LoopResultRef::new(format!("result:stale-budget-{suffix}")).expect("valid"),
            format!("{suffix} completed"),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )
    };
    let host = MockHost::new(Vec::new())
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![result("first"), result("second")],
            stopped_on_suspension: false,
        }])
        .fail_batch_with(AgentLoopHostErrorKind::StaleSurface);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let policy = host
        .run_context()
        .resolved_run_profile
        .resource_budget_policy
        .clone();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(policy.max_capability_invocations - 2);
    let surface = ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
        &host,
        VisibleCapabilityRequest,
    )
    .await
    .expect("visible surface");
    let calls = || match two_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected capability calls"),
    };

    let first = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state,
                surface: surface.clone(),
                calls: calls(),
            },
        )
        .await
        .expect("stale batch remains model-visible");
    let TurnCompletedStep::Continue { state, .. } = first else {
        panic!("stale batch must continue");
    };
    assert_eq!(
        state.budget_ledger.capability_invocations_made(),
        policy.max_capability_invocations - 2,
        "a stale surface launches no calls and must release the full reservation"
    );

    host.clear_batch_failure();
    let second = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state: *state,
                surface,
                calls: calls(),
            },
        )
        .await
        .expect("the next batch retains the remaining budget");
    let TurnCompletedStep::Continue { state, .. } = second else {
        panic!("successful batch must continue");
    };
    assert_eq!(
        state.budget_ledger.capability_invocations_made(),
        policy.max_capability_invocations
    );
    assert_eq!(host.batch_invocations().len(), 2);
}

/// Behavior tightening absorbed by the `BudgetLedger` refactor: a capability
/// retry dispatch now charges the invocation budget through the same
/// chokepoint the initial batch dispatch uses, BEFORE re-dispatching. Before
/// this fix, the retry dispatch (`executor/capabilities.rs`, the
/// `RecoveryOutcome::Retry` arm inside `handle_capability_error`) counted
/// unconditionally with no enforcement, so a retry could re-dispatch to the
/// host even when the run's capability-invocation budget was already
/// exhausted. This pins: when the budget has no remaining allowance at the
/// moment of the retry, the retry is never dispatched — it falls through to
/// the same model-visible blocked-result path `ToolErrorResult` uses, the
/// counter is not double-charged, and no new exit path is introduced (the
/// next `BudgetStage` iteration still hard-stops through the existing
/// `CapabilityInvocationLimit` exit).
#[tokio::test]
async fn capability_retry_does_not_dispatch_when_invocation_budget_is_exhausted() {
    let call = CapabilityCallCandidate {
        activity_id: CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: capability_id(),
        input_ref: CapabilityInputRef::new("input:retry-budget-exhausted").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            provider_turn_id: "turn_retry_budget".to_string(),
            provider_call_id: "call_retry_budget".to_string(),
            provider_tool_name: ProviderToolName::new("demo__echo").expect("provider tool name"),
            arguments: serde_json::json!({ "message": "retry-budget-exhausted" }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }),
    };

    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::Transient,
                "temporary failure".to_string(),
                diagnostic_failure_detail("temporary failure"),
            )],
            stopped_on_suspension: false,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let policy = host
        .run_context()
        .resolved_run_profile
        .resource_budget_policy
        .clone();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    // Exactly one invocation remains: the initial dispatch charges it and
    // lands the counter at the cap, leaving nothing for the retry.
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(policy.max_capability_invocations - 1);

    let surface = ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
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
                calls: vec![call],
            },
        )
        .await
        .expect("capability stage");

    // No retry dispatch reached the host at all — batch dispatch used the
    // batch channel (asserted via batch_invocations below), and the retry
    // never issued a single-call dispatch.
    assert!(host.single_invocations().is_empty());
    assert_eq!(host.batch_invocations().len(), 1);

    match step {
        TurnCompletedStep::Continue { state, .. } => {
            // Charged only once (the initial dispatch); the exhausted retry
            // charged nothing, so the counter stays exactly at the cap
            // rather than going one further as it would have before this
            // fix (which would have silently exceeded the budget).
            assert_eq!(
                state.budget_ledger.capability_invocations_made(),
                policy.max_capability_invocations
            );

            let appended = host.appended_result_refs();
            assert_eq!(appended.len(), 1);

            // The next BudgetStage iteration hard-stops the run through the
            // existing capability-invocation-limit exit — no new exit path
            // was introduced.
            match BudgetStage
                .process(ctx, BudgetInput { state: *state })
                .await
                .expect("budget stage")
            {
                BudgetStep::Exit(LoopExit::Failed(failed)) => {
                    assert_eq!(
                        failed.reason_kind,
                        LoopFailureKind::CapabilityInvocationLimit
                    );
                }
                other => panic!("an exhausted capability budget must hard-stop, got {other:?}"),
            }
        }
        TurnCompletedStep::Exit(exit) => panic!("expected continue, got {exit:?}"),
    }
}

#[tokio::test]
async fn iteration_limit_gives_model_one_warning_turn_to_finish() {
    let host = MockHost::new(vec![reply_response_with_text(
        "completed during the final iteration",
    )]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &crate::families::default_with_iteration_limit(0),
            &host,
            state,
        )
        .await
        .expect("iteration-limit warning turn should execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        !requests[0]
            .capability_view
            .as_ref()
            .expect("warning request has a capability view")
            .visible_capability_ids
            .is_empty(),
        "the warning uses a normal tool-capable model request"
    );
    assert!(requests[0].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("final recovery iteration")
    }));
}

#[tokio::test]
async fn iteration_warning_survives_before_model_checkpoint_reload() {
    let host = Arc::new(
        MockHost::new(vec![reply_response_with_text(
            "completed after checkpoint reload",
        )])
        .crash_after_checkpoint_progress(LoopCheckpointKind::BeforeModel),
    );
    let crashed_host = Arc::clone(&host);
    let crash = tokio::spawn(async move {
        CanonicalAgentLoopExecutor
            .execute_family(
                &crate::families::default_with_iteration_limit(0),
                crashed_host.as_ref(),
                LoopExecutionState::initial_for_run(crashed_host.run_context()),
            )
            .await
    })
    .await
    .expect_err("scripted worker crash must stop before the model request");
    assert!(crash.is_panic());

    let restored = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeModel);
    assert!(restored.terminal_warning_state.pending().is_some());
    assert!(host.model_requests().is_empty());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &crate::families::default_with_iteration_limit(0),
            host.as_ref(),
            restored,
        )
        .await
        .expect("checkpointed warning should reach the resumed model request");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let mut final_state = final_staged_state(&host);
    assert!(final_state.terminal_warning_state.pending().is_none());
    assert!(
        !final_state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::iteration_limit(0))
    );
    let requests = host.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("final recovery iteration")
    }));
}

#[tokio::test]
async fn terminal_warning_survives_model_budget_approval_and_reaches_resumed_request() {
    let gate_ref = LoopGateRef::new("gate:terminal-warning-budget").expect("gate ref");
    let host = MockHost::new(vec![reply_response_with_text(
        "completed after budget approval",
    )])
    .with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetApprovalRequired,
            "budget approval required",
        )
        .with_gate_ref(gate_ref),
    ]);
    let family = crate::families::default_with_iteration_limit(0);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let blocked = CanonicalAgentLoopExecutor
        .execute_family(&family, &host, state)
        .await
        .expect("warning request should block for budget approval");
    assert!(matches!(blocked, LoopExit::Blocked(_)));

    let restored = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        restored.terminal_warning_state.pending().is_some(),
        "a gate raised before provider dispatch must not consume the warning"
    );

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&family, &host, restored)
        .await
        .expect("approved retry should receive the pending warning");
    assert!(matches!(exit, LoopExit::Completed(_)));

    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].inline_messages.iter().any(|message| {
        message
            .safe_body
            .as_str()
            .contains("final recovery iteration")
    }));
}

#[tokio::test]
async fn budget_iteration_limit_schedules_normal_warning_turn() {
    let host = MockHost::new(Vec::new()).with_driver_nudges_enabled();
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
        .process(ctx, BudgetInput { state })
        .await
        .expect("budget stage");

    let BudgetStep::Continue { state, .. } = step else {
        panic!("first iteration-limit terminal should schedule a warning turn");
    };
    assert!(state.terminal_warning_state.pending().is_some());
    assert!(host.model_requests().is_empty());
}

#[tokio::test]
async fn scheduled_question_after_nudge_budget_fails_and_retains_replies() {
    let questions = [
        "Which repository should I inspect?",
        "Should I inspect the main branch?",
        "Would you like me to continue?",
    ];
    let host = MockHost::new(vec![
        reply_response_with_text(questions[0]),
        reply_response_with_text(questions[1]),
        reply_response_with_text(questions[2]),
    ])
    .with_driver_nudges_enabled()
    .with_scheduled_trigger_origin();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let LoopExit::Failed(failed) = exit else {
        panic!("expected invalid scheduled output to fail, got {exit:?}");
    };
    assert_eq!(failed.reason_kind, LoopFailureKind::InvalidModelOutput);
    assert_eq!(final_staged_state(&host).completion_nudges_used, 2);
    let finalized = host.finalized_assistant_messages();
    for question in questions {
        assert!(finalized.iter().any(|message| message == question));
    }
    assert!(
        !failed.explanation_message_refs.is_empty(),
        "the failed exit must retain assistant transcript evidence"
    );
}

#[tokio::test]
async fn repeated_model_budget_accounting_failure_preserves_typed_terminal_diagnostics() {
    let accounting_error = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetAccountingFailed,
            "resource accounting storage is unavailable",
        )
    };
    let host =
        MockHost::new(Vec::new()).with_model_errors(vec![accounting_error(), accounting_error()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("a repeated accounting outage must remain a typed host failure");

    assert_eq!(host.model_requests().len(), 2);
    assert!(
        host.model_requests()[1]
            .inline_messages
            .iter()
            .any(|message| message
                .safe_body
                .as_str()
                .contains("resource accounting failed"))
    );
    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::BudgetAccountingFailed,
            safe_summary: LoopSafeSummary::new("resource accounting storage is unavailable")
                .expect("safe"),
            reason_kind: None,
            detail: None,
        }
    );
}
