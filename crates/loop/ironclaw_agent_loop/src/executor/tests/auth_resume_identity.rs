use super::*;

#[tokio::test]
async fn auth_resume_after_approval_carries_resume_token_and_approval_request_id() {
    // Regression test for the fix that makes auth-gate re-dispatch reuse the
    // ORIGINAL invocation_id so a one-shot approval lease survives the auth gate.
    //
    // Without the fix: `capability_invocation_from_auth_resume_candidate` returned
    // `auth_resume: None` because `pending_auth.resume_token` was never set.
    // With the fix: `pending_auth.resume_token` carries the approval resume token and
    // `auth_resume` is populated, allowing the host to match the fingerprinted lease.
    //
    // This test drives the full 3-phase executor path:
    //   Phase 1: model → ApprovalRequired (with resume token) → Blocked
    //   Phase 2: approval-resume re-dispatch → AuthRequired → Blocked
    //   Phase 3: auth-resume re-dispatch → Completed
    // and asserts that the phase-3 invocation carries the correct auth_resume.

    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:approval-auth-test").expect("valid token");
    let correlation_id = CorrelationId::new();
    let original_input_ref =
        CapabilityInputRef::new("input:approval-auth-original").expect("valid");
    let auth_gate_ref = LoopGateRef::new("gate:auth-after-approval").expect("valid");
    let completed_ref = LoopResultRef::new("result:auth-after-approval-done").expect("valid");

    let approval_resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: resume_token.clone(),
        correlation_id,
        input_ref: original_input_ref.clone(),
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks with resume metadata
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{approval_request_id}"))
                        .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    auth_gate_ref.clone(),
                    Vec::new(),
                    "auth required after approval".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "completed after auth resume".to_string(),
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

    // ── Phase 1: model turn → approval gate → Blocked ────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must block on approval gate");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "expected Blocked exit from approval gate; got {phase1_exit:?}"
    );
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 1 must make exactly one model call"
    );

    // BeforeBlock checkpoint carries pending_approval_resume with the resume token.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let pending_approval = phase1_bb
        .pending_approval_resume
        .as_ref()
        .expect("phase 1 BeforeBlock must carry pending_approval_resume");
    assert_eq!(
        pending_approval.resume_token, resume_token,
        "phase 1 pending_approval_resume.resume_token must match the scripted token"
    );
    assert_eq!(
        pending_approval.approval_request_id, approval_request_id,
        "phase 1 pending_approval_resume.approval_request_id must match"
    );

    // ── Phase 2: approval-resume → auth gate → Blocked ───────────────────────
    let phase2_exit = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await
        .expect("phase 2 must block on auth gate");
    assert!(
        matches!(phase2_exit, LoopExit::Blocked(_)),
        "expected Blocked exit from auth gate; got {phase2_exit:?}"
    );
    // No new model call — approval-resume re-dispatched before the model.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 2 (approval-resume) must not trigger a new model call"
    );

    // BeforeBlock checkpoint for phase 2 carries pending_auth_resume.
    // It must propagate the resume_token and prior_approval from the approval.
    let phase2_bb_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock payload")
        })
        .collect();
    assert!(
        phase2_bb_states.len() >= 2,
        "expected at least two BeforeBlock checkpoints (phase 1 + phase 2)"
    );
    let phase2_bb = phase2_bb_states.last().expect("at least one").clone();
    let pending_auth = phase2_bb
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock must carry pending_auth_resume");
    assert_eq!(
        pending_auth.resume_token,
        Some(resume_token.clone()),
        "pending_auth_resume.resume_token must carry the approval resume token"
    );
    let pending_auth_pa = pending_auth
        .prior_approval
        .as_ref()
        .expect("pending_auth_resume.prior_approval must be set when approval preceded auth");
    assert_eq!(
        pending_auth_pa.approval_request_id, approval_request_id,
        "pending_auth_resume.prior_approval.approval_request_id must match the approval request"
    );
    assert!(
        phase2_bb.pending_approval_resume.is_none(),
        "phase 2 auth gate must fold prior approval into pending_auth_resume and clear pending_approval_resume"
    );

    // ── Phase 3: auth-resume → Completed ─────────────────────────────────────
    let phase3_exit = executor
        .execute_family(&crate::families::default(), &host, phase2_bb)
        .await
        .expect("phase 3 must complete after auth resume");
    assert!(
        matches!(phase3_exit, LoopExit::Completed(_)),
        "expected Completed exit after auth resume; got {phase3_exit:?}"
    );
    // Still no additional model call.
    assert_eq!(
        host.model_requests().len(),
        1,
        "phase 3 (auth-resume) must not trigger a new model call"
    );

    // Three total batch invocations: phase 1 (approval block) + phase 2 (auth
    // block) + phase 3 (completed).
    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        3,
        "expected three batch invocations (phase 1 approval + phase 2 auth + phase 3 complete)"
    );

    // Phase 1 invocation: plain, no approval_resume and no auth_resume.
    assert_eq!(
        batch_invocations[0].invocations[0].approval_resume, None,
        "phase 1 invocation must not carry approval_resume (set on the outcome, not the request)"
    );
    assert_eq!(
        batch_invocations[0].invocations[0].auth_resume, None,
        "phase 1 invocation must not carry auth_resume"
    );

    // Phase 2 invocation: this is the approval-resume re-dispatch.
    // approval_resume is set; auth_resume is not (auth hasn't happened yet).
    assert_eq!(
        batch_invocations[1].invocations[0].auth_resume, None,
        "phase 2 (approval-resume) invocation must not carry auth_resume"
    );

    // Phase 3 invocation: this is the auth-resume re-dispatch.
    // auth_resume must be set and carry the original resume_token + prior_approval.
    // Pre-fix: auth_resume would be None (resume_token was never propagated).
    // Post-fix: auth_resume carries the token so the host can reuse the original
    // invocation identifier and match the fingerprinted approval lease.
    let phase3_auth_resume = batch_invocations[2].invocations[0]
        .auth_resume
        .as_ref()
        .expect(
            "phase 3 (auth-resume) invocation must carry auth_resume \
                 (pre-fix: was None because resume_token was not propagated)",
        );
    assert_eq!(
        phase3_auth_resume.resume_token.as_ref(),
        Some(&resume_token),
        "auth_resume.resume_token must match the original approval resume token"
    );
    let phase3_pa = phase3_auth_resume
        .prior_approval
        .as_ref()
        .expect("phase 3 auth_resume.prior_approval must be set");
    assert_eq!(
        phase3_pa.approval_request_id, approval_request_id,
        "auth_resume.prior_approval.approval_request_id must match the original approval request id"
    );
    // correlation_id is observability-only post-§5.3 Stage 2 flip: it is
    // regenerated when the approval identity is reconstructed from the gate ref,
    // so prior_approval.correlation_id is NOT byte-stable with the original (the
    // authoritative correlation is reconstituted host-side from the replay
    // payload, §5.3 Stage 2a-ii). The prior-approval identity axis lives in
    // `auth_resume_after_approval_carries_prior_approval_identity`; presence of
    // `prior_approval` is already asserted above.

    // Final state: pending_auth_resume cleared and result recorded.
    let final_state = final_staged_state(&host);
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be cleared after successful auth-resume re-dispatch"
    );
    assert_eq!(
        final_state.result_refs,
        vec![completed_ref],
        "completed result ref must be recorded"
    );
}

