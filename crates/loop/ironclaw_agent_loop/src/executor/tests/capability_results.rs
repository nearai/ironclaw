use super::*;

/// A FAILED result-tool attempt must not complete a structured run: the
/// error path records no completed signature, so the stop strategy keeps
/// the run alive for the repair retry, counts the all-failed batch, and
/// aborts as invalid model output only after the threshold. (Recording
/// errored calls into `observed_signatures` completed structured runs off
/// a failed validation attempt with NO durable result.)
#[tokio::test]
async fn structured_stop_ignores_failed_result_attempts_and_counts_all_failed_batches() {
    use crate::state::CapabilityCallSignature;
    use crate::strategies::{
        CapabilityBatchTurnSummary, StopConditionStrategy as _, StopKind, StopOutcome,
        StructuredResultStopStrategy, TurnEndKind, TurnSummary,
    };
    use ironclaw_host_api::ids::CapabilityId;

    let host = MockHost::new(Vec::new());
    let strategy = StructuredResultStopStrategy::new(
        CapabilityId::new("builtin.structured_result").expect("valid"),
    );
    let state = LoopExecutionState::initial_for_run(host.run_context());

    // The only invocation in the batch FAILED: no completed signature.
    let failed_batch = TurnSummary {
        kind: TurnEndKind::AfterCapabilityBatch,
        assistant_message_ref: None,
        batch_result_refs: Vec::new(),
        capability_batch: CapabilityBatchTurnSummary {
            invocation_count: 1,
            terminate_hint_count: 0,
            observed_signatures: Vec::new(),
        },
    };
    let observed = strategy.observe_completed_turn(&state, &failed_batch).await;
    assert_eq!(
        observed.trailing_all_failed_batches, 1,
        "an all-failed batch must count toward the abort threshold"
    );
    assert!(
        matches!(
            strategy
                .should_stop_after_observed_turn(&state, &failed_batch)
                .await,
            StopOutcome::Continue {}
        ),
        "a failed result attempt must keep the run alive for the repair retry"
    );

    let mut exhausted = state.clone();
    exhausted.stop_state.trailing_all_failed_batches = 3;
    assert!(matches!(
        strategy
            .should_stop_after_observed_turn(&exhausted, &failed_batch)
            .await,
        StopOutcome::Stop {
            kind: StopKind::Aborted(ironclaw_loop_contracts::LoopFailureKind::InvalidModelOutput)
        }
    ));

    // A COMPLETED result-tool call stops the run gracefully.
    let completed_batch = TurnSummary {
        capability_batch: CapabilityBatchTurnSummary {
            invocation_count: 1,
            terminate_hint_count: 1,
            observed_signatures: vec![
                CapabilityCallSignature::from_call(
                    CapabilityId::new("builtin.structured_result").expect("valid"),
                    &serde_json::json!({"sentiment": "positive"}),
                )
                .expect("signature"),
            ],
        },
        ..failed_batch.clone()
    };
    assert!(matches!(
        strategy
            .should_stop_after_observed_turn(&state, &completed_batch)
            .await,
        StopOutcome::Stop {
            kind: StopKind::GracefulStop
        }
    ));
}

#[tokio::test]
async fn capability_stage_returns_after_batch_summary() {
    let result_ref = LoopResultRef::new("result:done").expect("valid");
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
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
                surface: ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
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
                        observed_signatures: vec![signature],
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
        activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
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
async fn failed_exit_finalizes_explanation_and_failed_exit_refs_partial_first() {
    // Vehicle: iteration-limit abort. The retired capability `Permanent` kind
    // merged into model-visible `OperationFailed` under the unified
    // FailureKind, so a scripted capability failure can no longer end the run;
    // the explanation flow under test is failure-kind-agnostic.
    //
    // The limit is 0, not 1, so the loop's one bounded pre-termination warning
    // turn is the *first* scripted turn: the budget stage schedules the warning
    // before the first model call, and that warning is already spent when the
    // limit is re-checked on the next iteration. The model call immediately
    // after the scripted capability batch is therefore the failure-explanation
    // call, which keeps this test about explanation behavior instead of
    // warning-turn accounting (pinned separately by
    // `iteration_limit_gives_model_one_warning_turn_to_finish`).
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "The run stopped after hitting the iteration limit.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
            "result:limit-1",
        )]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let partial_ref = message_ref("msg:partial-work");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.assistant_refs.push(partial_ref.clone());

    let exit = executor
        .execute_family(&family_with_iteration_limit(0), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::IterationLimit);
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
        vec!["The run stopped after hitting the iteration limit.".to_string()]
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
async fn cancellation_before_explanation_skips_explanation_model_call() {
    // Vehicle: iteration-limit abort at limit 0, so the model call that
    // cancellation must suppress is the failure-explanation call itself (see
    // `failed_exit_finalizes_explanation_and_failed_exit_refs_partial_first`).
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "This explanation should not be requested.".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
            "result:cancel-before-explanation",
        )]]),
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
        .execute_family(&family_with_iteration_limit(0), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(host.model_call_count(), 1);
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test]
async fn cancellation_during_explanation_model_call_propagates_cancelled() {
    // Vehicle: iteration-limit abort at limit 0, so the scripted `Cancelled`
    // error hits the failure-explanation model call rather than the bounded
    // pre-termination warning turn (see
    // `failed_exit_finalizes_explanation_and_failed_exit_refs_partial_first`).
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Cancelled,
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::completed(
            "result:cancel-during-explanation",
        )]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = DriverMockHost::builder().script(script).build();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = executor
        .execute_family(&family_with_iteration_limit(0), &host, state)
        .await;

    assert!(
        matches!(result, Err(AgentLoopExecutorError::Cancelled)),
        "in-flight cancellation during the explanation call must propagate as Cancelled, not produce a Failed exit: {result:?}"
    );
    assert!(host.finalized_assistant_messages().is_empty());
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
async fn completed_output_digest_is_not_promoted_to_loop_progress_policy() {
    // The digest remains part of the host result contract, but the loop does not
    // retain it as heuristic no-progress evidence. Repetition is advisory-only
    // and keyed by consecutive call signatures.
    let digest = ironclaw_loop_contracts::ContentDigest(4242);
    let result_ref = LoopResultRef::new("result:digest-recorded").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                result_ref.clone(),
                "completed with digest".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        recorded.is_empty(),
        "digest policy ring must stay inert; got {recorded:?}"
    );
}

