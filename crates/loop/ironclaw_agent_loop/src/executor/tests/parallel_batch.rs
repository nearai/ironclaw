use super::*;

#[tokio::test(start_paused = true)]
async fn model_emitted_batch_overlaps_calls_by_default_and_preserves_input_order() {
    let first_ref = LoopResultRef::new("result:parallel-first").expect("valid");
    let second_ref = LoopResultRef::new("result:parallel-second").expect("valid");
    let host = MockHost::new(vec![two_calls_response(), reply_response()])
        .with_single_outcomes(vec![
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
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let run_host = host.clone();
    let executor = tokio::spawn(async move {
        CanonicalAgentLoopExecutor
            .execute_family(&crate::families::default(), &run_host, state)
            .await
    });

    // Wait only for the first launch. If dispatch regresses to sequential,
    // waiting for both calls would deadlock under paused time instead of
    // reaching the concurrency assertion below.
    while host.single_invocations().is_empty() && !executor.is_finished() {
        tokio::task::yield_now().await;
    }
    // Let the initial poll register every concurrently launched sibling
    // before advancing either timer.
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(5)).await;
    tokio::time::advance(std::time::Duration::from_millis(75)).await;

    let exit = executor
        .await
        .expect("executor task must not panic")
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.max_concurrent_single_invocations(), 2);
    assert!(host.batch_invocations().is_empty());
    assert_eq!(host.single_invocations().len(), 2);
    assert_eq!(
        host.appended_result_refs()
            .into_iter()
            .map(|request| request.result_ref)
            .collect::<Vec<_>>(),
        vec![first_ref, second_ref]
    );
}