/// Verify that `pending_auth_resume.prior_approval` is present and carries the
/// byte-stable approval identity throughout the approval → auth-block →
/// auth-resume pipeline.
///
/// Post-§5.3 Stage 2 flip, correlation_id is observability-only: the approval
/// identity is reconstructed from the `gate:approval-{id}` ref and a fresh
/// correlation_id is minted at the executor boundary, so prior_approval's
/// correlation_id is NOT byte-stable with the approval's original one. The
/// authoritative correlation is reconstituted host-side from the replay payload
/// (§5.3 Stage 2a-ii). This test therefore asserts the weaker present-and-valid
/// contract on prior_approval; the byte-stable fields (request id, resume token)
/// are covered by `auth_resume_after_approval_carries_resume_token_and_approval_request_id`.
#[tokio::test]
async fn auth_resume_after_approval_carries_prior_approval_identity() {
    // The three-phase flow:
    //   phase 1 — model turn → approval gate (records correlation_id in approval_resume)
    //   phase 2 — approval-resume → auth gate → Blocked
    //             (pending_auth_resume.prior_approval.correlation_id must equal approval's)
    //   phase 3 — auth-resume → Completed
    //             (phase-3 invocation.auth_resume.prior_approval.correlation_id must match)

    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:corr-id-test").expect("valid token");
    let auth_gate_resume_token =
        CapabilityResumeToken::new("resume-token:corr-id-auth-gate").expect("valid token");
    let correlation_id = CorrelationId::new();
    let original_input_ref = CapabilityInputRef::new("input:corr-id-original").expect("valid");
    let completed_ref = LoopResultRef::new("result:corr-id-done").expect("valid");

    let approval_resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: resume_token.clone(),
        correlation_id,
        input_ref: original_input_ref.clone(),
    };

    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        // Phase 1: approval gate blocks.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{approval_request_id}"))
                        .expect("valid"),
                    "approval required".to_string(),
                    Some(approval_resume.clone()),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 2: auth gate blocks after approval-resume re-dispatch.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:corr-id-auth").expect("valid"),
                    Vec::new(),
                    "auth required".to_string(),
                    Some(CapabilityAuthResume {
                        gate_ref: LoopGateRef::new("gate:auth-corr-id-auth").expect("valid"),
                        resume_token: Some(auth_gate_resume_token),
                        disposition: None,
                        prior_approval: None,
                    }),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // Phase 3: auth-resume re-dispatch completes.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::completed(
                completed_ref.clone(),
                "done".to_string(),
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

    // Phase 1: model → approval gate.
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must block on approval gate");
    assert!(matches!(phase1_exit, LoopExit::Blocked(_)));

    // Phase 2: approval-resume → auth gate.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    let phase2_exit = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await
        .expect("phase 2 must block on auth gate");
    assert!(matches!(phase2_exit, LoopExit::Blocked(_)));

    // KEY: pending_auth_resume.prior_approval.correlation_id must equal the
    // original approval correlation_id written by the approval-gate outcome.
    let phase2_bb_states: Vec<_> = host
        .staged_payloads()
        .into_iter()
        .filter(|p| p.kind == LoopCheckpointKind::BeforeBlock)
        .map(|p| {
            LoopExecutionState::from_checkpoint_payload(&p.payload, CheckpointKind::BeforeBlock)
                .expect("phase 2 BeforeBlock payload")
        })
        .collect();
    let phase2_bb = phase2_bb_states.last().expect("at least one").clone();
    let pending_auth = phase2_bb
        .pending_auth_resume
        .as_ref()
        .expect("phase 2 BeforeBlock must carry pending_auth_resume");
    assert_eq!(
        pending_auth.resume_token.as_ref(),
        Some(&resume_token),
        "pending_auth_resume.resume_token must preserve the original approval invocation token"
    );
    let pending_pa = pending_auth
        .prior_approval
        .as_ref()
        .expect("pending_auth_resume.prior_approval must be set when approval preceded auth");
    // correlation_id is observability-only and regenerated at the executor
    // boundary post-flip, so it is NOT equal to the approval's original. Assert
    // prior_approval is present and its request id is byte-stable instead.
    assert_eq!(
        pending_pa.approval_request_id, approval_request_id,
        "pending_auth_resume.prior_approval.approval_request_id must equal the approval's request id"
    );

    // Phase 3: auth-resume → Completed.
    let phase3_exit = executor
        .execute_family(&crate::families::default(), &host, phase2_bb)
        .await
        .expect("phase 3 must complete");
    assert!(matches!(phase3_exit, LoopExit::Completed(_)));

    // Phase-3 invocation must carry prior_approval.correlation_id.
    let batch_invocations = host.batch_invocations();
    assert_eq!(batch_invocations.len(), 3);
    let phase3_ar = batch_invocations[2].invocations[0]
        .auth_resume
        .as_ref()
        .expect("phase 3 invocation must carry auth_resume");
    assert_eq!(
        phase3_ar.resume_token.as_ref(),
        Some(&resume_token),
        "phase 3 auth_resume.resume_token must preserve the original approval invocation token"
    );
    let phase3_pa = phase3_ar
        .prior_approval
        .as_ref()
        .expect("phase 3 auth_resume.prior_approval must be set");
    // correlation_id is observability-only post-flip and regenerated, so it does
    // NOT match the original; assert the byte-stable request id instead.
    assert_eq!(
        phase3_pa.approval_request_id, approval_request_id,
        "phase 3 auth_resume.prior_approval.approval_request_id must match the original approval request id"
    );
}

