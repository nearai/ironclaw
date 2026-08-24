use super::*;

#[tokio::test]
async fn approval_resume_metadata_is_replayed_after_before_block_checkpoint() {
    let original_input_ref = CapabilityInputRef::new("input:demo").expect("valid");
    let approval_resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new("resume-token:demo").expect("valid token"),
        correlation_id: CorrelationId::new(),
        input_ref: original_input_ref.clone(),
    };
    let completed_ref = LoopResultRef::new("result:approval-resumed").expect("valid");
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
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "approval resumed".to_string(),
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
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks");

    assert!(matches!(first_exit, LoopExit::Blocked(_)));
    assert_eq!(host.model_requests().len(), 1);
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_resume = before_block_state
        .pending_approval_resume
        .as_ref()
        .expect("blocked checkpoint carries pending approval resume");
    assert_eq!(
        pending_resume.approval_request_id,
        approval_resume.approval_request_id
    );
    assert_eq!(pending_resume.resume_token, approval_resume.resume_token);
    // correlation_id is observability-only post-§5.3 Stage 2 flip: the executor
    // reconstructs the approval identity from the `gate:approval-{id}` ref and
    // regenerates a fresh correlation_id, so it is NOT byte-stable with the
    // original. The authoritative correlation is reconstituted host-side from the
    // replay payload (§5.3 Stage 2a-ii). Assert it is present, not equal.
    assert_eq!(pending_resume.surface_version, surface_version());
    assert_eq!(
        pending_resume.effective_capability_ids,
        vec![capability_id()]
    );

    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes");

    assert!(matches!(second_exit, LoopExit::Completed(_)));
    assert_eq!(
        host.model_requests().len(),
        1,
        "approval resume must dispatch the saved invocation before asking the model again"
    );
    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 2);
    assert_eq!(batch_invocations[0].invocations[0].approval_resume, None);
    // The replayed invocation carries the byte-stable approval identity
    // (request id, resume token, input ref) reconstructed from the gate ref.
    // correlation_id is observability-only post-flip and regenerated, so the
    // full struct is NOT equal to the original — assert the stable fields.
    let replayed_resume = batch_invocations[1].invocations[0]
        .approval_resume
        .as_ref()
        .expect("resume metadata");
    assert_eq!(
        replayed_resume.approval_request_id,
        approval_resume.approval_request_id
    );
    assert_eq!(replayed_resume.resume_token, approval_resume.resume_token);
    assert_eq!(replayed_resume.input_ref, approval_resume.input_ref);
    assert_eq!(
        batch_invocations[1].invocations[0].input_ref,
        original_input_ref
    );
    assert_eq!(
        batch_invocations[1].invocations[0]
            .approval_resume
            .as_ref()
            .expect("resume metadata")
            .input_ref,
        original_input_ref
    );
    assert_eq!(final_staged_state(&host).result_refs, vec![completed_ref]);
}

#[tokio::test]
async fn auth_gate_block_stores_pending_auth_resume() {
    // Drive the full executor loop so the GateStage block arm runs through the
    // canonical path (cancel-check → progress emit → write_before_block).
    let gate_ref = LoopGateRef::new("gate:auth-block").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on auth gate");

    // Exit must be a blocked (auth) exit.
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit for auth gate, got {exit:?}"
    );

    // BeforeBlock checkpoint must have been written in the expected sequence.
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::BeforeBlock,
        ]
    );

    // Recover state from the BeforeBlock checkpoint — this is what the resume
    // path will load, so it must carry the pending_auth_resume record.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);

    // Auth slot must be populated.
    let pending = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_auth_resume when auth gate blocks");
    assert_eq!(
        pending.gate_ref, gate_ref,
        "pending_auth_resume.gate_ref must match the blocked gate ref"
    );
    assert_eq!(
        pending.capability_id,
        capability_id(),
        "pending_auth_resume.capability_id must match the scripted capability"
    );

    // Approval slot must NOT be touched by an auth block.
    assert!(
        before_block_state.pending_approval_resume.is_none(),
        "auth block must not populate pending_approval_resume"
    );
}

