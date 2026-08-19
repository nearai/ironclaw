use super::*;

/// Caller-level regression test for the approval-deny short-circuit in
/// `CapabilityStage::process`.
///
/// When a run resumes from a user-DENIED approval gate (i.e.
/// `pending_approval_resume` is set and `disposition = Some(Denied)`), the
/// executor must NOT re-dispatch the parked capability (re-dispatch → re-block
/// → infinite loop). It must:
///
/// 1. Return `TurnCompletedStep::Continue` (loop proceeds, not Blocked/Exit).
/// 2. Clear `pending_approval_resume` so the next iteration prompts the model
///    normally.
/// 3. Append a model-visible gate-declined failure observation with
///    `status = Error` and `same_call_retry = Forbidden` (via
///    `generic_failure_recovery` mapping).
/// 4. Issue zero batch-invoke calls to the capability host.
#[tokio::test]
async fn capability_stage_denied_approval_resume_surfaces_gate_declined_failure_and_continues() {
    let host = MockHost::new(Vec::new());
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let mut calls = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider calls fixture"),
    };
    let provider_replay = calls[0].provider_replay.clone();
    let denied_input_ref = calls[0].input_ref.clone();
    let denied_surface_version = calls[0].surface_version.clone();
    let denied_activity_id = CapabilityActivityId::new();
    calls[0].activity_id = denied_activity_id;

    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-test").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new(denied_activity_id.to_string())
            .expect("valid token"),
        activity_id: denied_activity_id,
        correlation_id: CorrelationId::new(),
        surface_version: denied_surface_version,
        input_ref: denied_input_ref,
        effective_capability_ids: vec![capability_id()],
        provider_replay,
        disposition: Some(GateResumeDisposition::Denied),
    });

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

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied approval resume, got Exit: {exit:?}")
        }
    };

    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared after surfacing the deny failure"
    );
    assert!(
        host.batch_invocations().is_empty(),
        "denied approval resume must not dispatch any capability batch invocations"
    );
    assert!(
        host.progress_events().iter().any(|event| matches!(
            event,
            LoopProgressEvent::CapabilityActivityFailed {
                activity_id,
                capability_id: emitted_capability_id,
                reason_kind: FailureKind::GateDeclined,
                ..
            } if *activity_id == denied_activity_id && *emitted_capability_id == capability_id()
        )),
        "denied approval resume must emit a persistent failed capability activity"
    );
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        1,
        "one model-visible result ref must be appended for the approval denial"
    );
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("model_observation must be Some for an gate-declined failure");
    assert_eq!(observation.status, ToolObservationStatus::Error);
    assert_eq!(
        observation.summary, "Capability declined by user.",
        "observation summary must describe the gate-declined failure"
    );
    let ToolObservationDetail::GenericFailure { detail, .. } = &observation.detail else {
        panic!("gate-declined observation must carry generic failure detail");
    };
    assert!(
        detail
            .as_deref()
            .is_some_and(|text| text.contains("user declined its approval request")),
        "gate-declined observation must tell the model why the capability did not run"
    );
    let recovery = observation
        .recovery
        .as_ref()
        .expect("recovery must be present");
    assert_eq!(recovery.same_call_retry, SameCallRetryConstraint::Forbidden);
}

