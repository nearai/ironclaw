use super::*;

#[tokio::test]
async fn model_budget_approval_required_with_gate_ref_blocks_resource_gate() {
    let gate_ref = LoopGateRef::new("gate:budget-test-approval").expect("gate ref");
    let host = MockHost::new(vec![reply_response()]).with_model_errors(vec![
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetApprovalRequired,
            "budget approval required",
        )
        .with_gate_ref(gate_ref.clone()),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Blocked(blocked) => {
            assert_eq!(
                blocked.kind,
                ironclaw_loop_contracts::LoopBlockedKind::Resource
            );
            assert_eq!(blocked.gate_ref, gate_ref);
            assert_eq!(blocked.blocked_activity_id, None);
        }
        other => panic!("expected budget approval to block, got {other:?}"),
    }
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeBlock
        ]
    );
    assert!(host.progress_event_names().contains(&"gate_blocked"));
    let blocked_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert_eq!(blocked_state.last_gate, Some(gate_ref));
}

#[tokio::test]
async fn model_budget_approval_required_without_gate_ref_fails_diagnostics_not_recovery() {
    let host =
        MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetApprovalRequired,
            "budget approval required",
        )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let error = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("budget approval without gate evidence must fail closed");

    assert_eq!(
        error,
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::BudgetApprovalRequired,
            safe_summary: LoopSafeSummary::new("budget approval required").expect("safe"),
            reason_kind: None,
            detail: None,
        }
    );
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel]
    );
    assert!(!host.progress_event_names().contains(&"gate_blocked"));
}

#[tokio::test]
async fn gate_blocks_with_before_block_checkpoint() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new("gate:approval").expect("valid"),
                    "approval required".to_string(),
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

    assert!(matches!(exit, LoopExit::Blocked(_)));
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::BeforeBlock,
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
            "gate_blocked",
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
    assert_eq!(completed, (0, 0, 1, 0));
}

/// Focused regression for gates.rs:85 — `GateStage` must stamp
/// `disposition: None` on the initial (blocking) `PendingApprovalResume`
/// checkpoint.  A denial has not yet occurred at block time; writing any
/// non-`None` disposition here would short-circuit the capability stage
/// incorrectly on the very next resume, before any user deny action.
#[tokio::test]
async fn approval_gate_before_block_checkpoint_disposition_is_none() {
    let approval_resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("resume-token:disposition-none-test")
            .expect("valid token"),
        correlation_id: CorrelationId::new(),
        input_ref: CapabilityInputRef::new("input:disposition-none-test").expect("valid"),
    };
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!(
                        "gate:approval-{}",
                        approval_resume.approval_request_id
                    ))
                    .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
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

    assert!(matches!(exit, LoopExit::Blocked(_)));

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_resume = before_block_state
        .pending_approval_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_approval_resume");

    // Key regression assertion: disposition must be None at the first-block
    // checkpoint.  A non-None value here means GateStage pre-stamped a denial
    // that hasn't happened yet, which would cause the capability stage to
    // incorrectly short-circuit on the very next resume.
    assert_eq!(
        pending_resume.disposition, None,
        "pending_approval_resume.disposition must be None at the initial BeforeBlock checkpoint \
         (no denial has occurred yet — GateStage must not pre-stamp a disposition)"
    );
}

#[tokio::test]
async fn gate_stage_skips_and_continues_records_skipped_summary() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-skip").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Auth,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
                auth_resume: None,
            },
        )
        .await
        .expect("gate stage");

    let BatchStep::Continue(state) = step else {
        panic!("expected skip-and-continue");
    };
    assert_eq!(state.result_refs.len(), 1);
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].safe_summary, "auth gate skipped");
    assert!(host.checkpoint_kinds().is_empty());
}

#[tokio::test]
async fn gate_stage_aborts_returns_failed_exit() {
    let failure_kind = LoopFailureKind::CapabilityProtocolError;
    let family = family_with_gate_outcome(GateOutcome::Abort {
        gate: empty_gate_state(),
        failure_kind,
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-abort").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Auth,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
                auth_resume: None,
            },
        )
        .await
        .expect("gate stage");

    match step {
        BatchStep::Exit(LoopExit::Failed(failed)) => {
            assert_eq!(failed.reason_kind, failure_kind);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    let appended = host.appended_result_refs();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].safe_summary, "auth gate aborted");
}