#[tokio::test]
async fn auth_resume_slot_targets_matching_activity_not_first_capability_match() {
    // Two calls can share one capability id. The resume slot belongs only to
    // the parked activity and must not be attached to, or cleared by, an
    // ordinary sibling that happens to use the same capability.
    //
    // Drive CapabilityStage directly because the prompt stage normally emits
    // the one parked resume call by itself.
    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new("resume-token:batch-dup-guard").expect("valid token");
    let correlation_id = CorrelationId::new();
    let input_ref = CapabilityInputRef::new("input:batch-dup-guard").expect("valid");

    // The ordinary sibling completes first; the resumed sibling then returns a
    // retry-fated backend failure. Clearing by capability id would erase the
    // resume origin and incorrectly dispatch a replacement call.
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::completed(
                    LoopResultRef::new("result:first").expect("valid"),
                    "first done".to_string(),
                    ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
                    false,
                    0,
                    None,
                    None,
                ),
                resolution::failed(
                    FailureKind::Backend,
                    "resumed sibling failed".to_string(),
                    diagnostic_failure_detail("resumed sibling failed"),
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

    // State with pending_auth_resume set — capability_id() matches both calls below.
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:batch-dup-auth").expect("valid"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: input_ref.clone(),
        effective_capability_ids: vec![],
        provider_replay: None,
        resume_token: Some(resume_token.clone()),
        activity_id: CapabilityActivityId::new(),
        prior_approval: Some(crate::state::AuthResumeApprovalIdentity {
            approval_request_id,
            correlation_id,
        }),
        disposition: None,
    });

    // Two calls to the same capability_id — extracted from the two_calls_response fixture.
    let calls = match two_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected calls fixture"),
    };
    state
        .pending_auth_resume
        .as_mut()
        .expect("seeded auth resume")
        .activity_id = calls[1].activity_id;

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

    let final_state = match step {
        TurnCompletedStep::Continue { state, .. } => state,
        TurnCompletedStep::Exit(exit) => {
            panic!("expected Continue from CapabilityStage; got Exit({exit:?})")
        }
    };
    assert!(
        final_state.pending_auth_resume.is_none(),
        "pending_auth_resume must be consumed after the auth-resume slot is dispatched"
    );

    let batch_invocations = host.batch_invocations();
    assert_eq!(
        batch_invocations.len(),
        1,
        "expected exactly one batch invocation"
    );
    let invocations = &batch_invocations[0].invocations;
    assert_eq!(invocations.len(), 2, "batch must have two calls");

    assert_eq!(
        invocations[0].auth_resume, None,
        "the ordinary first call must not consume a sibling activity's resume slot"
    );

    let second_auth = invocations[1]
        .auth_resume
        .as_ref()
        .expect("the matching second activity must carry auth_resume");
    assert_eq!(
        second_auth.resume_token.as_ref(),
        Some(&resume_token),
        "second call auth_resume.resume_token must match"
    );
    let second_prior_approval = second_auth
        .prior_approval
        .as_ref()
        .expect("second call auth_resume.prior_approval must be set");
    assert_eq!(
        second_prior_approval.approval_request_id, approval_request_id,
        "second call auth_resume.prior_approval.approval_request_id must match"
    );
    assert!(
        host.single_invocations().is_empty(),
        "the resumed sibling failure must remain resume-origin and suppress retry"
    );
}

