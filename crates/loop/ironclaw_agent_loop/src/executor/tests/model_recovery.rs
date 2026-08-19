use super::*;

#[tokio::test]
async fn model_shrink_context_call_scope_returns_planner_contract() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContextOverflow,
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

/// Behavior tightening absorbed by the `BudgetLedger` refactor: a model-call
/// retry now charges the model-call budget through the same chokepoint the
/// first dispatch of an iteration uses, BEFORE re-dispatching. Before this
/// fix (`executor/model.rs`, the per-attempt dispatch inside `ModelStage`)
/// the counter incremented unconditionally with no enforcement, so a
/// call-scope retry (e.g. `Unavailable`) could keep dispatching to the
/// provider even once the run's model-call budget was exhausted; only the
/// NEXT outer-loop `BudgetStage` pass would have noticed. This pins: when
/// the budget has no remaining allowance at the moment of a retry attempt,
/// the attempt is converted to `ModelStep::RetryIteration` instead of
/// dispatching — re-entering the outer loop, where `BudgetStage` then
/// hard-stops the run through its existing `ModelCallLimit` exit. No new
/// exit path is introduced.
#[tokio::test]
async fn model_retry_returns_to_outer_loop_when_call_budget_is_exhausted() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "model unavailable",
    )]);
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
    // Exactly one model call remains: the first dispatch attempt charges it
    // and lands the counter at the cap, leaving nothing for the call-scope
    // retry `Unavailable` would otherwise trigger.
    state
        .budget_ledger
        .set_model_calls_made_for_test(policy.max_model_calls - 1);

    let step = ModelStage
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
        .expect("model stage");

    let retried_state = match step {
        ModelStep::RetryIteration(state) => state,
        other => panic!(
            "an exhausted model-call budget mid-retry must re-enter the outer loop, got {}",
            match other {
                ModelStep::Response(..) => "Response",
                ModelStep::RetryIteration(_) => unreachable!(),
                ModelStep::Exit(_) => "Exit",
            }
        ),
    };
    // Exactly one dispatch reached the host — the exhausted retry attempt
    // was never sent to the provider.
    assert_eq!(host.model_requests().len(), 1);
    // Charged only once (the first attempt); the exhausted retry charged
    // nothing, so the counter stays exactly at the cap rather than going one
    // further as it would have before this fix.
    assert_eq!(
        retried_state.budget_ledger.model_calls_made(),
        policy.max_model_calls
    );

    // The next BudgetStage iteration hard-stops the run through the
    // existing model-call-limit exit — no new exit path was introduced.
    match BudgetStage
        .process(
            ctx,
            BudgetInput {
                state: *retried_state,
            },
        )
        .await
        .expect("budget stage")
    {
        BudgetStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelCallLimit);
        }
        other => panic!("an exhausted model-call budget must hard-stop, got {other:?}"),
    }
}

#[tokio::test]
async fn model_unrecoverable_host_error_preserves_inline_sanitized_cause() {
    let cause = "credential provider refused the configured model identity";
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::CredentialUnavailable,
            "model credentials are unavailable",
        )
        .with_detail(cause),
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
            detail: Some(cause.to_string()),
        }
    );
}