#[tokio::test]
async fn parallel_batch_merges_exiting_sibling_state_into_gate_checkpoint() {
    // CodeRabbit #3748865793 regression: `BatchStep::Exit` carries no state, so
    // an exiting sibling's outcome processing (error ref, failure explanation
    // ref, recent failure bookkeeping) ran on a snapshot clone whose mutations
    // were dropped instead of merged into the shared drain state. The first
    // gate's BeforeBlock checkpoint — the durable resume state — must retain
    // every processed sibling's mutations, while the first input-order exit
    // still controls the exit selection.
    let gate_ref = LoopGateRef::new("gate:parallel-exiting-sibling").expect("valid");
    let completed_ref =
        LoopResultRef::new("result:parallel-exiting-sibling-success").expect("valid"); // safety: test-only fixture
    let digest = ironclaw_loop_contracts::ContentDigest(4242);
    let error_ref = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_7"),
        sanitize_result_ref_suffix("call_7"),
    ))
    .expect("valid");
    let host = MockHost::new(vec![
        ironclaw_loop_contracts::LoopModelResponse {
            chunks: Vec::new(),
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![
                // Input order: gate (owns the exit), exiting sibling, success.
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
                    input_ref: CapabilityInputRef::new("input:exiting").expect("valid"),
                    effective_capability_ids: vec![capability_id()],
                    // Provider replay materializes the abort's error ref in
                    // `state.result_refs` (the safe-summary persistence seam
                    // no-ops without replay).
                    provider_replay: Some(ProviderToolCallReplay {
                        provider_id: "test-provider".to_string(),
                        provider_model_id: "test-model".to_string(),
                        provider_turn_id: "turn_7".to_string(),
                        provider_call_id: "call_7".to_string(),
                        provider_tool_name: ProviderToolName::new("demo__echo")
                            .expect("provider tool name"),
                        arguments: serde_json::json!({"message": "exiting sibling"}),
                        response_reasoning: None,
                        reasoning: None,
                        signature: Some("sig-exit".to_string()),
                    }),
                },
                CapabilityCallCandidate {
                    activity_id: CapabilityActivityId::new(),
                    surface_version: surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:success").expect("valid"),
                    effective_capability_ids: vec![capability_id()],
                    provider_replay: None,
                },
            ]),
            effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
                .expect("valid"),
            usage: None,
        },
        // Second model response: the exiting sibling's failure explanation.
        reply_response(),
    ])
    .with_single_outcomes(vec![
        resolution::approval_required(gate_ref.clone(), "approval required".to_string(), None)
            .resolution,
        resolution::failed(
            FailureKind::OperationFailed,
            "exiting sibling failed".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "exiting sibling failure detail".to_string(),
            },
        ),
        resolution::completed(
            completed_ref.clone(),
            "exiting sibling success".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            Some(digest),
            None,
        ),
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());
    // Bounded-parallel dispatch (so gate + failing sibling share one window)
    // with a recovery strategy that aborts capability errors (so the sibling
    // outcome exits).
    let planner = DefaultPlanner::compose_default()
        .with_recovery(Arc::new(support::ShrinkContextCallScopeRecoveryStrategy));
    let family = LoopFamily::new(
        LoopFamilyId::new("executor-exit-sibling-merge-test").expect("valid test family id"),
        ComponentIdentity::from_static(
            "executor-exit-sibling-merge-test",
            ComponentDigest([17; 32]),
        ),
        Arc::new(planner),
    );

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    // The first gate in input order owns the exit; the later sibling's exit is
    // processed and merged but never selected.
    let LoopExit::Blocked(blocked) = exit else {
        panic!("expected Blocked exit, got {exit:?}");
    };
    assert_eq!(
        blocked.gate_ref, gate_ref,
        "the first input-order gate must control the exit"
    );

    // Host-side durability: every processed sibling's ref is appended.
    let appended = host.appended_result_refs();
    assert_eq!(
        appended
            .iter()
            .map(|request| request.result_ref.clone())
            .collect::<Vec<_>>(),
        vec![completed_ref.clone(), error_ref.clone()],
        "the successful sibling's result and the exiting sibling's error ref must both be durable"
    );

    // State-side durability: the first gate's BeforeBlock checkpoint — the
    // state a resumer reads — retains the exiting sibling's error ref,
    // explanation ref, and failure bookkeeping alongside the successful
    // sibling's result.
    let before_block = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert_eq!(
        before_block.result_refs,
        vec![completed_ref.clone(), error_ref.clone()],
        "the resume checkpoint must retain the exiting sibling's appended error ref"
    );
    assert_eq!(
        before_block.assistant_refs,
        vec![message_ref("msg:assistant")],
        "the resume checkpoint must retain the exiting sibling's failure explanation ref"
    );
    assert_eq!(
        before_block
            .recent_failure_kinds
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![
            capability_error_to_failure_kind(FailureKind::OperationFailed),
            LoopFailureKind::CapabilityProtocolError,
        ],
        "the resume checkpoint must retain the exiting sibling's failure bookkeeping"
    );
    assert_eq!(
        before_block.last_gate.as_ref(),
        Some(&gate_ref),
        "the checkpoint must resume the first input-order gate"
    );
}

