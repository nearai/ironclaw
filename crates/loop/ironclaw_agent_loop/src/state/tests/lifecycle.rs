use super::checkpoint_wire::test_run_context;
use super::*;
use serde_json::json;

#[test]
fn rebase_for_run_resets_run_owned_refs_and_input_cursor() {
    let source_context = test_run_context();
    let target_context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&source_context);
    state.input_cursor = LoopInputCursor::from_host_token(
        &source_context,
        ironclaw_loop_contracts::LoopInputCursorToken::new("input-cursor:source-seen").unwrap(),
    );
    state
        .assistant_refs
        .push(LoopMessageRef::new("msg:source-run").unwrap());
    state
        .result_refs
        .push(ironclaw_host_api::turn::LoopResultRef::new("result:source-run").unwrap());
    state.iteration = 4;
    state.cumulative_model_usage = Some(ironclaw_loop_contracts::LoopModelUsage {
        input_tokens: 100,
        output_tokens: 50,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    });
    // Gate-bound resume state must survive the rebase: this path also
    // resumes a run after an approval/auth gate, where the pending-resume
    // record drives re-dispatch of the gated capability.
    state.last_gate = Some(LoopGateRef::new("gate:source-run").unwrap());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:source-approval").unwrap(),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").unwrap(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("00000000-0000-0000-0000-000000000002").unwrap(),
        activity_id: CapabilityActivityId::new(),
        correlation_id: CorrelationId::new(),
        surface_version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        input_ref: CapabilityInputRef::new("input:source-approval").unwrap(),
        effective_capability_ids: vec![],
        provider_replay: None,
        disposition: None,
    });
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:source-auth").unwrap(),
        capability_id: CapabilityId::new("gsuite.calendar.list_events").unwrap(),
        surface_version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        input_ref: CapabilityInputRef::new("input:source").unwrap(),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });

    let rebased = state.clone().rebase_for_run(&target_context);

    assert_eq!(rebased.iteration, state.iteration);
    assert!(rebased.input_cursor.is_for_run(&target_context));
    assert_eq!(
        rebased.input_cursor,
        LoopInputCursor::origin_for_run(&target_context)
    );
    assert!(rebased.assistant_refs.is_empty());
    assert!(rebased.result_refs.is_empty());
    // A retry rebases onto a different run id, so the failed run's token
    // total must be dropped rather than re-reported under the new run.
    assert!(rebased.cumulative_model_usage.is_none());
    // Gate-bound resume state is preserved so an approval/auth resume can
    // re-dispatch the gated capability.
    assert_eq!(rebased.last_gate, state.last_gate);
    assert_eq!(
        rebased.pending_approval_resume,
        state.pending_approval_resume
    );
    assert_eq!(rebased.pending_auth_resume, state.pending_auth_resume);
}

#[test]
fn rebase_for_run_drops_no_progress_observations_and_warning_for_a_new_run() {
    let source_context = test_run_context();
    let target_context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&source_context);
    let repeated_call_signature = CapabilityCallSignature::from_call(
        CapabilityId::new("demo.echo").expect("valid capability id"),
        &json!({"value": 1}),
    )
    .expect("valid call signature");
    state
        .recent_call_signatures
        .push(repeated_call_signature.clone());
    state.stop_state.repeated_call_warning =
        Some(RepeatedCallWarningState::rendered(repeated_call_signature));
    state
        .seen_capability_output_digests
        .push(CapabilityOutputObservation {
            signature: CapabilityCallSignature::from_call(
                CapabilityId::new("demo.echo").expect("valid capability id"),
                &json!({"value": 1}),
            )
            .expect("valid call signature"),
            output_digest: ironclaw_loop_contracts::ContentDigest(42),
        });
    assert!(
        state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::no_progress(Some(8), None))
    );
    state.stop_state.trailing_no_progress_results = 1;
    state.pending_model_error_observation = Some(ModelErrorRecoveryObservation::transient());
    state.pending_model_retry_directive = Some(PendingModelRetryDirective::RepairInvalidOutput);
    state.recovery_state =
        RecoveryStrategyState::with_attempts_for(RecoveryAttemptClass::ModelTransient, 2);

    let rebased = state.clone().rebase_for_run(&target_context);

    assert!(rebased.recent_call_signatures.is_empty());
    assert!(rebased.stop_state.repeated_call_warning.is_none());
    assert!(rebased.seen_capability_output_digests.is_empty());
    assert_eq!(
        rebased.terminal_warning_state,
        TerminalWarningState::default()
    );
    assert_eq!(rebased.stop_state.trailing_no_progress_results, 0);
    assert_eq!(
        rebased.pending_model_error_observation,
        state.pending_model_error_observation
    );
    assert_eq!(
        rebased.pending_model_retry_directive,
        state.pending_model_retry_directive
    );
    assert_eq!(rebased.recovery_state, state.recovery_state);
}