#[tokio::test]
async fn non_auth_gate_block_preserves_pending_auth_resume() {
    // Regression test for the fix where `_ => state.pending_auth_resume.take()`
    // would erase a live auth resume record when a non-auth gate (e.g. approval)
    // blocked mid-re-dispatch.
    //
    // Scenario: auth gate previously blocked → record stored → OAuth completes →
    // resume re-dispatches the call → re-dispatch hits an APPROVAL gate → Block
    // arm must NOT clear the auth record. The auth record must survive so that
    // the outer resume handler can still consume it.
    let approval_gate_ref = LoopGateRef::new("gate:approval-during-redispatch").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    approval_gate_ref.clone(),
                    "approval required during redispatch".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;

    // Seed a live auth resume record, simulating a state that was rehydrated
    // from a BeforeBlock checkpoint written when the auth gate first blocked.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original").expect("valid");
    let seeded_auth_resume = PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    };
    let mut initial_state = LoopExecutionState::initial_for_run(host.run_context());
    initial_state.pending_auth_resume = Some(seeded_auth_resume.clone());

    let exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("execute blocks on approval gate");

    // Exit must be a Blocked exit (approval gate fired).
    assert!(
        matches!(exit, LoopExit::Blocked(_)),
        "expected Blocked exit when approval gate blocks, got {exit:?}"
    );

    // The BeforeBlock checkpoint must carry the auth resume record unchanged —
    // the approval-gate Block arm must not have erased it.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let surviving_resume = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume must survive a non-auth gate block");
    assert_eq!(
        surviving_resume.gate_ref, seeded_gate_ref,
        "surviving pending_auth_resume.gate_ref must be the original auth gate ref, not the approval gate ref"
    );
    assert_eq!(
        surviving_resume.capability_id, seeded_auth_resume.capability_id,
        "surviving pending_auth_resume.capability_id must be unchanged"
    );
}

#[tokio::test]
async fn external_tool_gate_block_stores_pending_external_tool_resume() {
    // Driving the full loop, an ExternalToolPending outcome must block the run
    // and checkpoint a pending_external_tool_resume record (so resume can
    // re-dispatch the parked client-tool call).
    let gate_ref = LoopGateRef::new("gate:external_tool-block").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    gate_ref.clone(),
                    "awaiting client tool output".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
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

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending = before_block_state
        .pending_external_tool_resume
        .as_ref()
        .expect("BeforeBlock checkpoint must carry pending_external_tool_resume");
    assert_eq!(pending.gate_ref, gate_ref);
    assert_eq!(pending.capability_id, capability_id());
    // External-tool blocks must not touch the auth/approval slots.
    assert!(before_block_state.pending_auth_resume.is_none());
    assert!(before_block_state.pending_approval_resume.is_none());
}

#[tokio::test]
async fn resume_after_external_tool_gate_redispatches_without_model_turn() {
    // Phase 1: the client tool call parks (ExternalToolPending). Phase 2: resume
    // re-dispatches the parked call WITHOUT a model turn, restaging the provider
    // tool call so the host decorator can complete it from the catalog output.
    let gate_ref = LoopGateRef::new("gate:external_tool-resume").expect("valid");
    let completed_ref = LoopResultRef::new("result:external-tool-resumed").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::external_tool_pending(
                    gate_ref.clone(),
                    "awaiting client tool output".to_string(),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "external tool output".to_string(),
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
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on external tool gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    assert_eq!(host.model_requests().len(), 1);

    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_external_tool_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_external_tool_resume"
    );

    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes from external tool gate");
    assert!(
        matches!(second_exit, LoopExit::Completed(_)),
        "expected Completed exit after external tool resume, got {second_exit:?}"
    );

    // No additional model call: the parked call is re-dispatched before the model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "external tool resume must re-dispatch without a model call"
    );
    // Two batch invocations: phase 1 (park) + phase 2 (complete).
    assert_eq!(host.batch_invocations().len(), 2);
    // Resume re-registered the provider tool call so the decorator re-binds and
    // completes from the catalog.
    assert_eq!(
        host.registered_provider_calls().len(),
        1,
        "external tool resume must restage exactly one provider tool call"
    );
}