#[tokio::test]
async fn parallel_batch_rebuilds_pre_gate_terminal_exit_against_merged_checkpoint() {
    // Gap-1 regression (review follow-up): when the first input-order terminal
    // sibling exit precedes the deferred gate, later siblings are still
    // processed and merged, and the selected exit is rebuilt against a fresh
    // Final checkpoint so its checkpoint carries every processed sibling's
    // mutations. The deferred gate is persisted durably as a pending outcome
    // instead of staging an orphaned BeforeBlock the run never resumes from.
    let gate_ref = LoopGateRef::new("gate:parallel-rebuilt-exit").expect("valid");
    let error_ref_a = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_7"),
        sanitize_result_ref_suffix("call_7"),
    ))
    .expect("valid");
    let error_ref_b = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_8"),
        sanitize_result_ref_suffix("call_8"),
    ))
    .expect("valid");
    let pending_ref = LoopResultRef::new(format!(
        "result:provider-error-{}-{}",
        sanitize_result_ref_suffix("turn_9"),
        sanitize_result_ref_suffix("call_9"),
    ))
    .expect("valid");
    let host = MockHost::new(vec![
        ironclaw_loop_contracts::LoopModelResponse {
            chunks: Vec::new(),
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![
                CapabilityCallCandidate {
                    activity_id: CapabilityActivityId::new(),
                    surface_version: surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:failing-a").expect("valid"),
                    effective_capability_ids: vec![capability_id()],
                    provider_replay: Some(ProviderToolCallReplay {
                        provider_id: "test-provider".to_string(),
                        provider_model_id: "test-model".to_string(),
                        provider_turn_id: "turn_7".to_string(),
                        provider_call_id: "call_7".to_string(),
                        provider_tool_name: ProviderToolName::new("demo__echo")
                            .expect("provider tool name"),
                        arguments: serde_json::json!({"message": "failing A"}),
                        response_reasoning: None,
                        reasoning: None,
                        signature: Some("sig-a".to_string()),
                    }),
                },
                CapabilityCallCandidate {
                    activity_id: CapabilityActivityId::new(),
                    surface_version: surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:failing-b").expect("valid"),
                    effective_capability_ids: vec![capability_id()],
                    provider_replay: Some(ProviderToolCallReplay {
                        provider_id: "test-provider".to_string(),
                        provider_model_id: "test-model".to_string(),
                        provider_turn_id: "turn_8".to_string(),
                        provider_call_id: "call_8".to_string(),
                        provider_tool_name: ProviderToolName::new("demo__echo")
                            .expect("provider tool name"),
                        arguments: serde_json::json!({"message": "failing B"}),
                        response_reasoning: None,
                        reasoning: None,
                        signature: Some("sig-b".to_string()),
                    }),
                },
                CapabilityCallCandidate {
                    activity_id: CapabilityActivityId::new(),
                    surface_version: surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:gate").expect("valid"),
                    effective_capability_ids: vec![capability_id()],
                    provider_replay: Some(ProviderToolCallReplay {
                        provider_id: "test-provider".to_string(),
                        provider_model_id: "test-model".to_string(),
                        provider_turn_id: "turn_9".to_string(),
                        provider_call_id: "call_9".to_string(),
                        provider_tool_name: ProviderToolName::new("demo__echo")
                            .expect("provider tool name"),
                        arguments: serde_json::json!({"message": "gate call"}),
                        response_reasoning: None,
                        reasoning: None,
                        signature: Some("sig-gate".to_string()),
                    }),
                },
            ]),
            effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
                .expect("valid"),
            usage: None,
        },
        // One failure-explanation model call per aborting sibling.
        reply_response(),
        reply_response(),
    ])
    .with_single_outcomes(vec![
        resolution::failed(
            FailureKind::OperationFailed,
            "failing A".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "A detail".to_string(),
            },
        ),
        resolution::failed(
            FailureKind::InputEncode,
            "failing B".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "B detail".to_string(),
            },
        ),
        resolution::approval_required(gate_ref.clone(), "approval required".to_string(), None)
            .resolution,
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());
    let planner = DefaultPlanner::compose_default()
        .with_recovery(Arc::new(support::ShrinkContextCallScopeRecoveryStrategy));
    let family = LoopFamily::new(
        LoopFamilyId::new("executor-rebuilt-exit-test").expect("valid test family id"),
        ComponentIdentity::from_static("executor-rebuilt-exit-test", ComponentDigest([19; 32])),
        Arc::new(planner),
    );

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    // The first input-order terminal exit (the aborting sibling) still wins,
    // now rebuilt against the fully merged checkpoint.
    let LoopExit::Failed(failed) = exit else {
        panic!("expected Failed exit, got {exit:?}");
    };
    assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
    assert!(
        failed.checkpoint_id.is_some(),
        "the rebuilt exit must reference the fresh Final checkpoint"
    );
    assert_eq!(
        failed.explanation_message_refs,
        vec![message_ref("msg:assistant")],
        "the rebuilt exit must re-derive and deduplicate explanation refs from the merged state"
    );

    // Both failing siblings were processed, and the deferred gate was
    // persisted as a pending outcome — no orphaned BeforeBlock.
    let appended = host.appended_result_refs();
    assert_eq!(
        appended
            .iter()
            .map(|request| request.result_ref.clone())
            .collect::<Vec<_>>(),
        vec![
            error_ref_a.clone(),
            error_ref_b.clone(),
            pending_ref.clone()
        ],
        "every processed sibling and the persisted pending gate must be durably appended"
    );
    assert_eq!(
        appended[2].safe_summary, "approval gate pending",
        "the deferred gate must be model-visibly pending, not resumable"
    );
    assert!(
        !host
            .checkpoint_kinds()
            .contains(&LoopCheckpointKind::BeforeBlock),
        "a terminal pre-gate exit must not stage the gate's BeforeBlock"
    );

    // The exit's Final checkpoint is the fresh one: it carries BOTH failing
    // siblings' bookkeeping and the persisted gate, unlike the exit's
    // originally staged checkpoint which predated sibling B.
    let final_state = final_staged_state(&host);
    assert_eq!(
        final_state.result_refs,
        vec![error_ref_a, error_ref_b, pending_ref],
        "the rebuilt exit's checkpoint must carry every processed sibling's refs"
    );
    assert_eq!(
        final_state
            .recent_failure_kinds
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![
            LoopFailureKind::CapabilityProtocolError,
            LoopFailureKind::CapabilityProtocolError,
            LoopFailureKind::ModelError,
            LoopFailureKind::CapabilityProtocolError,
        ],
        "the rebuilt exit's checkpoint must carry both siblings' failure bookkeeping"
    );
    assert_eq!(
        final_state.assistant_refs,
        vec![message_ref("msg:assistant"), message_ref("msg:assistant")],
        "the rebuilt exit's checkpoint must carry both siblings' explanation refs"
    );
    let expected_signatures = vec![
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "message": "failing A" }),
        )
        .expect("signature"),
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "message": "failing B" }),
        )
        .expect("signature"),
        CapabilityCallSignature::from_call(
            capability_id(),
            &serde_json::json!({ "message": "gate call" }),
        )
        .expect("signature"),
    ];
    assert_eq!(
        final_state
            .recent_call_signatures
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        expected_signatures,
        "every launched call's signature must survive in the rebuilt exit's checkpoint"
    );
}