#[tokio::test]
async fn parallel_batch_uses_batch_port_when_ordered_middleware_requires_it() {
    let first_ref = LoopResultRef::new("result:ordered-first").expect("valid");
    let second_ref = LoopResultRef::new("result:ordered-second").expect("valid");
    let host = MockHost::new(vec![two_calls_response(), reply_response()])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
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
            ],
            stopped_on_suspension: false,
        }])
        .requiring_ordered_batch_invocation();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.batch_invocations().len(), 1);
    assert!(host.single_invocations().is_empty());
    assert_eq!(
        host.appended_result_refs()
            .into_iter()
            .map(|request| request.result_ref)
            .collect::<Vec<_>>(),
        vec![first_ref, second_ref]
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_stops_launching_new_calls_after_a_park() {
    let mut outcomes = vec![
        resolution::approval_required(
            LoopGateRef::new("gate:parallel-window-park").expect("valid"),
            "approval required".to_string(),
            None,
        )
        .resolution,
    ];
    outcomes.extend((1..6).map(|index| {
        resolution::completed(
            LoopResultRef::new(format!("result:parallel-window-{index}")).expect("valid"),
            format!("completed {index}"),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )
    }));

    let host = MockHost::new(vec![calls_response_with_count(6)])
        .with_single_outcomes(outcomes)
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    // Drive the executor on a separate task so the test controls the paused
    // clock: the scripted sleeps only complete when time is explicitly
    // advanced, so the park-before-replacement ordering no longer depends on
    // wall-clock sleep margins (a loaded runner can no longer let a 75 ms
    // sibling finish before the 5 ms parking call).
    let run_host = host.clone();
    let executor = tokio::spawn(async move {
        CanonicalAgentLoopExecutor
            .execute_family(
                &support::family_with_parallel_batch_execution(),
                &run_host,
                state,
            )
            .await
    });

    // Wait for the initial four-call window to be launched (every invocation
    // is parked in its paused sleep) before advancing the clock.
    while host.single_invocations().len() < 4 && !executor.is_finished() {
        tokio::task::yield_now().await;
    }
    // Call 0 (5 ms) completes first by construction, parks the batch, and
    // closes the launch window; nothing can launch a replacement before the
    // park is processed.
    tokio::time::advance(std::time::Duration::from_millis(5)).await;
    // In-flight siblings (75 ms) finish; the calls outside the window stay
    // unlaunched.
    tokio::time::advance(std::time::Duration::from_millis(75)).await;

    let exit = executor
        .await
        .expect("executor task must not panic")
        .expect("execute");

    assert!(matches!(exit, LoopExit::Blocked(_)));
    assert_eq!(host.max_concurrent_single_invocations(), 4);
    assert_eq!(
        host.single_invocations().len(),
        4,
        "the two calls outside the initial window must remain unlaunched"
    );
    let persisted_refs = host
        .appended_result_refs()
        .into_iter()
        .map(|request| request.result_ref)
        .collect::<Vec<_>>();
    assert_eq!(
        persisted_refs,
        (1..4)
            .map(|index| {
                LoopResultRef::new(format!("result:parallel-window-{index}")).expect("valid")
            })
            .collect::<Vec<_>>(),
        "completed siblings already in flight must be durable before the gate exit"
    );
    assert_eq!(
        final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock).result_refs,
        persisted_refs,
        "the suspension checkpoint must retain all completed siblings"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_stops_launching_new_calls_after_cancelled_resolution() {
    let mut outcomes = vec![resolution::failed(
        FailureKind::Cancelled,
        "cancelled by capability".to_string(),
        diagnostic_failure_detail("cancelled by capability"),
    )];
    outcomes.extend((1..6).map(|index| {
        resolution::completed(
            LoopResultRef::new(format!("result:parallel-cancel-window-{index}")).expect("valid"),
            format!("completed {index}"),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )
    }));

    let host = MockHost::new(vec![calls_response_with_count(6)])
        .with_single_outcomes(outcomes)
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let run_host = host.clone();
    let executor = tokio::spawn(async move {
        CanonicalAgentLoopExecutor
            .execute_family(
                &support::family_with_parallel_batch_execution(),
                &run_host,
                state,
            )
            .await
    });

    while host.single_invocations().len() < 4 && !executor.is_finished() {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(std::time::Duration::from_millis(5)).await;
    tokio::time::advance(std::time::Duration::from_millis(75)).await;

    let exit = executor
        .await
        .expect("executor task must not panic")
        .expect("execute");
    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(
        host.single_invocations().len(),
        4,
        "a typed cancelled result must close the launch window before replacement calls start"
    );
    assert_eq!(
        host.appended_result_refs()
            .into_iter()
            .map(|request| request.result_ref)
            .collect::<Vec<_>>(),
        (1..4)
            .map(|index| {
                LoopResultRef::new(format!("result:parallel-cancel-window-{index}")).expect("valid")
            })
            .collect::<Vec<_>>(),
        "the cancelled prefix must drain every launched sibling into durable state"
    );
    assert_eq!(
        host.checkpoint_kinds()
            .into_iter()
            .filter(|kind| *kind == LoopCheckpointKind::Final)
            .count(),
        1,
        "the drained cancellation must stage one authoritative Final checkpoint"
    );
}

#[tokio::test]
async fn parallel_batch_persists_sibling_gates_and_exits_on_first_input_order_gate() {
    // Two Approval gates, one completed call, and one retry-fated failure share
    // a bounded-parallel window. The first gate controls the exit, while every
    // sibling remains durable and the failure is surfaced without dispatching
    // a replacement call. The later approval becomes a model-visible pending
    // result because only one gate can own the BeforeBlock checkpoint.
    let first_approval_request_id = ApprovalRequestId::new();
    let first_gate_ref =
        LoopGateRef::new(format!("gate:approval-{first_approval_request_id}")).expect("valid");
    let second_gate_ref = LoopGateRef::new("gate:parallel-sibling-approval-2").expect("valid");
    let first_approval_resume = CapabilityApprovalResume {
        approval_request_id: first_approval_request_id,
        resume_token: CapabilityResumeToken::new("resume-token:parallel-sibling-approval")
            .expect("valid token"),
        correlation_id: CorrelationId::new(),
        input_ref: CapabilityInputRef::new("input:sibling-approval-1").expect("valid"),
    };
    let completed_ref = LoopResultRef::new("result:parallel-sibling-completed").expect("valid");
    let failed_ref = LoopResultRef::new("result:provider-error-turn_4-call_4").expect("valid");
    // The later approval call carries provider replay metadata so its merged
    // "approval gate pending" safe-summary ref materializes through the
    // existing safe-summary persistence (which no-ops without replay).
    let pending_ref = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_2"),
        sanitize_result_ref_suffix("call_2"),
    ))
    .expect("valid");
    let host = MockHost::new(vec![ironclaw_loop_contracts::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:sibling-completed").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:sibling-approval-1").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:sibling-approval-2").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_2".to_string(),
                    provider_call_id: "call_2".to_string(),
                    provider_tool_name: ProviderToolName::new("demo__echo")
                        .expect("provider tool name"),
                    arguments: serde_json::json!({"message": "second approval"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                }),
            },
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:sibling-failed").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_4".to_string(),
                    provider_call_id: "call_4".to_string(),
                    provider_tool_name: ProviderToolName::new("demo__echo")
                        .expect("provider tool name"),
                    arguments: serde_json::json!({"message": "failed sibling"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                }),
            },
        ]),
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    }])
    .with_single_outcomes(vec![
        resolution::completed(
            completed_ref.clone(),
            "parallel sibling completed".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        ),
        resolution::approval_required(
            first_gate_ref.clone(),
            "first approval required".to_string(),
            Some(first_approval_resume),
        )
        .resolution,
        resolution::approval_required(
            second_gate_ref.clone(),
            "second approval required".to_string(),
            None,
        )
        .resolution,
        resolution::failed(
            FailureKind::Network,
            "retry-fated sibling failed".to_string(),
            diagnostic_failure_detail("retry-fated sibling failed"),
        ),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect("execute");

    let LoopExit::Blocked(blocked) = exit else {
        panic!("expected Blocked exit, got {exit:?}");
    };
    assert_eq!(
        blocked.gate_ref, first_gate_ref,
        "the first input-order gate must control the exit"
    );

    // Every launched call is dispatched exactly once. In particular, the
    // retry-fated failure is made model-visible without a replacement dispatch
    // while the first gate is deferred.
    assert_eq!(
        host.single_invocations().len(),
        4,
        "gate drain must not dispatch a replacement for the failed sibling"
    );
    let appended = host.appended_result_refs();
    assert_eq!(
        appended
            .iter()
            .map(|request| request.result_ref.clone())
            .collect::<Vec<_>>(),
        vec![
            completed_ref.clone(),
            pending_ref.clone(),
            failed_ref.clone()
        ]
    );
    assert_eq!(
        appended[1].safe_summary, "approval gate pending",
        "the later approval must be durably model-visible as a pending gate"
    );
    assert_eq!(
        appended[2]
            .model_observation
            .as_ref()
            .expect("failed sibling must remain model-visible")
            .status,
        ToolObservationStatus::Error
    );

    // ONE coherent BeforeBlock checkpoint: the exit's own — a resumer reading
    // the (single) staged BeforeBlock state sees exactly the gate the exit
    // points at.
    assert_eq!(
        host.checkpoint_kinds()
            .into_iter()
            .filter(|kind| *kind == LoopCheckpointKind::BeforeBlock)
            .count(),
        1,
        "the batch must stage exactly one BeforeBlock checkpoint"
    );
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);

    // Every launched call's signature is durable in the exit's checkpoint, in
    // input order (the second approval's signature derives from its replay
    // arguments, mirroring capability_call_signature).
    let expected_signatures = vec![
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "input_ref": "input:sibling-completed" }),
        )
        .expect("signature"),
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "input_ref": "input:sibling-approval-1" }),
        )
        .expect("signature"),
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "message": "second approval" }),
        )
        .expect("signature"),
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "message": "failed sibling" }),
        )
        .expect("signature"),
    ];
    assert_eq!(
        before_block_state
            .recent_call_signatures
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        expected_signatures,
        "the exit checkpoint must represent every launched call's signature"
    );
    assert_eq!(
        before_block_state.result_refs,
        vec![completed_ref, pending_ref, failed_ref],
        "the gate checkpoint must retain completed, pending-gate, and failed sibling results"
    );
    assert_eq!(
        before_block_state.last_gate.as_ref(),
        Some(&first_gate_ref),
        "the checkpoint must resume the first input-order gate"
    );
    assert_eq!(
        before_block_state
            .pending_approval_resume
            .as_ref()
            .map(|resume| resume.gate_ref.clone()),
        Some(first_gate_ref.clone()),
        "the single approval resume slot must belong to the first input-order gate"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_reports_first_input_order_terminal_error() {
    let host = MockHost::new(vec![two_calls_response()])
        .with_single_results(vec![
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "first input call failed",
            )
            .with_detail("first input detail")),
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "second input call failed",
            )
            .with_detail("second input detail")),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect_err("terminal capability failures must end the run");

    assert!(
        matches!(
            &error,
            AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                safe_summary,
                ..
            } if safe_summary.as_str() == "first input call failed"
        ),
        "terminal error selection must follow input order, not completion order: {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_persists_completed_sibling_before_terminal_port_error() {
    let completed_ref =
        LoopResultRef::new("result:parallel-terminal-sibling-completed").expect("valid");
    let host = MockHost::new(vec![provider_two_calls_response()])
        .with_single_results(vec![
            Ok(resolution::completed(
                completed_ref.clone(),
                "completed before sibling host error".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )),
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "second input call failed terminally",
            )),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(75),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect_err("terminal capability failure must end the run");

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics { .. }
            | AgentLoopExecutorError::HostUnavailable { .. }
    ));
    assert_eq!(
        host.appended_result_refs()
            .into_iter()
            .map(|request| request.result_ref)
            .collect::<Vec<_>>(),
        vec![completed_ref.clone()],
        "a launched successful sibling must be durable before the terminal error returns"
    );
    assert_eq!(
        final_staged_state(&host).result_refs,
        vec![completed_ref],
        "the terminal path checkpoint must retain the launched sibling result"
    );
}