#[tokio::test]
async fn resume_after_auth_gate_redispatches_original_call_without_model_turn() {
    // Phase 1: executor blocks on an auth gate and writes a BeforeBlock checkpoint
    // that carries a pending_auth_resume record with the original input_ref.
    let gate_ref = LoopGateRef::new("gate:auth-resume-test").expect("valid");
    let completed_ref = LoopResultRef::new("result:auth-resumed").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: the auth is now satisfied, call completes.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "auth resumed and completed".to_string(),
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
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    // Phase 1 run — expect a Blocked exit.
    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    // Exactly one model call happened during Phase 1.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // Recover the BeforeBlock checkpoint state — this is what resume loads.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_auth_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_auth_resume"
    );

    // Derive the stale input_ref from the BeforeBlock checkpoint before the
    // state is consumed by the phase 2 execute call.
    let checkpoint_input_ref = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set in BeforeBlock checkpoint")
        .input_ref
        .clone();
    let parked_activity_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set")
        .activity_id_for_resume();
    assert!(
        before_block_state
            .pending_auth_resume
            .as_ref()
            .expect("pending_auth_resume set")
            .provider_replay
            .is_some(),
        "provider-backed auth resumes must checkpoint replay metadata"
    );
    assert!(
        host.registered_provider_calls().is_empty(),
        "phase 1 model response is already a candidate; registration happens on auth resume"
    );

    // Phase 2 run — seeded from the BeforeBlock checkpoint state.
    // The prompt stage must detect pending_auth_resume and skip the model call,
    // restaging the provider replay metadata before re-dispatching the capability.
    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute resumes from auth gate");
    assert!(
        matches!(second_exit, LoopExit::Completed(_)),
        "expected Completed exit after auth resume, got {second_exit:?}"
    );

    // (a) No additional model call during Phase 2 — capability re-dispatched before model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "auth resume must re-dispatch the saved invocation without a model call"
    );

    // (b) Exactly two batch invocations total: Phase 1 (blocked) + Phase 2 (completed).
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        2,
        "expected two batch invocations (phase 1 block + phase 2 re-dispatch)"
    );
    assert_eq!(
        batch_invocations[0].invocations[0].activity_id, parked_activity_id,
        "auth gate must park the original provider activity identity"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].activity_id, parked_activity_id,
        "provider-backed auth resume must re-dispatch with the parked activity identity"
    );

    // The Phase 2 invocation must carry a freshly staged input_ref. The
    // checkpoint input_ref belonged to the old provider-call input resolver.
    assert_ne!(
        batch_invocations[1].invocations[0].input_ref, checkpoint_input_ref,
        "provider-backed auth resume must not reuse the stale checkpoint input_ref"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].input_ref.as_str(),
        "input:registered-provider-1",
        "provider-backed auth resume must invoke with the restaged provider input"
    );
    let registered_provider_calls = host.registered_provider_calls();
    assert_eq!(
        registered_provider_calls.len(),
        1,
        "auth resume must restage exactly one provider tool call"
    );
    assert_eq!(
        registered_provider_calls[0].name.as_str(),
        "demo__echo",
        "auth resume must restage the checkpointed provider tool name"
    );
    assert_eq!(
        registered_provider_calls[0].arguments,
        serde_json::json!({"message":"hello"}),
        "auth resume must restage the checkpointed provider tool arguments"
    );

    // (c) Neither invocation carries an approval_resume token.
    //     Phase 1 is a plain first invocation; phase 2 is a token-less auth re-dispatch.
    assert_eq!(
        batch_invocations[0].invocations[0].approval_resume, None,
        "phase-1 invocation must not carry an approval_resume token"
    );
    assert_eq!(
        batch_invocations[1].invocations[0].approval_resume, None,
        "auth re-dispatch must not carry an approval_resume token"
    );

    // (d) pending_auth_resume is cleared in the final state.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after successful re-dispatch"
    );

    // (e) The completed result was recorded.
    assert_eq!(
        final_state.result_refs,
        vec![completed_ref],
        "completed result ref must be recorded in final state"
    );
}

