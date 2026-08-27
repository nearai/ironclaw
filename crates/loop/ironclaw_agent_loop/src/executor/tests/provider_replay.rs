use super::*;

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

/// A denial must reach the model with something it can act on.
///
/// Denials passed `model_observation: None`, so the model got a summary string
/// and nothing structured — no recovery, no retry constraint, no repairs. A
/// denial meaning *authenticate and this works* was indistinguishable from a
/// permanent block (#6284 item 4). This drives a real denial through the
/// executor and asserts the appended result carries a recovery observation
/// naming the next move.
#[tokio::test]
async fn a_denial_tells_the_model_what_would_unlock_it() {
    // `auth_denied` is minted by the capability port for a real authorization
    // failure; #6781 maps it to `DenyReason::UnknownSecret`. Provider replay
    // metadata is required for a denial to mint a result ref, so this uses the
    // two-provider-call shape (one completed, one denied) that
    // `denied_provider_call_appends_failure_tool_result_for_replay` uses.
    let result_ref = LoopResultRef::new("result:denial-hint-ok").expect("valid");
    let host = MockHost::new(vec![provider_two_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    result_ref.clone(),
                    "provider call completed".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    true,
                    0,
                    None,
                    None,
                ),
                resolution::denied(
                    ironclaw_loop_contracts::CapabilityDeniedReasonKind::unknown("auth_denied")
                        .expect("valid reason tag"),
                    // Deliberately avoids the word "credential": the summary
                    // channel's credential-marker guard would scrub it to a
                    // placeholder, which is orthogonal to what this pins.
                    "sign-in required for this provider".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 2, "completed call plus the denial");
    let denial_result = &appended[1];
    let observation = denial_result
        .model_observation
        .as_ref()
        .expect("a denial must carry a model observation, not None");
    let recovery = observation
        .recovery
        .as_ref()
        .expect("a denial observation must carry recovery guidance");

    // The specific action, not a generic "obey the constraint".
    assert_eq!(
        recovery.recovery_hint,
        CapabilityRecoveryHint::AuthenticateThenRetry,
        "a credential-missing denial must point the model at an auth flow"
    );
    assert!(
        recovery.recovery_hint.names_an_action(),
        "the denial hint must name a concrete next move"
    );
    assert_eq!(recovery.same_call_retry, SameCallRetryConstraint::Forbidden);
}

#[tokio::test]
async fn denial_without_summary_emits_actionable_detail() {
    let host = MockHost::new(vec![provider_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![ironclaw_host_api::resolution::Resolution::Denied(
                Denial::new(DenyRef::new()).with_reason_kind(DenyReason::PolicyDenied),
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
    let observation = appended
        .first()
        .and_then(|result| result.model_observation.as_ref())
        .expect("summary-less denial must append a model-visible observation");
    let ToolObservationDetail::GenericFailure {
        failure_kind,
        detail,
    } = &observation.detail
    else {
        panic!("summary-less denial must carry generic failure detail");
    };
    assert_eq!(*failure_kind, FailureKind::PolicyDenied);
    assert_eq!(
        detail.as_deref(),
        Some(
            "The host denied this capability with reason policy_denied; follow the recovery \
             guidance before choosing the next action."
        )
    );
}

#[tokio::test]
async fn policy_denied_capability_error_honors_retry_recovery() {
    let host = MockHost::new(vec![calls_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::denied(
                    ironclaw_loop_contracts::CapabilityDeniedReasonKind::EmptySurface,
                    "provider call denied".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        }])
        .with_single_outcomes(vec![resolution::completed(
            LoopResultRef::new("result:policy-retry").expect("valid"),
            "policy retry completed".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
    let recovered = host
        .progress_events()
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                LoopProgressEvent::FailureRecovered {
                    sequence: 1,
                    stage: LoopRecoveryStage::Capability,
                    class: LoopRecoveryClass::Capability(FailureKind::PolicyDenied),
                    disposition: LoopRecoveryDisposition::Retried,
                }
            )
        })
        .count();
    assert_eq!(
        recovered, 1,
        "one applied capability retry must emit exactly one recovery numerator"
    );
}

#[tokio::test]
async fn spawned_process_fails_closed_until_process_wait_contract_exists() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
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
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Transcript,
            kind: AgentLoopHostErrorKind::TranscriptWriteFailed,
            safe_summary: LoopSafeSummary::assistant_transcript_write_failed(),
            reason_kind: None,
            detail: None,
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
        ironclaw_host_api::resolution::ResolutionBatch {
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
        ironclaw_host_api::safe_summary::SafeSummary::placeholder().as_str()
    );
    assert_ne!(appended[0].safe_summary, "/Users/alice/.ssh/id_rsa");
}

#[tokio::test]
async fn completed_provider_call_appends_provider_replay_metadata() {
    let result_ref = LoopResultRef::new("result:provider-call").expect("valid");
    let safe_summary = "a".repeat(300);
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                safe_summary.clone(),
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
            structured_json_view: false,
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    result_ref.clone(),
                    "provider call completed".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    true,
                    0,
                    None,
                    None,
                ),
                resolution::denied(
                    ironclaw_loop_contracts::CapabilityDeniedReasonKind::EmptySurface,
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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::InputEncode,
                "provider arguments failed schema validation".to_string(),
                CapabilityFailureDetail::InvalidInput {
                    issues: vec![CapabilityInputIssue {
                        path: "file_path".to_string(),
                        code: DispatchInputIssueCode::MissingRequired,
                        expected: Some("required field".to_string()),
                        received: None,
                        schema_path: Some("required".to_string()),
                    }],
                },
            )],
            stopped_on_suspension: false,
        }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(
            &family_requiring_structured_capability_observation(),
            &host,
            state,
        )
        .await
        .expect("execute");
    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "the real caller must feed the strategy the same structured observation it appends"
    );

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
        ironclaw_host_api::resolution::ResolutionBatch {
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