#[tokio::test]
async fn capability_stage_denied_auth_resume_surfaces_gate_declined_failure_and_continues() {
    // Use a provider call fixture (provider_replay set) so the observation is
    // actually appended to appended_result_refs by
    // append_capability_safe_summary_ref_with_observation.
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::GateDeclined,
                "auth gate denied by user".to_string(),
                diagnostic_failure_detail("auth gate denied by user"),
            )],
            stopped_on_suspension: false,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Build state with pending_auth_resume carrying Denied disposition,
    // matching the capability_id() from provider_calls_response.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-test").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:deny-test")
            .expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: Some(
            CapabilityResumeToken::new(denied_activity_id.to_string()).expect("valid token"),
        ),
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    // Use provider_calls_response so provider_replay is set, enabling the
    // model-visible observation to be written to appended_result_refs.
    let mut calls = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider calls fixture"),
    };
    calls[0].activity_id = denied_activity_id;

    let mut current_surface = ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
        &host,
        VisibleCapabilityRequest,
    )
    .await
    .expect("visible surface");
    current_surface.descriptors.clear();
    current_surface.callable_capability_ids = Some(Vec::new());

    let step = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state,
                surface: current_surface,
                calls,
            },
        )
        .await
        .expect("capability stage");

    // 1. Must return Continue (loop proceeds, not Blocked or Failed).
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied auth resume, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared after the denied-resume path.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after surfacing the deny failure"
    );

    // 3. The denial crosses the canonical capability port as a typed terminal
    // auth resume even though the blocked capability has disappeared from the
    // current surface. The production host uses this request only to
    // terminalize the exact durable invocation; it never dispatches the
    // capability provider.
    let batches = host.batch_invocations();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].invocations.len(), 1);
    let auth_resume = batches[0].invocations[0]
        .auth_resume
        .as_ref()
        .expect("denied auth resume reaches the capability lifecycle");
    assert_eq!(
        auth_resume.disposition,
        Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
    );
    assert!(auth_resume.resume_token.is_none());
    // 4. One model-visible observation appended with GateDeclined error + Forbidden retry.
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        1,
        "exactly one model-visible result ref must be appended for the deny failure"
    );
    let observation = appended[0]
        .model_observation
        .as_ref()
        .expect("model_observation must be Some for an gate-declined failure");
    assert_eq!(
        observation.status,
        ToolObservationStatus::Error,
        "observation status must be Error"
    );
    assert_eq!(
        observation.summary, "Capability declined by user.",
        "observation summary must describe the gate-declined failure"
    );
    let recovery = observation
        .recovery
        .as_ref()
        .expect("recovery must be present");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "gate-declined failure must map to Forbidden retry constraint (model must not retry)"
    );
}

#[tokio::test]
async fn auth_gate_without_resume_token_records_activity_id_for_denial_failure() {
    let mut calls = match calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected capability call fixture"),
    };
    let blocked_activity_id = calls[0].activity_id;

    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:hook-auth-tokenless").expect("valid"),
                    Vec::new(),
                    "hook requested auth".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: the denied resume crosses the port as a typed terminal
        // auth resume; the host terminalizes it as a gate-declined failure.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::GateDeclined,
                "auth gate denied by user".to_string(),
                diagnostic_failure_detail("auth gate denied by user"),
            )],
            stopped_on_suspension: false,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    let phase1 = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state: LoopExecutionState::initial_for_run(host.run_context()),
                surface: ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
                    &host,
                    VisibleCapabilityRequest,
                )
                .await
                .expect("visible surface"),
                calls: calls.clone(),
            },
        )
        .await
        .expect("phase 1 must block on auth gate");
    assert!(
        matches!(phase1, TurnCompletedStep::Exit(LoopExit::Blocked(_))),
        "tokenless auth gate must block the turn"
    );

    let mut blocked_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending = blocked_state
        .pending_auth_resume
        .as_ref()
        .expect("tokenless auth gate must create pending auth resume");
    assert_eq!(
        pending.resume_token, None,
        "hook-style tokenless auth gates should remain tokenless"
    );
    assert_eq!(
        pending.activity_id, blocked_activity_id,
        "tokenless auth gate must persist the call activity id for later terminal events"
    );

    blocked_state
        .pending_auth_resume
        .as_mut()
        .expect("pending auth resume")
        .disposition = Some(ironclaw_host_api::turn::GateResumeDisposition::Denied);

    let phase2 = CapabilityStage
        .process(
            ctx,
            CapabilityInput {
                state: blocked_state,
                surface: ironclaw_loop_contracts::LoopCapabilityPort::visible_capabilities(
                    &host,
                    VisibleCapabilityRequest,
                )
                .await
                .expect("visible surface"),
                calls: std::mem::take(&mut calls),
            },
        )
        .await
        .expect("phase 2 must surface declined auth gate");
    let final_state = match phase2 {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after denied tokenless auth gate, got Exit: {exit:?}")
        }
    };

    assert!(
        final_state.pending_auth_resume.is_none(),
        "denied tokenless auth resume must be cleared"
    );
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        2,
        "phase 2 must send exactly one more batch: the typed denial terminalization"
    );
    assert_eq!(batches[1].invocations.len(), 1);
    let denied_resume = batches[1].invocations[0]
        .auth_resume
        .as_ref()
        .expect("denied tokenless resume reaches the capability lifecycle");
    assert_eq!(
        denied_resume.disposition,
        Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
    );
    assert!(
        denied_resume.resume_token.is_none(),
        "hook-style tokenless auth gates should remain tokenless"
    );
    // The parked activity id must travel on the typed denial so the host
    // terminalizes the ORIGINAL durable invocation (the loop no longer
    // synthesizes a CapabilityActivityFailed event for it).
    assert_eq!(
        batches[1].invocations[0].activity_id, blocked_activity_id,
        "denied tokenless auth gate must terminalize the original activity id"
    );
}