#[tokio::test]
async fn truncated_batch_gate_preserves_unlaunched_sibling_auth_resume() {
    // Distinct from the full-batch case above: the host reports only the first
    // call's gate, so the second same-capability activity has not run yet.
    // Staging that first gate must not overwrite the second activity's parked
    // auth token.
    let resume_token =
        CapabilityResumeToken::new("resume-token:truncated-sibling").expect("valid token");
    let gate_ref = LoopGateRef::new("gate:truncated-prefix-auth").expect("valid gate ref");
    let host = MockHost::new(Vec::new()).with_batch_outcomes(vec![
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    gate_ref,
                    Vec::new(),
                    "prefix needs auth".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
    ]);
    let family = crate::families::default();
    let ctx = StageContext {
        planner: family.planner(),
        host: &host,
    };
    let calls = match two_calls_response().output {
        ParentLoopOutput::CapabilityCalls(calls) => calls,
        ParentLoopOutput::AssistantReply(_) => panic!("expected calls fixture"),
    };
    let parked_activity_id = calls[1].activity_id;
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.pending_auth_resume = Some(PendingAuthResume {
        gate_ref: LoopGateRef::new("gate:parked-sibling-auth").expect("valid gate ref"),
        capability_id: capability_id(),
        surface_version: surface_version(),
        input_ref: CapabilityInputRef::new("input:parked-sibling-auth").expect("valid input"),
        effective_capability_ids: Vec::new(),
        provider_replay: None,
        resume_token: Some(resume_token.clone()),
        activity_id: parked_activity_id,
        prior_approval: None,
        disposition: None,
    });
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
    let TurnCompletedStep::Continue { state, .. } = step else {
        panic!("the prefix gate must become model-visible without replacing the parked slot");
    };
    let surviving = state
        .pending_auth_resume
        .as_ref()
        .expect("unlaunched sibling resume must survive");
    assert_eq!(surviving.activity_id, parked_activity_id);
    assert_eq!(surviving.resume_token.as_ref(), Some(&resume_token));

    let invocations = host.batch_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].invocations.len(), 2);
    assert_eq!(invocations[0].invocations[0].auth_resume, None);
    assert_eq!(
        invocations[0].invocations[1]
            .auth_resume
            .as_ref()
            .and_then(|resume| resume.resume_token.as_ref()),
        Some(&resume_token)
    );
}