#[tokio::test]
async fn auth_resume_provider_registration_failure_fails_before_invocation() {
    let gate_ref = LoopGateRef::new("gate:auth-resume-register-fails").expect("valid");
    let completed_ref = LoopResultRef::new("result:unused-auth-resume").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()])
        .with_batch_outcomes(vec![
            ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![
                    resolution::auth_required(
                        gate_ref.clone(),
                        Vec::new(),
                        "auth required".to_string(),
                        None,
                    )
                    .resolution,
                ],
                stopped_on_suspension: true,
            },
            ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![resolution::completed(
                    completed_ref,
                    "should not invoke".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    true,
                    0,
                    None,
                    None,
                )],
                stopped_on_suspension: false,
            },
        ])
        .with_provider_registration_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "provider registration failed",
        )]);
    let executor = CanonicalAgentLoopExecutor;

    let first_exit = executor
        .execute_family(
            &crate::families::default(),
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);

    let error = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect_err("provider registration failure should fail auth resume");

    assert!(matches!(
        error,
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability
        }
    ));
    assert!(
        host.registered_provider_calls().is_empty(),
        "failed provider registration must not be recorded as staged"
    );
    assert_eq!(
        host.batch_invocations().len(),
        1,
        "phase 2 must fail before invoking the resumed capability"
    );
}