#[tokio::test]
async fn coalesced_dependent_gate_precedes_later_terminal_sibling() {
    let gate_ref = LoopGateRef::new("gate:coalesced-before-terminal").expect("valid");
    let host = MockHost::new(vec![calls_response_with_count(3), reply_response()])
        .with_single_outcomes(vec![
            resolution::await_dependent_run(
                gate_ref.clone(),
                LoopResultRef::new("result:coalesced-terminal-a").expect("valid"),
                "first dependent result".to_string(),
                0,
                None,
            )
            .resolution,
            resolution::await_dependent_run(
                gate_ref.clone(),
                LoopResultRef::new("result:coalesced-terminal-b").expect("valid"),
                "second dependent result".to_string(),
                0,
                None,
            )
            .resolution,
            resolution::failed(
                FailureKind::OperationFailed,
                "later sibling failed".to_string(),
                diagnostic_failure_detail("later sibling failure"),
            ),
        ]);
    let planner = DefaultPlanner::compose_default()
        .with_recovery(Arc::new(support::ShrinkContextCallScopeRecoveryStrategy));
    let family = LoopFamily::new(
        LoopFamilyId::new("coalesced-gate-terminal-order-test").expect("valid test family id"),
        ComponentIdentity::from_static(
            "coalesced-gate-terminal-order-test",
            ComponentDigest([23; 32]),
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

    let LoopExit::Blocked(blocked) = exit else {
        panic!("earlier coalesced gate must control the exit");
    };
    assert_eq!(blocked.gate_ref, gate_ref);
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert_eq!(
        before_block_state.result_refs.len(),
        2,
        "the gate checkpoint must retain both coalesced dependent results"
    );
    assert!(
        before_block_state
            .recent_failure_kinds
            .iter()
            .any(|kind| *kind == LoopFailureKind::CapabilityProtocolError),
        "the gate checkpoint must retain the later terminal sibling's failure bookkeeping"
    );
}

#[tokio::test]
async fn coalesced_dependent_gate_suppresses_sibling_retry_dispatch() {
    let gate_ref = LoopGateRef::new("gate:coalesced-suppresses-retry").expect("valid");
    let host = MockHost::new(vec![calls_response_with_count(3)]).with_single_outcomes(vec![
        resolution::await_dependent_run(
            gate_ref.clone(),
            LoopResultRef::new("result:coalesced-retry-a").expect("valid"),
            "first dependent result".to_string(),
            0,
            None,
        )
        .resolution,
        resolution::await_dependent_run(
            gate_ref.clone(),
            LoopResultRef::new("result:coalesced-retry-b").expect("valid"),
            "second dependent result".to_string(),
            0,
            None,
        )
        .resolution,
        resolution::failed(
            FailureKind::Network,
            "retry-fated sibling failed".to_string(),
            diagnostic_failure_detail("retry-fated sibling failure"),
        ),
    ]);

    let exit = CanonicalAgentLoopExecutor
        .execute_family(
            &support::family_with_parallel_batch_execution(),
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("execute");

    let LoopExit::Blocked(blocked) = exit else {
        panic!("coalesced dependent gate must block the run");
    };
    assert_eq!(blocked.gate_ref, gate_ref);
    assert_eq!(
        host.single_invocations().len(),
        3,
        "a retry-fated sibling must not dispatch again while a coalesced gate is pending"
    );
}

/// `GateOutcome::validate_for_gate_kind` is the owning contract for every gate
/// stage. A custom strategy cannot use `SkipAndContinue` to discard an
/// `AwaitDependentRun` suspension and report a normal completion; that would
/// orphan the child-run relationship and hide a planner bug.
#[tokio::test]
async fn await_dependent_run_gate_skip_and_continue_fails_as_driver_bug() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::await_dependent_run(
                    LoopGateRef::new("gate:await-skip").expect("valid"),
                    LoopResultRef::new("result:await-skip").expect("valid"),
                    "dependent run skip and continue".to_string(),
                    33_001,
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::DriverBug);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("invalid AwaitDependentRun skip must fail as DriverBug, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "a driver contract violation must stop before another model turn"
    );
}

/// An external-tool suspension cannot be silently discarded by a custom gate
/// strategy. Drive the full executor so the assertion covers the GateStage
/// caller path, not only `GateOutcome::validate_for_gate_kind`.
#[tokio::test]
async fn external_tool_gate_skip_and_continue_fails_as_driver_bug() {
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    LoopGateRef::new("gate:external-tool-skip").expect("valid"),
                    "external tool skip and continue".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::DriverBug);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("invalid ExternalTool skip must fail as DriverBug, got {other:?}"),
    }
    assert_eq!(
        host.model_requests().len(),
        1,
        "a driver contract violation must stop before another model turn"
    );
}