#[test]
fn rebase_for_run_resets_per_run_budget_counters_for_a_different_run() {
    // A retry rebases the failed run's checkpoint onto a fresh TurnRunId.
    // run_started_at/model_calls_made/capability_invocations_made are
    // per-run budget accounting (see their doc comments on
    // LoopExecutionState); carrying an exhausted counter or a stale
    // wall-clock start into the retry would make it fail the budget
    // stage immediately, before it does any work.
    let source_context = test_run_context();
    let target_context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&source_context);
    state
        .budget_ledger
        .set_run_started_at_for_test(Some(chrono::Utc::now() - chrono::Duration::seconds(120)));
    state.budget_ledger.set_model_calls_made_for_test(32);
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(64);

    let rebased = state.rebase_for_run(&target_context);

    assert!(rebased.budget_ledger.run_started_at().is_none());
    assert_eq!(rebased.budget_ledger.model_calls_made(), 0);
    assert_eq!(rebased.budget_ledger.capability_invocations_made(), 0);
}

#[test]
fn rebase_for_run_preserves_refs_for_same_run_gate_resume() {
    let context = test_run_context();
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.input_cursor = LoopInputCursor::from_host_token(
        &context,
        ironclaw_loop_contracts::LoopInputCursorToken::new("input-cursor:gate-seen").unwrap(),
    );
    state
        .assistant_refs
        .push(LoopMessageRef::new("msg:same-run").unwrap());
    state
        .result_refs
        .push(ironclaw_host_api::turn::LoopResultRef::new("result:same-run").unwrap());
    state.iteration = 3;
    // A same-run gate resume must preserve the run's accumulated token
    // total; the full-equality assertion below locks that in.
    state.cumulative_model_usage = Some(ironclaw_loop_contracts::LoopModelUsage {
        input_tokens: 100,
        output_tokens: 50,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    });
    // A same-run gate resume must also preserve per-run budget
    // accounting: the full-equality assertion below locks that in
    // alongside the token total above.
    state
        .budget_ledger
        .set_run_started_at_for_test(Some(chrono::Utc::now() - chrono::Duration::seconds(120)));
    state.budget_ledger.set_model_calls_made_for_test(5);
    state
        .budget_ledger
        .set_capability_invocations_made_for_test(7);
    state
        .seen_capability_output_digests
        .push(CapabilityOutputObservation {
            signature: CapabilityCallSignature::from_call(
                CapabilityId::new("demo.echo").expect("valid capability id"),
                &json!({"value": 1}),
            )
            .expect("valid call signature"),
            output_digest: ironclaw_loop_contracts::ContentDigest(42),
        });
    let repeated_call_signature = CapabilityCallSignature::from_call(
        CapabilityId::new("demo.echo").expect("valid capability id"),
        &json!({"value": 1}),
    )
    .expect("valid call signature");
    state
        .recent_call_signatures
        .push(repeated_call_signature.clone());
    state.stop_state.repeated_call_warning =
        Some(RepeatedCallWarningState::rendered(repeated_call_signature));
    assert!(
        state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::no_progress(Some(8), None))
    );
    state.stop_state.trailing_no_progress_results = 1;

    let rebased = state.clone().rebase_for_run(&context);

    assert_eq!(rebased.recent_call_signatures, state.recent_call_signatures);
    assert_eq!(
        rebased.stop_state.repeated_call_warning,
        state.stop_state.repeated_call_warning
    );
    assert_eq!(rebased, state);
}
