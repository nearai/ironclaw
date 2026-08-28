use super::*;

#[tokio::test]
async fn prompt_stage_circuit_breaker_disables_compaction_after_repeated_ineffective_runs() {
    use ironclaw_loop_contracts::PromptContextTokenBudget;

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
            redacted_leak_count: 0,
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
            .process(ctx, PromptInput { state })
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
        .process(ctx, PromptInput { state })
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
    use ironclaw_loop_contracts::PromptContextTokenBudget;

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
            redacted_leak_count: 0,
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
        .process(ctx, PromptInput { state })
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
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 99,
        prompt_fingerprint: 123,
    });

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
    assert_eq!(
        output.state.compaction_state.last_compacted_through_seq,
        Some(1)
    );
    assert_eq!(output.state.compaction_state.last_deferred, None);
    assert!(
        !host
            .progress_event_names()
            .contains(&"compaction_leak_detected"),
        "zero redactions must not emit leak telemetry"
    );
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
        .process(ctx, PromptInput { state })
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
        .process(ctx, PromptInput { state })
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
        PromptStep::SkipModel(_) => panic!("unexpected SkipModel"),
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
        .process(ctx, PromptInput { state })
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
        | PromptStep::SkipModel(_) => panic!("unexpected prompt step"),
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
        .process(ctx, PromptInput { state })
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

// `start_paused` makes the ordering deterministic: both the cancellation delay
// below and `with_compaction_delay` sleep on Tokio's clock, so the runtime
// auto-advances to the 5 ms timer before the 50 ms compaction timer instead of
// racing them against wall-clock scheduling on a loaded runner.
#[tokio::test(start_paused = true)]
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
            redacted_leak_count: 0,
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
        .process(ctx, PromptInput { state })
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
            redacted_leak_count: 0,
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
        PromptStage.process(ctx, PromptInput { state }),
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
            redacted_leak_count: 1,
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
        .process(ctx, PromptInput { state })
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
            "compaction_leak_detected",
            "checkpoint_written",
        ]
    );
}

#[tokio::test]
async fn model_context_overflow_retries_through_canonical_compaction_stage() {
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContextOverflow,
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
            redacted_leak_count: 0,
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
    assert_eq!(
        host.progress_events()
            .into_iter()
            .filter(|event| matches!(
                event,
                LoopProgressEvent::FailureRecovered {
                    sequence: 1,
                    stage: LoopRecoveryStage::Model,
                    class: LoopRecoveryClass::ModelContextOverflow,
                    disposition: LoopRecoveryDisposition::Retried,
                }
            ))
            .count(),
        1,
        "one failed model attempt must produce exactly one recovery numerator event"
    );

    let recovery_checkpoint = host
        .staged_payloads()
        .into_iter()
        .filter(|request| request.kind == LoopCheckpointKind::BeforeModel)
        .filter_map(|request| {
            LoopExecutionState::from_checkpoint_payload(
                &request.payload,
                CheckpointKind::BeforeModel,
            )
            .ok()
        })
        .find(|state| {
            state
                .recovery_state
                .attempts_for(crate::state::RecoveryAttemptClass::ModelContextOverflow)
                == 1
                && state.compaction_state.force_compact_on_next_iteration
        })
        .expect("recovery checkpoint persists the consumed attempt and compaction request");
    assert!(
        recovery_checkpoint
            .pending_model_error_observation
            .is_none()
    );

    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.compaction_state.last_compacted_through_seq,
        Some(5)
    );
    assert!(!final_state.compaction_state.force_compact_on_next_iteration);
}

#[tokio::test]
async fn second_model_context_overflow_aborts_without_another_compaction() {
    let overflow = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::ContextOverflow,
            "model request exceeded its context budget",
        )
    };
    let host = MockHost::new(Vec::new())
        .with_model_errors(vec![overflow(), overflow()])
        .with_prompt_compaction_indexes(vec![
            vec![compaction_metadata(1, LoopContextCompactionKind::User, 10)],
            active_task_preserving_compaction_index(),
            Vec::new(),
        ])
        .with_compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary:overflow-once")
                .expect("valid summary id"),
            compression_ratio_ppm: 100_000,
            redacted_leak_count: 0,
        }));
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Failed(_)));
    assert_eq!(host.model_requests().len(), 2);
    assert_eq!(
        host.progress_events()
            .into_iter()
            .filter(|event| matches!(event, LoopProgressEvent::CompactionStarted { .. }))
            .count(),
        1,
        "a second overflow must not start another compaction"
    );
    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state
            .recovery_state
            .attempts_for(crate::state::RecoveryAttemptClass::ModelContextOverflow),
        2
    );
    assert!(final_state.pending_model_error_observation.is_none());
}

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
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                LoopResultRef::new("result:big-f12").expect("valid"),
                "big result for F12".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
            redacted_leak_count: 0,
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
                ironclaw_loop_contracts::LoopProgressEvent::CompactionStarted { .. }
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
        ironclaw_loop_contracts::LoopProgressEvent::CompactionStarted { initiator, .. } => {
            assert_eq!(
                initiator,
                &ironclaw_loop_contracts::CompactionInitiator::CapabilityResultOverflow,
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
    .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
        resolutions: vec![resolution::completed(
            LoopResultRef::new("result:big-compaction-rejected").expect("valid"),
            "large search result".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