/// Regression test for the terminal `scope_mismatch` / `HostUnavailable` failure
/// that surfaces when a capability's approval-resume dispatch returns a transient
/// `Backend` error.
///
/// # Bug (capabilities.rs, pre-fix)
///
/// 1. Executor dispatches the capability with `approval_resume` set (batch path).
/// 2. Host returns `Failed(Backend)`.
/// 3. `handle_capability_error` clears `state.pending_approval_resume` BEFORE
///    asking the planner for a recovery outcome.
/// 4. Planner returns `RecoveryOutcome::Retry`.
/// 5. Retry dispatch calls `invoke_capability(…, None)` — `approval_resume` is
///    dropped.
/// 6. MockHost `invoke_capability` has no scripted outcome → returns
///    `Err(Internal, "single script exhausted")` → `capability_host_error` →
///    `AgentLoopExecutorError::HostUnavailable { stage: Capability }`.
///    In production the host would instead fail with `ScopeMismatch` because the
///    original run's `input_ref` has no approval context to validate against.
///
/// # Fix (Part C-sub-A)
///
/// When the failure originated from an approval-resume dispatch, intercept
/// `RecoveryOutcome::Retry` and redirect it to `ToolErrorResult` instead.
/// The model sees the real backend error and the user can re-approve — no retry
/// of the side effect, no scope_mismatch.
///
/// # What this test asserts (observable, not implementation detail)
///
/// - **Pre-fix (RED)**: `execute_family` on Phase 2 returns
///   `Err(AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })`
///   — the run terminally dies.
/// - **Post-fix (GREEN)**: `execute_family` on Phase 2 returns `Ok(LoopExit::Completed(_))`
///   — the model sees the backend error as a tool result, issues a final reply,
///   and the run completes cleanly.
#[tokio::test]
async fn resume_origin_backend_failure_does_not_die_as_scope_mismatch() {
    let cap1_request_id = ApprovalRequestId::new();
    let cap1_resume_token =
        CapabilityResumeToken::new("resume-token:sm-test-cap1").expect("valid token");
    let cap1_correlation_id = CorrelationId::new();
    let cap1_input_ref =
        CapabilityInputRef::new("input:run-original:sm-cap1-uuid").expect("valid input ref");

    let cap1_approval_resume = CapabilityApprovalResume {
        approval_request_id: cap1_request_id,
        resume_token: cap1_resume_token,
        correlation_id: cap1_correlation_id,
        input_ref: cap1_input_ref.clone(),
    };

    // Phase 1 model response: issues cap1 with original-run input_ref.
    let cap1_model_response = ironclaw_loop_contracts::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: cap1_input_ref,
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    };

    // Batch outcomes:
    //   [0] Phase 1: cap1 → ApprovalRequired → gate blocked.
    //   [1] Phase 2: cap1 approval-resume → Failed(Backend) — the bug trigger.
    let batch_outcomes = vec![
        // [0] cap1 → ApprovalRequired (gate blocked).
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::approval_required(
                    LoopGateRef::new(format!("gate:approval-{cap1_request_id}")).expect("valid"),
                    "cap1 needs approval".to_string(),
                    Some(cap1_approval_resume),
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // [1] cap1 approval-resume → Backend failure.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::Backend,
                "transient backend error during cap1 resume".to_string(),
                diagnostic_failure_detail("transient backend error during cap1 resume"),
            )],
            stopped_on_suspension: false,
        },
    ];

    // Phase 1: one model turn (cap1); Phase 2: after Backend→ToolErrorResult
    // the loop continues and needs a model turn to issue the final reply.
    let host = MockHost::new(vec![
        cap1_model_response, // Phase 1: issues cap1
        reply_response(),    // Phase 2 (post-fix): model sees tool error, issues reply
    ])
    .with_batch_outcomes(batch_outcomes);
    // Deliberately NO single_outcomes: pre-fix, the retry would consume one and
    // get `Err(Internal)` → HostUnavailable.  Post-fix, no retry is attempted.

    let executor = CanonicalAgentLoopExecutor;

    // ── Phase 1: cap1 → ApprovalRequired → Blocked ───────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must succeed (blocks on cap1 gate)");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "phase 1 must block on cap1 approval gate; got {phase1_exit:?}"
    );

    // Recover the BeforeBlock checkpoint state to use as Phase 2 input.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        phase1_bb.pending_approval_resume.is_some(),
        "phase 1 BeforeBlock must carry pending_approval_resume"
    );
    assert_eq!(
        phase1_bb
            .pending_approval_resume
            .as_ref()
            .unwrap()
            .approval_request_id,
        cap1_request_id,
        "phase 1 BeforeBlock pending_approval_resume must be for cap1"
    );

    // ── Phase 2: approve cap1 → approval-resume → Backend failure ─────────────
    //
    // BUG TRIGGER: the batch returns Failed(Backend) for the approval-resume
    // dispatch.  On unfixed code:
    //   handle_capability_error clears pending_approval_resume at L637 BEFORE
    //   recovery decides → retry fires with None → invoke_capability has no
    //   scripted outcome → Err(Internal) → HostUnavailable.
    //
    // After fix (Part C-sub-A):
    //   Resume-origin Backend failure is intercepted before any retry → surfaced
    //   as ToolErrorResult → loop continues → model issues final reply → Done.
    let phase2_result = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await;

    // Primary assertion: the run must NOT die as HostUnavailable.
    // Pre-fix this panics; post-fix it passes.
    let phase2_exit = phase2_result.expect(
        "REGRESSION: resume-origin Backend failure must not kill the run as HostUnavailable. \
         Pre-fix: handle_capability_error clears pending_approval_resume BEFORE recovery \
         decides to retry → retry fires invoke_capability with approval_resume=None → \
         single script exhausted → HostUnavailable (in production: ScopeMismatch). \
         Fix: intercept Retry for resume-origin failures and surface as ToolErrorResult \
         so the model can re-approve without a terminal run death.",
    );

    // Post-fix: the model saw the tool error, issued a final reply → Completed.
    assert!(
        matches!(phase2_exit, LoopExit::Completed(_)),
        "phase 2 must complete the run after Backend→ToolErrorResult; got {phase2_exit:?}"
    );

    // No single invoke_capability calls should have been made: the C-sub-A guard
    // prevents the retry dispatch entirely for resume-origin failures.
    assert!(
        host.single_invocations().is_empty(),
        "no single invoke_capability call must be made for a resume-origin Backend failure \
         (retry is suppressed to avoid double-exec)"
    );
    assert_eq!(
        host.progress_events()
            .into_iter()
            .filter(|event| matches!(
                event,
                LoopProgressEvent::FailureRecovered {
                    sequence: 1,
                    stage: LoopRecoveryStage::Capability,
                    class: LoopRecoveryClass::Capability(FailureKind::Backend),
                    disposition: LoopRecoveryDisposition::ModelVisible,
                }
            ))
            .count(),
        1,
        "the redirected resume-origin failure must emit one model-visible recovery event"
    );
}