/// Regression test for the auth-deny partition fix.
///
/// When a resumed run carries `pending_auth_resume` with `disposition = Some(Denied)`
/// and the parallel batch contains BOTH the denied capability (X = `capability_id()`)
/// AND an unrelated capability (Y = `other_capability_id()`), only X must receive
/// an gate-declined failure. Y must be dispatched normally and its outcome must
/// appear in the result refs. The loop must continue (not exit), and
/// `pending_auth_resume` must be cleared.
#[tokio::test]
async fn capability_stage_denied_auth_resume_only_fails_matching_call_remaining_dispatched() {
    let y_result_ref = LoopResultRef::new("result:y-outcome").expect("valid");

    // The MockHost surface normally contains only capability_id() ("demo.echo").
    // Add other_capability_id() ("demo.list") so that call Y is treated as
    // visible rather than denied-by-surface.
    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_loop_contracts::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                description_trust: Default::default(),
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            // Order matches invocation order: X's typed denial terminalizes
            // first, then Y's normal dispatch completes.
            resolutions: vec![
                resolution::failed(
                    FailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    diagnostic_failure_detail("auth gate denied by user"),
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "list done".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_auth_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-multi-test").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:deny-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    // Build a batch with two calls:
    //   call X: capability_id() ("demo.echo")  — matches denied pending_auth_resume
    //   call Y: other_capability_id() ("demo.list") — unrelated, must proceed normally
    //
    // Use provider_replay on X so the gate-declined failure observation is
    // written to appended_result_refs (same pattern as the single-call test).
    let calls = vec![
        // call X — denied
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:x-denied")
                .expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_loop_contracts::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // call Y — unrelated, must dispatch normally
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:y-unrelated")
                .expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop must proceed, not exit.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after partial auth-deny, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared — the denial was consumed.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after the denied call was surfaced"
    );

    // 3. One batch carries BOTH calls: X as a typed terminal denial (the host
    // terminalizes the durable invocation, never dispatches the provider) and
    // Y as a plain dispatch.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry the typed denial for X and the dispatch for Y"
    );
    assert_eq!(
        batches[0].invocations.len(),
        2,
        "batch must contain X's denial terminalization and Y's dispatch"
    );
    let x_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == capability_id())
        .expect("X's invocation must be present");
    let x_resume = x_invocation
        .auth_resume
        .as_ref()
        .expect("X must carry the typed denied auth resume");
    assert_eq!(
        x_resume.disposition,
        Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
    );
    assert!(x_resume.resume_token.is_none());
    let y_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == other_capability_id())
        .expect("Y's invocation must be present");
    assert!(
        y_invocation.auth_resume.is_none(),
        "Y must dispatch as a plain invocation"
    );

    // 4. Two result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        2,
        "expected two appended result refs: one gate-declined failure for X, one Completed for Y"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let auth_failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = auth_failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // The Completed result for Y must be present (no model observation on a Completed outcome).
    let y_completed = appended
        .iter()
        .find(|r| r.result_ref == y_result_ref)
        .expect("Y's completed result ref must be appended");
    assert!(
        y_completed.model_observation.is_none()
            || y_completed
                .model_observation
                .as_ref()
                .is_some_and(|o| o.status != ToolObservationStatus::Error),
        "Y's result must not be an gate-declined failure"
    );

    // 5. final_state.result_refs must carry BOTH refs so the next model prompt sees them.
    //    A regression that appends to the host but forgets to update result_refs would still
    //    pass all checks above — this assertion catches that gap.
    assert!(
        final_state
            .result_refs
            .contains(&auth_failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
}

#[tokio::test]
async fn capability_stage_denied_auth_resume_only_fails_matching_activity_when_capability_repeats()
{
    let y_result_ref = LoopResultRef::new("result:same-cap-y-outcome").expect("valid");
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            // Order matches invocation order: the denied activity's typed
            // terminalization first, then the surviving call's completion.
            resolutions: vec![
                resolution::failed(
                    FailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    diagnostic_failure_detail("auth gate denied by user"),
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "same capability second call done".to_string(),
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

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let denied_activity_id = CapabilityActivityId::new();
    let surviving_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-same-cap").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:deny-same-cap-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    let calls = vec![
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:x-same-cap-denied").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x_same_cap".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: surviving_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:y-same-cap-survives").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        },
    ];

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

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after same-capability partial auth deny, got Exit: {exit:?}")
        }
    };

    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry the denied activity's terminalization and the surviving call"
    );
    assert_eq!(batches[0].invocations.len(), 2);
    let denied_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.activity_id == denied_activity_id)
        .expect("denied activity's invocation must be present");
    assert_eq!(
        denied_invocation
            .auth_resume
            .as_ref()
            .expect("denied activity carries the typed denied auth resume")
            .disposition,
        Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
    );
    let surviving_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.activity_id == surviving_activity_id)
        .expect("surviving activity's invocation must be present");
    assert_eq!(surviving_invocation.capability_id, capability_id());
    assert!(
        surviving_invocation.auth_resume.is_none(),
        "the surviving same-capability call must dispatch as a plain invocation"
    );
    // The typed batch above already pins that ONLY the parked activity carries
    // the denied resume (the host terminalizes the durable invocation; the
    // loop no longer synthesizes a CapabilityActivityFailed event for it).
    // The model-visible gate-declined observation lands for X alone:
    let appended = host.appended_result_refs();
    let declined_observations: Vec<_> = appended
        .iter()
        .filter_map(|entry| entry.model_observation.as_ref())
        .filter(|observation| observation.status == ToolObservationStatus::Error)
        .collect();
    assert_eq!(
        declined_observations.len(),
        1,
        "only the parked activity should receive the gate-declined failure"
    );
    assert_eq!(
        declined_observations[0].summary, "Capability declined by user.",
        "the gate-declined failure must be the model-visible declined observation"
    );
    assert!(final_state.pending_auth_resume.is_none());
    assert!(final_state.result_refs.contains(&y_result_ref));
}