#[tokio::test(start_paused = true)]
async fn terminal_error_truncates_launch_window_and_drains_launched_siblings() {
    let completed_refs = (1..4)
        .map(|index| {
            LoopResultRef::new(format!("result:parallel-terminal-window-{index}")).expect("valid")
        })
        .collect::<Vec<_>>();
    let mut results = vec![Err(AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "first launched call failed terminally",
    )
    .with_detail("terminal launch-window regression"))];
    results.extend(
        completed_refs
            .iter()
            .enumerate()
            .map(|(index, result_ref)| {
                Ok(resolution::completed(
                    result_ref.clone(),
                    format!("completed sibling {}", index + 1),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ))
            }),
    );
    let host = MockHost::new(vec![calls_response_with_count(6)])
        .with_single_results(results)
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(75),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect_err("terminal capability failure must end the run");

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics { .. }
            | AgentLoopExecutorError::HostUnavailable { .. }
    ));
    assert_eq!(
        host.single_invocations().len(),
        4,
        "a terminal host error must close the launch window before replacements start"
    );
    assert_eq!(
        host.appended_result_refs()
            .into_iter()
            .map(|request| request.result_ref)
            .collect::<Vec<_>>(),
        completed_refs,
        "every sibling launched before the terminal error must be drained"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_selects_earlier_cancelled_resolution_over_terminal_port_error() {
    let host = MockHost::new(vec![provider_two_calls_response()])
        .with_single_results(vec![
            Ok(resolution::failed(
                FailureKind::Cancelled,
                "cancelled by capability".to_string(),
                diagnostic_failure_detail("cancelled by capability"),
            )),
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "later input call failed terminally",
            )),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect("the earlier typed cancellation must control the run exit");

    assert!(
        matches!(exit, LoopExit::Cancelled(_)),
        "terminal selection must use input order rather than completion order or error class"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_selects_earlier_terminal_port_error_over_cancelled_resolution() {
    let host = MockHost::new(vec![provider_two_calls_response()])
        .with_single_results(vec![
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "earlier input call failed terminally",
            )),
            Ok(resolution::failed(
                FailureKind::Cancelled,
                "cancelled by capability".to_string(),
                diagnostic_failure_detail("cancelled by capability"),
            )),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(75),
            std::time::Duration::from_millis(5),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect_err("the earlier terminal host error must control the run exit");

    assert!(
        matches!(
            error,
            AgentLoopExecutorError::HostUnavailableWithDiagnostics { .. }
                | AgentLoopExecutorError::HostUnavailable { .. }
        ),
        "terminal selection must use input order rather than completion order or error class"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_persists_recoverable_sibling_before_terminal_port_error() {
    let expected_error_ref =
        LoopResultRef::new("result:provider-error-turn_1-call_1").expect("valid");
    let host = MockHost::new(vec![provider_two_calls_response()])
        .with_single_results(vec![
            Ok(resolution::failed(
                FailureKind::Network,
                "first invocation hit a network failure".to_string(),
                diagnostic_failure_detail("first invocation hit a network failure"),
            )),
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "second input call failed terminally",
            )),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(75),
        ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect_err("the terminal host failure must end the run");

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics { .. }
            | AgentLoopExecutorError::HostUnavailable { .. }
    ));
    assert_eq!(
        host.single_invocations().len(),
        2,
        "terminal drain must persist retry-fated siblings without dispatching replacement calls"
    );
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].result_ref, expected_error_ref);
    assert_eq!(
        appended[0]
            .model_observation
            .as_ref()
            .expect("recoverable failure must remain model-visible")
            .status,
        ToolObservationStatus::Error
    );
    assert_eq!(
        final_staged_state(&host).result_refs,
        vec![expected_error_ref],
        "the terminal checkpoint must retain recoverable sibling bookkeeping"
    );
}

#[tokio::test]
async fn parallel_batch_preserves_success_when_sibling_returns_recoverable_port_error() {
    let completed_ref = LoopResultRef::new("result:parallel-mixed-success").expect("valid"); // safety: test-only fixture
    let host = MockHost::new(vec![provider_two_calls_response(), reply_response()])
        .with_single_results(vec![
            Ok(resolution::completed(
                completed_ref.clone(),
                "first completed".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )),
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "second invocation was invalid",
            )
            .with_detail("second invocation rejected")),
        ])
        .with_single_invoke_delays(vec![
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(5),
        ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect("recoverable sibling error must not discard completed outcomes");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 2);
    assert_eq!(
        appended[0].result_ref, completed_ref,
        "the successful first call must be persisted in input order"
    );
    let observation = appended[1]
        .model_observation
        .as_ref()
        .expect("the failed second call must carry a model-visible error");
    assert_eq!(observation.status, ToolObservationStatus::Error);
    assert!(
        matches!(
            &observation.detail,
            ToolObservationDetail::GenericFailure {
                failure_kind: FailureKind::InputEncode,
                detail: Some(detail),
            } if detail == "second invocation rejected"
        ),
        "the recoverable error must be attributed only to its matching call"
    );
}