/// Byte-threshold trips through the full executor turn: capability batch returns
/// a result whose `byte_len` exceeds `ByteCapStrategy::DEFAULT_FALLBACK_CAP_BYTES`
/// (32 000). PostCapabilityStage should set both compaction flags on the state
/// that is written to the Final checkpoint.
#[tokio::test]
async fn executor_post_capability_trips_policy_and_sets_flags_in_final_state() {
    // Use terminate_hint so the loop exits immediately after the capability
    // turn, giving us a deterministic Final checkpoint to inspect.
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big").expect("valid"),
                "big result".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        Some(ironclaw_loop_contracts::CompactionInitiator::CapabilityResultOverflow),
        "force_compact_initiator must be CapabilityResultOverflow after a byte-cap trip"
    );
}

/// Under-threshold: small byte_len leaves both flags false in the final state.
#[tokio::test]
async fn executor_post_capability_does_not_trip_under_threshold() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:small").expect("valid"),
                "small result".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big-no-term").expect("valid"),
                "big result no terminate".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    true,
                    20_000,
                    None,
                    None,
                ),
                resolution::completed(
                    LoopResultRef::new("result:second").expect("valid"),
                    "second".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        Some(ironclaw_loop_contracts::CompactionInitiator::CapabilityResultOverflow),
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
        ironclaw_host_api::resolution::ResolutionBatch {
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
        ironclaw_host_api::resolution::ResolutionBatch {
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
async fn aborting_stale_surface_batch_error_writes_final_checkpoint() {
    let host = MockHost::new(vec![calls_response(), reply_response()])
        .fail_batch_with(AgentLoopHostErrorKind::StaleSurface);
    let planner = DefaultPlanner::compose_default()
        .with_recovery(Arc::new(support::ShrinkContextCallScopeRecoveryStrategy));
    let family = LoopFamily::new(
        LoopFamilyId::new("stale-surface-abort-checkpoint-test").expect("valid test family id"),
        ComponentIdentity::from_static(
            "stale-surface-abort-checkpoint-test",
            ComponentDigest([29; 32]),
        ),
        Arc::new(planner),
    );

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &family,
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("execute");

    let LoopExit::Failed(failed) = exit else {
        panic!("aborting recovery must fail the run");
    };
    assert!(
        failed.checkpoint_id.is_some(),
        "a direct batch-error terminal path must carry its Final checkpoint"
    );
    assert_eq!(
        host.checkpoint_kinds()
            .into_iter()
            .filter(|kind| *kind == LoopCheckpointKind::Final)
            .count(),
        1,
        "the direct batch-error terminal path must stage exactly one Final checkpoint"
    );
}

/// Regression test for epic #6284 item 1: a caller-shaped capability port
/// error (here `Unauthorized`, but any kind whose
/// `capability_port_error_is_terminal` is false) returned from the batch
/// dispatch must NOT end the run as `HostUnavailable { Capability }`.
///
/// Pre-fix (RED): every non-Cancelled port `Err` funneled through
/// `capability_host_error` and killed the run. Post-fix (GREEN): the executor
/// routes the error by `FailureKind::fate` — the model receives a tool-error
/// observation (kind `authorization`) via `handle_capability_error`, the loop
/// continues, and the scripted final reply completes the run.
#[tokio::test]
async fn recoverable_batch_port_error_surfaces_as_model_visible_tool_error() {
    let host = MockHost::new(vec![provider_calls_response(), reply_response()])
        .fail_batch_with(AgentLoopHostErrorKind::Unauthorized);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect(
            "REGRESSION: an Unauthorized capability port error must not kill the run as \
             HostUnavailable — it must surface to the model as a tool error",
        );
    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "run must complete after the model observes the authorization tool error; got {exit:?}"
    );

    // The model saw the failure as a tool-error observation with the honest
    // unified kind (Unauthorized -> Authorization), not a run-ending fault.
    let appended = host.appended_result_refs();
    let observation = appended
        .iter()
        .find_map(|request| request.model_observation.as_ref())
        .expect("a tool-error observation must be appended for the failed call");
    assert_eq!(observation.status, ToolObservationStatus::Error);
    match &observation.detail {
        ToolObservationDetail::GenericFailure {
            failure_kind,
            detail,
        } => {
            assert_eq!(*failure_kind, FailureKind::Authorization);
            assert_eq!(
                detail.as_deref(),
                Some("scripted batch failure"),
                "the caller-shaped host cause must survive the persistence/model observation seam"
            );
        }
        other => panic!("expected GenericFailure observation detail, got {other:?}"),
    }
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