/// Regression test: partition + sizing invariant with 1 denied + 2 remaining calls.
///
/// When the denied auth-resume batch contains one denied call (X) and TWO
/// unrelated remaining calls (Y and Z), the partition must place both Y and Z
/// into the fall-through batch.  `invoke_capability_batch` must receive exactly
/// 2 invocations, return 2 outcomes, and the loop must continue.
///
/// This exercises the `outcomes.len() == invocations.len()` invariant under
/// more than one remaining call, which the 1+1 sibling test above does not cover.
#[tokio::test]
async fn capability_stage_denied_auth_resume_one_denied_two_remaining_all_dispatched() {
    let y_result_ref = LoopResultRef::new("result:y-multi").expect("valid");
    let z_result_ref = LoopResultRef::new("result:z-multi").expect("valid");
    let z_capability_id =
        ironclaw_host_api::ids::CapabilityId::new("demo.write").expect("valid cap id");

    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            // Y: demo.list
            ironclaw_loop_contracts::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                description_trust: Default::default(),
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
            // Z: demo.write
            ironclaw_loop_contracts::CapabilityDescriptorView {
                capability_id: z_capability_id.clone(),
                provider: None,
                runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                safe_name: "demo_write".to_string(),
                safe_description: "demo write capability".to_string(),
                description_trust: Default::default(),
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            // Order matches invocation order: X's typed denial terminalizes
            // first, then Y and Z complete.
            resolutions: vec![
                resolution::failed(
                    FailureKind::GateDeclined,
                    "auth gate denied by user".to_string(),
                    diagnostic_failure_detail("auth gate denied by user"),
                ),
                resolution::completed(
                    y_result_ref.clone(),
                    "list done".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::completed(
                    z_result_ref.clone(),
                    "write done".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
            ],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // pending_auth_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:auth-deny-1plus2").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:deny-x-1plus2")
            .expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        resume_token: None,
        activity_id: denied_activity_id,
        prior_approval: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    // Three calls: X (denied), Y (unrelated), Z (unrelated).
    let calls = vec![
        // X — matches denied pending_auth_resume; provider_replay set so the
        // gate-declined failure observation is written to appended_result_refs.
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:x-denied-1plus2")
                .expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_loop_contracts::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // Y — unrelated, must dispatch normally.
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:y-unrelated-1plus2")
                .expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
        // Z — unrelated, must dispatch normally.
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: z_capability_id.clone(),
            input_ref: ironclaw_loop_contracts::CapabilityInputRef::new("input:z-unrelated-1plus2")
                .expect("valid"),
            effective_capability_ids: vec![z_capability_id.clone()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop proceeds with both Y and Z completed.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after 1-denied + 2-remaining, got Exit: {exit:?}")
        }
    };

    // 2. pending_auth_resume must be cleared.
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after the denied call was surfaced"
    );

    // 3. One batch carries all three: X as a typed terminal denial plus the
    //    plain dispatches for Y and Z. This validates the
    //    outcomes.len() == invocations.len() invariant for >1 remaining.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "one batch must carry X's denial terminalization plus Y and Z"
    );
    assert_eq!(
        batches[0].invocations.len(),
        3,
        "batch must contain X's denial terminalization and the Y/Z dispatches"
    );
    let x_invocation = batches[0]
        .invocations
        .iter()
        .find(|invocation| invocation.capability_id == capability_id())
        .expect("X's invocation must be present");
    assert_eq!(
        x_invocation
            .auth_resume
            .as_ref()
            .expect("X carries the typed denied auth resume")
            .disposition,
        Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
    );
    let dispatched_ids: std::collections::HashSet<_> = batches[0]
        .invocations
        .iter()
        .filter(|invocation| invocation.auth_resume.is_none())
        .map(|invocation| invocation.capability_id.clone())
        .collect();
    let expected_ids: std::collections::HashSet<_> =
        [other_capability_id(), z_capability_id.clone()]
            .into_iter()
            .collect();
    assert_eq!(
        dispatched_ids, expected_ids,
        "the plain dispatches must be exactly Y (demo.list) and Z (demo.write)"
    );

    // 4. Three result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    //    - one Completed result for Z
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        3,
        "expected 3 appended result refs: gate-declined failure for X, Completed for Y and Z"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let auth_failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = auth_failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // Both Y and Z Completed results must be present.
    assert!(
        appended.iter().any(|r| r.result_ref == y_result_ref),
        "Y's completed result ref must be appended"
    );
    assert!(
        appended.iter().any(|r| r.result_ref == z_result_ref),
        "Z's completed result ref must be appended"
    );

    // 5. final_state.result_refs must carry all three refs.
    assert!(
        final_state
            .result_refs
            .contains(&auth_failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
    assert!(
        final_state.result_refs.contains(&z_result_ref),
        "final_state.result_refs must contain Z's completed result ref"
    );
}

/// When a resumed run carries `pending_approval_resume` with
/// `disposition = Some(Denied)` and the parallel batch contains BOTH the
/// denied capability (X = `capability_id()`) AND an unrelated capability
/// (Y = `other_capability_id()`), only X must receive a gate-declined
/// failure. Y must be dispatched normally and its outcome must appear in
/// the result refs. The loop must continue, and `pending_approval_resume`
/// must be cleared.
#[tokio::test]
async fn capability_stage_denied_approval_resume_only_fails_matching_call_remaining_dispatched() {
    let y_result_ref = LoopResultRef::new("result:approval-y-outcome").expect("valid");

    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_loop_contracts::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                description_trust: Default::default(),
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                y_result_ref.clone(),
                "list done".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_approval_resume for parked activity X, Denied.
    let denied_activity_id = CapabilityActivityId::new();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-multi").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("00000000-0000-0000-0000-000000000043")
            .expect("valid"),
        activity_id: denied_activity_id,
        correlation_id: ironclaw_host_api::ids::CorrelationId::new(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:approval-deny-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    // Build a batch with two calls:
    //   call X: capability_id() ("demo.echo")  — matches denied pending_approval_resume
    //   call Y: other_capability_id() ("demo.list") — unrelated, must proceed normally
    let calls = vec![
        // call X — denied
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: denied_activity_id,
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: CapabilityInputRef::new("input:x-approval-denied").expect("valid"),
            effective_capability_ids: vec![capability_id()],
            provider_replay: Some(ironclaw_loop_contracts::ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_x".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                arguments: serde_json::json!({"message": "x"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        },
        // call Y — unrelated, must dispatch normally
        ironclaw_loop_contracts::CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: other_capability_id(),
            input_ref: CapabilityInputRef::new("input:y-approval-unrelated").expect("valid"),
            effective_capability_ids: vec![other_capability_id()],
            provider_replay: None,
        },
    ];

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

    // 1. Must return Continue — the loop must proceed, not exit.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue after partial approval-deny, got Exit: {exit:?}")
        }
    };

    // 2. pending_approval_resume must be cleared — the denial was consumed.
    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared after the denied call was surfaced"
    );

    // 3. Exactly one batch invocation containing only call Y (not X).
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "exactly one batch invocation must occur for the remaining (non-denied) call Y"
    );
    let batch_ids: Vec<_> = batches[0]
        .invocations
        .iter()
        .map(|inv| &inv.capability_id)
        .collect();
    assert!(
        batch_ids.iter().all(|id| **id == other_capability_id()),
        "the batch must contain only call Y (other_capability_id), not call X"
    );
    assert_eq!(
        batch_ids.len(),
        1,
        "batch must contain exactly one invocation (call Y)"
    );

    // 4. Two result refs appended total:
    //    - one gate-declined failure observation for X
    //    - one Completed result for Y
    let appended = host.appended_result_refs();
    assert_eq!(
        appended.len(),
        2,
        "expected two appended result refs: one gate-declined failure for X, one Completed for Y"
    );

    // The gate-declined failure for X must carry a model-visible observation.
    let failure_entry = appended
        .iter()
        .find(|r| r.model_observation.is_some())
        .expect("one appended result ref must be the gate-declined failure for X");
    let obs = failure_entry.model_observation.as_ref().unwrap();
    assert_eq!(
        obs.status,
        ToolObservationStatus::Error,
        "X failure observation status must be Error"
    );
    assert_eq!(
        obs.summary, "Capability declined by user.",
        "X failure observation summary must describe the gate-declined failure"
    );
    let recovery = obs
        .recovery
        .as_ref()
        .expect("recovery must be present for X");
    assert_eq!(
        recovery.same_call_retry,
        SameCallRetryConstraint::Forbidden,
        "X failure must map to Forbidden retry"
    );

    // The Completed result for Y must be present.
    let y_completed = appended
        .iter()
        .find(|r| r.result_ref == y_result_ref)
        .expect("Y's completed result ref must be appended");
    assert!(
        y_completed.model_observation.is_none()
            || y_completed
                .model_observation
                .as_ref()
                .is_some_and(|o| o.status != ToolObservationStatus::Error),
        "Y's result must not be an gate-declined failure"
    );

    // 5. final_state.result_refs must carry BOTH refs.
    assert!(
        final_state.result_refs.contains(&failure_entry.result_ref),
        "final_state.result_refs must contain the gate-declined failure ref for X"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );
}