#[tokio::test]
async fn auth_resume_provider_activity_remap_fails_before_invocation() {
    let gate_ref = LoopGateRef::new("gate:auth-resume-activity-remap").expect("valid");
    let completed_ref = LoopResultRef::new("result:unused-auth-resume-remap").expect("valid");
    let host = MockHost::new(vec![provider_calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref,
                "should not invoke".to_string(),
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

    let first_exit = executor
        .execute_family(
            &crate::families::default(),
            &host,
            LoopExecutionState::initial_for_run(host.run_context()),
        )
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit, got {first_exit:?}"
    );
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let parked_activity_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("auth resume checkpointed")
        .activity_id_for_resume();
    let remapped_activity_id = loop {
        let candidate = CapabilityActivityId::new();
        if candidate != parked_activity_id {
            break candidate;
        }
    };
    host.set_provider_registration_activity_remap(remapped_activity_id);

    let error = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect_err("provider activity remap should fail auth resume");

    assert!(
        matches!(
            error,
            AgentLoopExecutorError::PlannerContract { detail }
                if detail.contains("provider replay no longer matches")
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        host.registered_provider_calls().len(),
        1,
        "phase 2 should restage the provider call before rejecting identity drift"
    );
    assert_eq!(
        host.batch_invocations().len(),
        1,
        "phase 2 must fail before invoking the remapped resumed capability"
    );
}

#[tokio::test]
async fn resume_with_still_missing_credentials_blocks_again_without_model_turn() {
    // Phase 1: scripted AuthRequired -> executor exits Blocked and writes a
    // BeforeBlock checkpoint carrying a pending_auth_resume record.
    let gate_ref = LoopGateRef::new("gate:auth-still-missing").expect("valid");
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref.clone(),
                    Vec::new(),
                    "auth required (phase 1)".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2 scripted outcome: credentials are STILL missing — block again.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:auth-still-missing-2").expect("valid"),
                    Vec::new(),
                    "auth required (phase 2 — still missing)".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());

    // Phase 1 run — expect Blocked exit.
    let first_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("first execute blocks on auth gate");
    assert!(
        matches!(first_exit, LoopExit::Blocked(_)),
        "expected Blocked exit in phase 1, got {first_exit:?}"
    );
    // Exactly one model call in phase 1.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // Recover the BeforeBlock checkpoint — this is what resume loads.
    let before_block_state = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        before_block_state.pending_auth_resume.is_some(),
        "BeforeBlock checkpoint must carry pending_auth_resume after phase 1"
    );
    let phase1_capability_id = before_block_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume set")
        .capability_id
        .clone();

    // Phase 2 run — seeded from the BeforeBlock state.
    // Credentials are still missing: the capability re-dispatches and blocks again.
    let second_exit = executor
        .execute_family(&crate::families::default(), &host, before_block_state)
        .await
        .expect("second execute — still-missing credentials path should not error");
    assert!(
        matches!(second_exit, LoopExit::Blocked(_)),
        "expected Blocked exit in phase 2 (credentials still missing), got {second_exit:?}"
    );

    // (a) No additional model call during phase 2 — re-dispatch happened without model turn.
    assert_eq!(
        host.model_requests().len(),
        1,
        "auth resume with still-missing credentials must not trigger a new model call"
    );

    // (b) Exactly two batch invocations total: phase 1 block + phase 2 re-dispatch block.
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        2,
        "expected two batch invocations (phase 1 block + phase 2 re-dispatch block)"
    );

    // (c) The new BeforeBlock checkpoint must carry a pending_auth_resume record
    //     whose capability_id matches the original one from phase 1.
    let phase2_before_block_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock checkpoint payload")
        })
        .collect();
    // There should be at least two BeforeBlock checkpoints (one per phase).
    assert!(
        phase2_before_block_states.len() >= 2,
        "expected at least two BeforeBlock checkpoints (phase 1 + phase 2)"
    );
    let phase2_resume = phase2_before_block_states
        .last()
        .expect("at least one")
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock checkpoint must carry pending_auth_resume");
    assert_eq!(
        phase2_resume.capability_id, phase1_capability_id,
        "phase 2 pending_auth_resume.capability_id must match the original capability"
    );
    // The gate_ref in the phase-2 BeforeBlock checkpoint must reflect the refreshed
    // AuthRequired outcome from phase 2, not the stale phase-1 gate ref.  This
    // proves GateStage wrote a fresh record rather than preserving the old one.
    let phase2_gate_ref = LoopGateRef::new("gate:auth-still-missing-2").expect("valid");
    assert_eq!(
        phase2_resume.gate_ref, phase2_gate_ref,
        "phase 2 pending_auth_resume.gate_ref must equal the refreshed phase-2 gate ref"
    );
}

#[tokio::test]
async fn gate_stage_skip_and_continue_clears_stale_pending_auth_resume() {
    // Bug scenario: auth record stored for capability A → resume re-dispatches A
    // → re-dispatch blocks again → GateStage runs → planner returns
    // SkipAndContinue. Without the fix, pending_auth_resume for A survives, and
    // the next prompt iteration re-dispatches A again — potential infinite
    // re-dispatch loop with no model turn.
    //
    // Driven through an Auth gate: SkipAndContinue is only a valid outcome for
    // Auth/Resource gates now that GateStage enforces
    // `GateOutcome::validate_for_gate_kind` (§5a.1) — an Approval-gate skip
    // fails the run as DriverBug (covered by the executor failure matrix).
    //
    // This test exercises GateStage directly (not the full executor) so we can
    // seed pending_auth_resume before the gate runs, mirroring the existing
    // gate_stage_skips_and_continues_records_skipped_summary pattern.
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    // Seed a pending_auth_resume for the same capability that will be dispatched
    // through GateStage — this simulates the state reloaded from a BeforeBlock
    // checkpoint that was written when the auth gate first blocked.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    state
        .pending_auth_resume
        .as_mut()
        .expect("seeded auth resume")
        .activity_id = call.activity_id;
    let gate_ref = LoopGateRef::new("gate:auth-skip-stale").expect("valid");

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

    let BatchStep::Continue(final_state) = step else {
        panic!("expected SkipAndContinue to return Continue");
    };
    assert!(
        final_state.pending_auth_resume.is_none(),
        "SkipAndContinue must clear pending_auth_resume for the skipped capability \
         to prevent an infinite re-dispatch loop on the next prompt iteration"
    );
}