#[tokio::test(start_paused = true)]
async fn parallel_batch_cancelled_sibling_ends_run_with_checked_state() {
    // Gap-2 regression (review follow-up): a cancellation passthrough exit
    // (`OutcomeStep::Exit { state: None }`) must end the run immediately with
    // the cancelled sibling's own coherent checkpoint — not let the deferred
    // gate produce a later cancellation checkpoint from the older shared
    // state that omits the sibling's pre-cancel mutations.
    let gate_ref = LoopGateRef::new("gate:parallel-cancelled-sibling").expect("valid");
    let error_ref = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_5"),
        sanitize_result_ref_suffix("call_5"),
    ))
    .expect("valid");
    let host = MockHost::new(vec![ironclaw_loop_contracts::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:gate").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: None,
            },
            CapabilityCallCandidate {
                activity_id: CapabilityActivityId::new(),
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:cancelled").expect("valid"),
                effective_capability_ids: vec![capability_id()],
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_5".to_string(),
                    provider_call_id: "call_5".to_string(),
                    provider_tool_name: ProviderToolName::new("demo__echo")
                        .expect("provider tool name"),
                    arguments: serde_json::json!({"message": "cancelled sibling"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: Some("sig-cancel".to_string()),
                }),
            },
        ]),
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    }])
    .with_single_outcomes(vec![
        resolution::approval_required(gate_ref.clone(), "approval required".to_string(), None)
            .resolution,
        resolution::failed(
            FailureKind::OperationFailed,
            "cancelled sibling".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "cancelled detail".to_string(),
            },
        ),
    ])
    .with_single_invoke_delays(vec![
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(75),
    ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    // Drive the executor on a separate task so the test controls the paused
    // clock: the cancellation signal is raised while both invocations are
    // parked, before the drain processes the failing sibling.
    let run_host = host.clone();
    let executor = tokio::spawn(async move {
        CanonicalAgentLoopExecutor
            .execute_family(
                &support::family_with_parallel_batch_execution(),
                &run_host,
                state,
            )
            .await
    });
    while host.single_invocations().len() < 2 && !executor.is_finished() {
        tokio::task::yield_now().await;
    }
    host.request_cancellation(LoopCancelReasonKind::UserRequested);
    tokio::time::advance(std::time::Duration::from_millis(5)).await;
    tokio::time::advance(std::time::Duration::from_millis(75)).await;

    let exit = executor
        .await
        .expect("executor task must not panic")
        .expect("execute");

    let LoopExit::Cancelled(cancelled) = exit else {
        panic!("expected Cancelled exit, got {exit:?}");
    };
    assert_eq!(
        cancelled.reason_kind,
        LoopCancelledReasonKind::HostCancellation
    );
    assert!(
        cancelled.checkpoint_id.is_some(),
        "the cancelled sibling's checked state must be checkpointed"
    );
    assert!(
        !host
            .checkpoint_kinds()
            .contains(&LoopCheckpointKind::BeforeBlock),
        "the deferred gate must not stage a BeforeBlock after a cancellation exit"
    );

    // The exit's Final checkpoint is the cancelled sibling's own: it carries
    // the pre-cancel mutations (error ref + failure kind) that the older
    // shared state — which the deferred gate would have checkpointed — omits.
    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.result_refs,
        vec![error_ref],
        "the cancelled sibling's checked state must retain its appended error ref"
    );
    assert_eq!(
        final_state
            .recent_failure_kinds
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![capability_error_to_failure_kind(
            FailureKind::OperationFailed
        )],
        "the cancelled sibling's checked state must retain its failure bookkeeping"
    );
}