#[tokio::test]
async fn model_budget_accounting_failure_gets_one_durable_final_model_turn() {
    let accounting_error = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetAccountingFailed,
            "resource accounting storage is unavailable",
        )
    };
    let host = MockHost::new(vec![reply_response_with_text(
        "completed after accounting recovery",
    )])
    .with_model_errors(vec![accounting_error()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("one recovered accounting failure should reach a final model turn");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["completed after accounting recovery".to_string()]
    );
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].inline_messages.iter().any(|message| {
            message
                .safe_body
                .as_str()
                .contains("resource accounting failed")
        }),
        "the recovered request must explain why it is the one final model turn"
    );
    assert!(
        !requests[1]
            .capability_view
            .as_ref()
            .expect("warning request has a capability view")
            .visible_capability_ids
            .is_empty(),
        "the warning uses the normal tool-capable model request"
    );
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::Final,
        ],
        "the consumed warning budget is durable before the warned request"
    );
    let mut final_state = final_staged_state(&host);
    assert!(
        !final_state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::budget_accounting_failed())
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
    let host =
        MockHost::new(Vec::new()).with_model_errors(vec![stale(), stale(), stale(), stale()]);
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
        4,
        "stale-request retries are bounded by the per-class budget plus one observation turn"
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
        AgentLoopExecutorError::CheckpointRejected {
            stage: CheckpointKind::BeforeModel,
            safe_summary,
        } if safe_summary.as_str() == "scripted checkpoint failure"
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
async fn unsupported_checkpointed_terminal_warning_stops_before_model_call() {
    let host = MockHost::new(vec![reply_response()]);
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    let mut observation = TerminalWarningObservation::iteration_limit(8);
    observation.schema_version += 1;
    assert!(state.terminal_warning_state.schedule(observation));
    let payload = serde_json::to_vec(&state).expect("checkpoint state serializes");
    let restored =
        LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
            .expect("checkpoint state reloads before semantic validation");

    let error = CanonicalAgentLoopExecutor
        .execute_family(&crate::families::default(), &host, restored)
        .await
        .expect_err("unsupported terminal warning must fail prompt construction");

    assert!(matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "terminal warning control text was invalid"
        }
    ));
    assert!(host.model_requests().is_empty());
}

#[tokio::test]
async fn pending_no_progress_warning_preempts_iteration_warning_until_delivered() {
    let host = MockHost::new(vec![reply_response_with_text(
        "completed after the original warning",
    )]);
    let family = crate::families::default_with_iteration_limit(0);
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    assert!(
        state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::no_progress(None, None))
    );

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&family, &host, state)
        .await
        .expect("the original pending warning should reach the model");
    assert!(matches!(exit, LoopExit::Completed(_)));

    let requests = host.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .inline_messages
            .iter()
            .any(|message| { message.safe_body.as_str().contains("no progress detected") })
    );
    assert!(
        !requests[0]
            .inline_messages
            .iter()
            .any(|message| { message.safe_body.as_str().contains("iteration limit") })
    );
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
    assert_eq!(
        host.progress_events()
            .into_iter()
            .filter(|event| matches!(
                event,
                LoopProgressEvent::FailureRecovered {
                    sequence: 1,
                    stage: LoopRecoveryStage::Model,
                    class: LoopRecoveryClass::ModelContentFiltered,
                    disposition: LoopRecoveryDisposition::ModelVisible,
                }
            ))
            .count(),
        1,
        "one model-visible recovery must emit one durable numerator event"
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
async fn model_unavailable_retry_advances_fallback_and_accepts_authoritative_evidence() {
    let host = MockHost::new(vec![reply_response()]).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model provider is temporarily unavailable",
        )
        .with_next_fallback_index(2),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("fallback retry completes");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].fallback_index, 0);
    assert_eq!(requests[1].fallback_index, 2);
    assert_eq!(final_staged_state(&host).model_state.fallback_index, 2);
}

#[tokio::test]
async fn model_unavailable_retry_rejects_a_non_advancing_fallback_index() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model provider is temporarily unavailable",
        )
        .with_next_fallback_index(0),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("a host-selected fallback index must advance the current route");

    assert!(matches!(
        error,
        AgentLoopExecutorError::PlannerContract {
            detail: "fallback model route index did not advance"
        }
    ));
}

#[tokio::test]
async fn model_unavailable_without_fallback_evidence_retries_the_current_route() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model provider is temporarily unavailable",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("same-route availability retry completes");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let requests = host.model_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].fallback_index, 0);
    assert_eq!(requests[1].fallback_index, 0);
    assert_eq!(final_staged_state(&host).model_state.fallback_index, 0);
}

#[tokio::test(start_paused = true)]
async fn model_error_abort_skips_explanation_and_carries_partial_refs() {
    // Availability-class model errors retry on the deep availability budget
    // before aborting; paused time fast-forwards the backoff sleeps.
    let abort_call_count = crate::strategies::DefaultRecoveryStrategy::default()
        .max_model_availability_attempts as usize
        + 2;
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
async fn exhausted_model_unavailability_gets_one_observation_and_recovers() {
    let family = crate::families::default_with_overrides(
        crate::families::FamilyOverrides::default().set_model_availability_attempts(1),
    );
    let unavailable = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model service is unavailable",
        )
    };
    let host = MockHost::new(vec![reply_response_with_text(
        "continued after provider recovery",
    )])
    .with_model_errors(vec![unavailable(), unavailable()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("availability observation should let the recovered provider continue");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.model_requests()
            .iter()
            .flat_map(|request| &request.inline_messages)
            .filter(|message| message
                .safe_body
                .as_str()
                .contains("model error observation"))
            .count(),
        1
    );
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["continued after provider recovery"]
    );
}