#[tokio::test]
async fn gate_stage_abort_clears_stale_pending_auth_resume() {
    // Bug scenario: auth record stored for capability A → resume re-dispatches A
    // → re-dispatch returns ResourceBlocked → GateStage runs with kind Resource
    // → planner returns Abort. Without the fix, pending_auth_resume persists
    // into the Final checkpoint, leaving a stale record.
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
    // Seed a pending_auth_resume for the same capability.
    let seeded_gate_ref = LoopGateRef::new("gate:auth-original-abort").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:original-abort").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    state
        .pending_auth_resume
        .as_mut()
        .expect("seeded auth resume")
        .activity_id = call.activity_id;
    let gate_ref = LoopGateRef::new("gate:resource-abort").expect("valid");

    let step = GateStage
        .process(
            ctx,
            GateInput {
                state,
                call,
                kind: GateKind::Resource,
                gate_ref,
                credential_requirements: Vec::new(),
                approval_resume: None,
                auth_resume: None,
            },
        )
        .await
        .expect("gate stage");

    // The Abort arm must return a Failed exit and write a Final checkpoint.
    let BatchStep::Exit(LoopExit::Failed(failed)) = step else {
        panic!("expected failed exit from Abort arm");
    };
    assert_eq!(failed.reason_kind, failure_kind);
    assert!(failed.checkpoint_id.is_some());

    // The Final checkpoint must NOT carry a stale pending_auth_resume.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "Abort must clear pending_auth_resume for the aborted capability \
         to prevent a stale record from persisting into the Final checkpoint"
    );
}

#[tokio::test]
async fn gate_stage_skip_does_not_clear_auth_resume_for_different_capability() {
    // The clear is capability-scoped: a SkipAndContinue for capability B must NOT
    // erase a pending_auth_resume record belonging to capability A.
    // Driven through an Auth gate (a valid skip kind) since GateStage now
    // enforces `GateOutcome::validate_for_gate_kind` (§5a.1).
    let family = family_with_gate_outcome(GateOutcome::SkipAndContinue {
        gate: empty_gate_state(),
    });
    let host = MockHost::new(Vec::new());
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    // Seed a pending_auth_resume for a DIFFERENT capability (not the one being gated).
    let different_cap_id = ironclaw_host_api::ids::CapabilityId::new("other.cap").expect("valid");
    let seeded_gate_ref = LoopGateRef::new("gate:auth-other-cap").expect("valid");
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: seeded_gate_ref.clone(),
        capability_id: different_cap_id.clone(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:other-cap").expect("valid"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: None,
        activity_id: CapabilityActivityId::new(),
        prior_approval: None,
        disposition: None,
    });
    // The call being dispatched through GateStage is capability_id() ("demo.echo"),
    // not the seeded "other.cap".
    let call = match provider_calls_response().output {
        ParentLoopOutput::CapabilityCalls(mut calls) => calls.remove(0),
        ParentLoopOutput::AssistantReply(_) => panic!("expected provider call fixture"),
    };
    let gate_ref = LoopGateRef::new("gate:auth-skip-other").expect("valid");

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

    let BatchStep::Continue(final_state) = step else {
        panic!("expected SkipAndContinue to return Continue");
    };
    // The record for "other.cap" must survive — only the matching capability is cleared.
    let surviving = final_state
        .pending_auth_resume
        .as_ref()
        .expect("pending_auth_resume for a different capability must not be cleared");
    assert_eq!(
        surviving.capability_id, different_cap_id,
        "surviving pending_auth_resume must belong to the other capability"
    );
    assert_eq!(
        surviving.gate_ref, seeded_gate_ref,
        "surviving pending_auth_resume.gate_ref must be unchanged"
    );
}