#[tokio::test]
async fn ordered_middleware_preserves_the_complete_model_batch_contract() {
    let host = MockHost::new(vec![calls_response_with_count(2)]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new("gate:exclusive-early-break").expect("valid"),
                    "approval required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            state,
        )
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Blocked(_)));
    assert!(host.single_invocations().is_empty());
    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 1);
    assert!(batch_invocations[0].stop_on_first_suspension);
    assert_eq!(
        final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock)
            .budget_ledger
            .capability_invocations_made(),
        1,
        "only the first call reached the ordered host before suspension"
    );
}

#[tokio::test]
async fn parallel_batch_records_completed_results_before_blocking_on_suspension() {
    let completed_ref = LoopResultRef::new("result:parallel-completed").expect("valid"); // safety: test-only fixture
    let host = MockHost::new(vec![two_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
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
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
        ironclaw_host_api::resolution::ResolutionBatch {
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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::completed(
                    LoopResultRef::new("result:second").expect("valid"),
                    "second".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
async fn parallel_batch_records_completed_results_before_external_tool_block() {
    let completed_ref = LoopResultRef::new("result:parallel-external-completed").expect("valid");
    let external_gate_ref = LoopGateRef::new("gate:external-tool-parallel").expect("valid");
    let host = MockHost::new(vec![two_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    completed_ref.clone(),
                    "parallel call completed".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
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