/// No-match variant of the denied-approval short-circuit test.
///
/// When `pending_approval_resume` is set with `disposition = Some(Denied)` for
/// capability X but the model emits *only* calls for capability Y (no X in
/// `visible_calls`), the executor must:
///
/// 1. NOT surface X as an gate-declined failure (X is absent from the batch).
/// 2. Dispatch Y normally and record its outcome.
/// 3. Clear `pending_approval_resume` after processing the batch.
/// 4. Return `TurnCompletedStep::Continue` (loop proceeds).
///
/// This ensures the stale denied state is always evicted even when the model
/// does not reproduce the denied call in the next turn.
#[tokio::test]
async fn capability_stage_denied_approval_resume_no_matching_call_dispatches_unrelated_normally() {
    let y_result_ref = LoopResultRef::new("result:approval-no-match-y").expect("valid");

    // Only capability Y is needed in the batch outcome; X is never submitted.
    let host = MockHost::new(Vec::new())
        .with_extra_capability_descriptors(vec![
            ironclaw_loop_contracts::CapabilityDescriptorView {
                capability_id: other_capability_id(),
                provider: None,
                runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                safe_name: "demo_list".to_string(),
                safe_description: "demo list capability".to_string(),
                description_trust: Default::default(),
                parameters_schema: serde_json::json!({"type":"object","properties":{}}),
            },
        ])
        .with_batch_outcomes(vec![ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                y_result_ref.clone(),
                "list done no-match".to_string(),
                ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                false,
                0,
                None,
                None,
            )],
            stopped_on_suspension: false,
        }]);

    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };

    // Seed pending_approval_resume for capability X = capability_id(), Denied.
    // The model will emit ONLY capability Y calls — X is absent from visible_calls.
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_approval_resume = Some(PendingApprovalResume {
        gate_ref: LoopGateRef::new("gate:approval-deny-no-match").expect("valid"),
        capability_id: capability_id(),
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("00000000-0000-0000-0000-000000000099")
            .expect("valid"),
        activity_id: CapabilityActivityId::new(),
        correlation_id: ironclaw_host_api::ids::CorrelationId::new(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:approval-deny-no-match-x").expect("valid"),
        effective_capability_ids: vec![capability_id()],
        provider_replay: None,
        disposition: Some(ironclaw_host_api::turn::GateResumeDisposition::Denied),
    });

    // The model emits ONLY call Y (other_capability_id); no X in this batch.
    let calls = vec![ironclaw_loop_contracts::CapabilityCallCandidate {
        activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
        surface_version: surface_version(),
        capability_id: other_capability_id(),
        input_ref: CapabilityInputRef::new("input:y-no-match-approval").expect("valid"),
        effective_capability_ids: vec![other_capability_id()],
        provider_replay: None,
    }];

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

    // 1. Must return Continue — the loop proceeds.
    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue when denied X is absent from the batch, got Exit: {exit:?}")
        }
    };

    // 2. X must NOT appear as a failure — it was not in the visible batch.
    let appended = host.appended_result_refs();
    assert!(
        !appended.iter().any(|r| r
            .model_observation
            .as_ref()
            .is_some_and(|o| o.summary == "Capability declined by user.")),
        "X must NOT be surfaced as an gate-declined failure when it is absent from visible_calls"
    );

    // 3. Y dispatches normally: exactly one batch invocation containing Y.
    let batches = host.batch_invocations();
    assert_eq!(
        batches.len(),
        1,
        "exactly one batch invocation must occur for call Y"
    );
    assert!(
        batches[0]
            .invocations
            .iter()
            .all(|inv| inv.capability_id == other_capability_id()),
        "the batch must contain only call Y"
    );

    // 4. Y's result ref is present in appended refs and final_state.
    assert!(
        appended.iter().any(|r| r.result_ref == y_result_ref),
        "Y's completed result ref must be appended"
    );
    assert!(
        final_state.result_refs.contains(&y_result_ref),
        "final_state.result_refs must contain Y's completed result ref"
    );

    // 5. pending_approval_resume is cleared after the batch — stale denied
    //    state must not survive into the next iteration.
    assert!(
        final_state.pending_approval_resume.is_none(),
        "pending_approval_resume must be cleared even when the denied capability X was absent \
         from the model's batch"
    );
}