/// Regression test for the terminal `scope_mismatch` / `HostUnavailable` failure
/// that surfaces when a capability's **auth-resume** dispatch returns a transient
/// `Backend` error.
///
/// # Bug (capabilities.rs, pre-fix)
///
/// 1. Phase 1: executor dispatches the capability; host returns `AuthRequired`.
///    GateStage stores `pending_auth_resume` in the BeforeBlock checkpoint.
/// 2. Phase 2: executor detects `pending_auth_resume` in the prompt stage,
///    re-dispatches the capability via `invoke_capability_batch(auth_resume=…)`.
/// 3. Host returns `Failed(Backend)` for the re-dispatch.
/// 4. `handle_capability_error` clears `state.pending_auth_resume` at ~L667
///    BEFORE asking the planner for a recovery outcome.
/// 5. Planner returns `RecoveryOutcome::Retry`.
/// 6. Retry calls `invoke_capability(…)` (single, non-batch) with no auth context.
///    MockHost has no scripted single outcome → `Err(Internal, "single script
///    exhausted")` → `capability_host_error` → `HostUnavailable { stage: Capability }`.
///    In production the product adapter would instead fail with `ScopeMismatch`
///    because the original run's `input_ref` has no auth context to validate against.
///
/// # Fix (Part C-sub-A extended to auth-resume)
///
/// Before clearing `pending_auth_resume`, snapshot whether this failure is
/// auth-resume-origin (`captured_auth_resume_origin`).  When `is_resume_origin`
/// is true (either approval- or auth-resume origin), intercept
/// `RecoveryOutcome::Retry` and redirect it to `ToolErrorResult` instead.
/// The model sees the real backend error and the user can re-authenticate —
/// no retry of the side effect, no scope_mismatch.
///
/// # What this test asserts (observable, not implementation detail)
///
/// - **Pre-fix (RED)**: Phase 2 `execute_family` returns
///   `Err(AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })`
///   — the run terminally dies.
/// - **Post-fix (GREEN)**: Phase 2 returns `Ok(LoopExit::Completed(_))` — the
///   model sees the backend error as a tool result, issues a final reply, and
///   the run completes cleanly.
#[tokio::test]
async fn auth_resume_origin_backend_failure_does_not_die_as_scope_mismatch() {
    let cap1_input_ref =
        CapabilityInputRef::new("input:run-original:auth-sm-cap1-uuid").expect("valid input ref");

    // Phase 1 model response: issues cap1 with original-run input_ref.
    // (No provider_replay — this is a non-provider-backed auth resume, so
    // Phase 2 reuses the stored input_ref directly via
    // pending_auth_resume_staged_input_candidate.)
    let cap1_model_response = ironclaw_loop_contracts::LoopModelResponse {
        chunks: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: capability_id(),
            input_ref: cap1_input_ref,
            effective_capability_ids: vec![capability_id()],
            provider_replay: None,
        }]),
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("model")
            .expect("valid"),
        usage: None,
    };

    // Batch outcomes:
    //   [0] Phase 1: cap1 → AuthRequired → gate blocked.
    //   [1] Phase 2: cap1 auth-resume → Failed(Backend) — the bug trigger.
    let batch_outcomes = vec![
        // [0] Phase 1: cap1 → AuthRequired (gate blocked).
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![
                resolution::auth_required(
                    LoopGateRef::new("gate:auth-sm-test-cap1").expect("valid"),
                    Vec::new(),
                    "cap1 needs auth".to_string(),
                    None,
                )
                .resolution,
            ],
            stopped_on_suspension: true,
        },
        // [1] Phase 2: cap1 auth-resume → Backend failure.
        ironclaw_host_api::resolution::ResolutionBatch {
            resolutions: vec![resolution::failed(
                FailureKind::Backend,
                "transient backend error during cap1 auth-resume".to_string(),
                diagnostic_failure_detail("transient backend error during cap1 auth-resume"),
            )],
            stopped_on_suspension: false,
        },
    ];

    // Phase 1: one model turn (cap1); Phase 2: after Backend→ToolErrorResult
    // the loop continues and needs a model turn to issue the final reply.
    // (Auth-resume Phase 2 skips the model for the capability re-dispatch but
    // needs a model call AFTER the error is surfaced for the final reply.)
    let host = MockHost::new(vec![
        cap1_model_response, // Phase 1: issues cap1
        reply_response(),    // Phase 2 (post-fix): model sees tool error, issues reply
    ])
    .with_batch_outcomes(batch_outcomes);
    // Deliberately NO single_outcomes: pre-fix, the retry would consume one and
    // get `Err(Internal)` → HostUnavailable.  Post-fix, no retry is attempted.

    let executor = CanonicalAgentLoopExecutor;

    // ── Phase 1: cap1 → AuthRequired → Blocked ──────────────────────────────
    let initial_state = LoopExecutionState::initial_for_run(host.run_context());
    let phase1_exit = executor
        .execute_family(&crate::families::default(), &host, initial_state)
        .await
        .expect("phase 1 must succeed (blocks on cap1 auth gate)");
    assert!(
        matches!(phase1_exit, LoopExit::Blocked(_)),
        "phase 1 must block on cap1 auth gate; got {phase1_exit:?}"
    );

    // Recover the BeforeBlock checkpoint state to use as Phase 2 input.
    let phase1_bb = final_staged_state_for_kind(&host, LoopCheckpointKind::BeforeBlock);
    assert!(
        phase1_bb.pending_auth_resume.is_some(),
        "phase 1 BeforeBlock must carry pending_auth_resume"
    );
    assert_eq!(
        phase1_bb
            .pending_auth_resume
            .as_ref()
            .unwrap()
            .capability_id,
        capability_id(),
        "phase 1 BeforeBlock pending_auth_resume must be for cap1"
    );

    // ── Phase 2: OAuth completes → auth-resume → Backend failure ────────────
    //
    // BUG TRIGGER: the batch returns Failed(Backend) for the auth-resume
    // dispatch.  On unfixed code:
    //   handle_capability_error clears pending_auth_resume at ~L667 BEFORE
    //   recovery decides → retry fires with None → invoke_capability has no
    //   scripted outcome → Err(Internal) → HostUnavailable.
    //
    // After fix (Part C-sub-A extended to auth-resume):
    //   Auth-resume-origin Backend failure is intercepted before any retry →
    //   surfaced as ToolErrorResult → loop continues → model issues final
    //   reply → Done.
    let phase2_result = executor
        .execute_family(&crate::families::default(), &host, phase1_bb)
        .await;

    // Primary assertion: the run must NOT die as HostUnavailable.
    // Pre-fix this panics; post-fix it passes.
    let phase2_exit = phase2_result.expect(
        "REGRESSION: auth-resume-origin Backend failure must not kill the run as \
         HostUnavailable. Pre-fix: handle_capability_error clears pending_auth_resume \
         BEFORE recovery decides to retry → retry fires invoke_capability with \
         auth_resume=None → single script exhausted → HostUnavailable (in production: \
         ScopeMismatch). Fix: intercept Retry for auth-resume-origin failures and \
         surface as ToolErrorResult so the model can re-auth without a terminal run death.",
    );

    // Post-fix: the model saw the tool error, issued a final reply → Completed.
    assert!(
        matches!(phase2_exit, LoopExit::Completed(_)),
        "phase 2 must complete the run after Backend→ToolErrorResult; got {phase2_exit:?}"
    );

    // No single invoke_capability calls should have been made: the C-sub-A guard
    // prevents the retry dispatch entirely for auth-resume-origin failures.
    assert!(
        host.single_invocations().is_empty(),
        "no single invoke_capability call must be made for an auth-resume-origin Backend \
         failure (retry is suppressed to avoid double-exec)"
    );
    assert_eq!(
        host.progress_events()
            .into_iter()
            .filter(|event| matches!(
                event,
                LoopProgressEvent::FailureRecovered {
                    sequence: 1,
                    stage: LoopRecoveryStage::Capability,
                    class: LoopRecoveryClass::Capability(FailureKind::Backend),
                    disposition: LoopRecoveryDisposition::ModelVisible,
                }
            ))
            .count(),
        1,
        "the redirected auth-resume-origin failure must emit one model-visible recovery event"
    );
}