#[tokio::test]
async fn exhausted_stale_model_request_gets_one_observation_and_recovers() {
    let stale = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::StaleSurface,
            "model request surface is stale",
        )
    };
    let host = MockHost::new(vec![reply_response_with_text(
        "continued with the refreshed surface",
    )])
    .with_model_errors(vec![stale(), stale(), stale()]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("stale-request observation should allow one refreshed iteration");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.model_requests()
            .iter()
            .flat_map(|request| &request.inline_messages)
            .filter(|message| message
                .safe_body
                .as_str()
                .contains("model error observation"))
            .count(),
        1
    );
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["continued with the refreshed surface"]
    );
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
    let cause = "provider connection timed out before a response arrived";
    // Exactly attempts+2 scripted failures: exhaustion gets one observation
    // attempt, and the final call proves the bounded strategy then aborts. Any
    // extra call would
    // hit the mock's script-exhausted Internal fallback and change the
    // observed failure category.
    let host = MockHost::new(Vec::new()).with_model_errors(
        (0..attempts.saturating_add(2))
            .map(|_| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable")
                    .with_detail(cause)
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
                failed
                    .safe_summary
                    .as_ref()
                    .and_then(|summary| summary.detail()),
                Some(cause),
                "abort must inline the model error's bounded cause"
            );
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        (attempts + 2) as usize,
        "the loop must allow the configured availability budget plus one observation turn"
    );
}

#[tokio::test]
async fn retry_uses_single_call_invocation() {
    for error_kind in [FailureKind::Transient, FailureKind::Network] {
        let host = MockHost::new(vec![calls_response()])
            .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    error_kind,
                    "temporary failure".to_string(),
                    diagnostic_failure_detail("temporary failure"),
                )],
                stopped_on_suspension: false,
            }])
            .with_single_outcomes(vec![resolution::completed(
                LoopResultRef::new("result:retry").expect("valid"),
                "retry completed".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        let final_state = final_staged_state(&host);
        assert_eq!(final_state.recovery_state, Default::default());
        // Accounting invariant (executor/capabilities.rs): "every invocation
        // that reaches dispatch counts, whatever its outcome." The initial
        // batch dispatch (1 call) plus the one same-call retry dispatch must
        // both count toward the budget, or a caller retrying failed calls
        // silently escapes the invocation budget.
        assert_eq!(
            final_state.budget_ledger.capability_invocations_made(),
            2,
            "initial dispatch (1) + one retry dispatch (1) must both count"
        );
    }
}

#[tokio::test]
async fn model_visible_provider_tool_failures_append_failure_tool_result_for_replay() {
    for (error_kind, safe_summary, expected_summary) in [
        (
            FailureKind::InputEncode,
            "invalid input",
            "capability failed with input_encode: invalid input",
        ),
        (
            FailureKind::InputEncode,
            "provider arguments failed schema validation at instance path root against schema path required",
            "capability failed with input_encode: provider arguments failed schema validation at instance path root against schema path required",
        ),
        (
            FailureKind::MissingRuntime,
            "runtime missing",
            "capability failed with missing_runtime: runtime missing",
        ),
        (
            FailureKind::OperationFailed,
            "operation failed",
            "capability failed with operation_failed: operation failed",
        ),
        (
            FailureKind::OutputTooLarge,
            "response body exceeded limit 10000000",
            "capability failed with output_too_large: response body exceeded limit 10000000",
        ),
    ] {
        let host = MockHost::new(vec![provider_calls_response(), reply_response()])
            .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![resolution::failed(
                    error_kind,
                    safe_summary.to_string(),
                    diagnostic_failure_detail(safe_summary),
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::OutputTooLarge,
                long_summary,
                diagnostic_failure_detail("capability output exceeded the allowed size"),
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
