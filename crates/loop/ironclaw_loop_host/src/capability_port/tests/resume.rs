use super::*;

/// Guard: a `LoopRequest` with both `approval_resume` and `auth_resume` set
/// must be rejected fail-closed with `InvalidInvocation` — the two resume modes are
/// mutually exclusive and simultaneous presence indicates a malformed invocation.
#[tokio::test]
async fn invoke_capability_rejects_both_resume_modes_set() {
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::{CapabilityApprovalResume, CapabilityAuthResume};

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )])),
        dummy_result_writer(),
        dummy_milestone_sink(),
        "thread-both-resume-modes-set",
    )
    .await;

    // Obtain a valid surface_version and input_ref so the invocation
    // reaches the dispatch match — the guard fires there.
    let invocation = visible_runtime_invocation(&port).await;

    let resume_token =
        CapabilityResumeToken::new(InvocationId::new().to_string()).expect("valid token");
    let dual_resume_invocation = LoopRequest {
        activity_id: ironclaw_turns::CapabilityActivityId::new(),
        surface_version: invocation.surface_version,
        capability_id: invocation.capability_id,
        input_ref: invocation.input_ref,
        approval_resume: Some(CapabilityApprovalResume {
            approval_request_id: ApprovalRequestId::new(),
            resume_token: resume_token.clone(),
            correlation_id: CorrelationId::new(),
            input_ref: CapabilityInputRef::new("input:test-dual-resume").expect("valid input ref"),
        }),
        auth_resume: Some(CapabilityAuthResume {
            resume_token: Some(resume_token),
            disposition: None,
            prior_approval: None,
        }),
    };

    let err = port
        .invoke_capability(dual_resume_invocation)
        .await
        .expect_err("dual-resume invocation must be rejected");

    assert_eq!(
        err.kind,
        AgentLoopHostErrorKind::InvalidInvocation,
        "expected InvalidInvocation, got {:?}",
        err.kind
    );
    assert!(
        err.safe_summary.contains("mutually exclusive"),
        "error message should name the mutual-exclusion constraint: {:?}",
        err.safe_summary
    );
}

#[tokio::test]
async fn invoke_capability_rejects_approval_resume_activity_mismatch() {
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::CapabilityApprovalResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        runtime.clone(),
        dummy_result_writer(),
        dummy_milestone_sink(),
        "thread-approval-resume-activity-mismatch",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let err = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref.clone(),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id: ApprovalRequestId::new(),
                resume_token: resume_token_for_different_activity(invocation.activity_id),
                correlation_id: CorrelationId::new(),
                input_ref: invocation.input_ref,
            }),
            auth_resume: None,
        })
        .await
        .expect_err("mismatched approval resume activity must be rejected");

    assert_eq!(err.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        err.safe_summary.contains("activity identity"),
        "error should name the activity identity mismatch: {:?}",
        err.safe_summary
    );
    assert!(runtime.take_requests().is_empty());
    assert!(runtime.take_spawn_requests().is_empty());
}

#[tokio::test]
async fn invoke_capability_checks_registered_activity_on_approval_resume_input_ref() {
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::CapabilityApprovalResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingResumeHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    // The resume now reconstitutes its effective input_ref from the host
    // payload (hazard 3, §5.3 Stage 0), so seed the payload the mismatched
    // resume loads — carrying the REGISTERED input_ref — and the port then
    // runs the registered-activity check against it (the resume's own
    // loop-supplied input_ref is advisory and ignored).
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = runtime_capability_port_with_replay_store(
        &capability_id,
        &provider_id,
        runtime.clone(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        replay_store.clone(),
        "thread-approval-resume-effective-input-ref-mismatch",
    )
    .await;

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
        .await
        .expect("provider tool call registers");
    let mismatched_activity_id = loop {
        let candidate_activity = CapabilityActivityId::new();
        if candidate_activity != candidate.activity_id {
            break candidate_activity;
        }
    };
    // Seed the payload the mismatched resume reconstitutes from, keyed by the
    // resume token's invocation id, carrying the registered provider tool-call
    // input_ref so the registered-activity check has the same input to reject.
    replay_store.seed(
        ResourceScope::system(),
        InvocationId::from_uuid(mismatched_activity_id.as_uuid()),
        ReplayPayload {
            input: serde_json::json!({}),
            estimate: ResourceEstimate::default(),
            prior_approval: None,
            input_ref: candidate.input_ref.clone(),
            correlation_id: CorrelationId::new(),
        },
    );
    let err = port
        .invoke_capability(LoopRequest {
            activity_id: mismatched_activity_id,
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: CapabilityInputRef::new("input:outer-stale-approval-resume")
                .expect("valid input ref"),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id: ApprovalRequestId::new(),
                resume_token: CapabilityResumeToken::new(mismatched_activity_id.to_string())
                    .expect("valid resume token"),
                correlation_id: CorrelationId::new(),
                input_ref: candidate.input_ref,
            }),
            auth_resume: None,
        })
        .await
        .expect_err("registered approval resume input must reject activity mismatch");

    assert_eq!(err.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        err.safe_summary
            .contains("registered provider tool-call activity identity"),
        "error should name the registered activity mismatch: {:?}",
        err.safe_summary
    );
    assert_eq!(runtime.resume_request_count(), 0);
}

#[tokio::test]
async fn invoke_capability_rejects_cached_approval_resume_activity_mismatch() {
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::CapabilityApprovalResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingResumeHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    // This test injects an approval resume directly (no preceding fresh gate
    // raise), so seed the host-private replay payload the resume-read path
    // reconstitutes {input, estimate} from (§5.3 Stage 2a-i).
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = runtime_capability_port_with_replay_store(
        &capability_id,
        &provider_id,
        runtime.clone(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        replay_store.clone(),
        "thread-cached-approval-resume-activity-mismatch",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let seeded_invocation_id = InvocationId::from_uuid(invocation.activity_id.as_uuid());
    replay_store.seed(
        ResourceScope::system(),
        seeded_invocation_id,
        ReplayPayload {
            input: serde_json::json!({}),
            estimate: ResourceEstimate::default(),
            prior_approval: None,
            input_ref: invocation.input_ref.clone(),
            correlation_id: CorrelationId::new(),
        },
    );
    let resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new(invocation.activity_id.to_string())
            .expect("valid resume token"),
        correlation_id: CorrelationId::new(),
        input_ref: invocation.input_ref.clone(),
    };
    let first_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version.clone(),
            capability_id: invocation.capability_id.clone(),
            input_ref: invocation.input_ref.clone(),
            approval_resume: Some(resume.clone()),
            auth_resume: None,
        })
        .await
        .expect("matching approval resume succeeds");
    assert!(matches!(&first_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(runtime.resume_request_count(), 1);

    let mismatched_activity_id = loop {
        let candidate = CapabilityActivityId::new();
        if candidate != invocation.activity_id {
            break candidate;
        }
    };
    let err = port
        .invoke_capability(LoopRequest {
            activity_id: mismatched_activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref,
            approval_resume: Some(resume),
            auth_resume: None,
        })
        .await
        .expect_err("cached approval resume must still reject activity mismatch");

    assert_eq!(err.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        err.safe_summary.contains("activity identity"),
        "error should name the activity identity mismatch: {:?}",
        err.safe_summary
    );
    assert_eq!(
        runtime.resume_request_count(),
        1,
        "mismatched cached replay must fail before runtime resume"
    );
}

#[tokio::test]
async fn invoke_capability_rejects_auth_resume_activity_mismatch() {
    use ironclaw_loop_contracts::CapabilityAuthResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        runtime.clone(),
        dummy_result_writer(),
        dummy_milestone_sink(),
        "thread-auth-resume-activity-mismatch",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let err = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref,
            approval_resume: None,
            auth_resume: Some(CapabilityAuthResume {
                resume_token: Some(resume_token_for_different_activity(invocation.activity_id)),
                disposition: None,
                prior_approval: None,
            }),
        })
        .await
        .expect_err("mismatched auth resume activity must be rejected");

    assert_eq!(err.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        err.safe_summary.contains("activity identity"),
        "error should name the activity identity mismatch: {:?}",
        err.safe_summary
    );
    assert!(runtime.take_requests().is_empty());
    assert!(runtime.take_spawn_requests().is_empty());
}

#[tokio::test]
async fn approval_resume_with_missing_replay_payload_fails_closed() {
    // §5.3 Stage 2a-i: a resume whose host-private replay payload is ABSENT is
    // a sanitized terminal failure — the port must NOT re-dispatch with empty
    // or re-resolved input. Wire an EMPTY replay store (nothing seeded) and
    // drive a matching approval resume: the resume-read path fails CLOSED
    // before any runtime dispatch.
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::CapabilityApprovalResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingResumeHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = runtime_capability_port_with_replay_store(
        &capability_id,
        &provider_id,
        runtime.clone(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        replay_store,
        "thread-approval-resume-missing-replay-payload",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let resume = CapabilityApprovalResume {
        approval_request_id: ApprovalRequestId::new(),
        resume_token: CapabilityResumeToken::new(invocation.activity_id.to_string())
            .expect("valid resume token"),
        correlation_id: CorrelationId::new(),
        input_ref: invocation.input_ref.clone(),
    };
    let err = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref,
            approval_resume: Some(resume),
            auth_resume: None,
        })
        .await
        .expect_err("a resume with no persisted replay payload must fail closed");

    assert_eq!(
        err.kind,
        AgentLoopHostErrorKind::Unavailable,
        "a missing replay payload is a sanitized terminal failure, got {:?}",
        err.kind
    );
    assert!(
        !err.safe_summary.is_empty(),
        "the terminal failure carries a sanitized summary"
    );
    // Fail-closed BEFORE any runtime dispatch — no empty-input dispatch reached
    // the runtime.
    assert_eq!(
        runtime.resume_request_count(),
        0,
        "the run must fail before re-dispatching with empty/absent input"
    );
}

#[tokio::test]
async fn approval_resume_derives_input_ref_and_key_from_store_not_loop_supplied() {
    // Hazard 3 (§5.3 Stage 0): on resume the effective input_ref used for the
    // idempotency key + validation is reconstituted from the host-persisted
    // payload, NOT the advisory loop-supplied `resume.input_ref`. Proven two
    // ways: (1) a resume whose loop-supplied input_ref is a WRONG/stale value
    // still reconstitutes the ORIGINAL input from the store; (2) a second
    // resume differing ONLY in that advisory input_ref collapses to the SAME
    // idempotency key, so it REPLAYS the cached outcome instead of
    // re-dispatching — the key is byte-stable regardless of the loop value.
    use ironclaw_host_api::ids::ApprovalRequestId;
    use ironclaw_loop_contracts::CapabilityApprovalResume;

    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingResumeHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = runtime_capability_port_with_replay_store(
        &capability_id,
        &provider_id,
        runtime.clone(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        replay_store.clone(),
        "thread-approval-resume-store-derived-input-ref",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let seeded_invocation_id = InvocationId::from_uuid(invocation.activity_id.as_uuid());
    // The payload the FRESH gate raise persisted: the ORIGINAL input_ref +
    // input the host reconstitutes on resume.
    let original_input = serde_json::json!({"query": "original"});
    replay_store.seed(
        ResourceScope::system(),
        seeded_invocation_id,
        ReplayPayload {
            input: original_input.clone(),
            estimate: ResourceEstimate::default(),
            prior_approval: None,
            input_ref: invocation.input_ref.clone(),
            correlation_id: CorrelationId::new(),
        },
    );

    // Resume carrying a DELIBERATELY WRONG loop-supplied input_ref: the host
    // must ignore it and reconstitute the original from the store.
    let stale_ref = CapabilityInputRef::new("input:stale-loop-supplied").expect("valid input ref");
    let approval_request_id = ApprovalRequestId::new();
    let resume_token =
        CapabilityResumeToken::new(invocation.activity_id.to_string()).expect("valid resume token");
    let correlation_id = CorrelationId::new();
    let first = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version.clone(),
            capability_id: invocation.capability_id.clone(),
            input_ref: stale_ref.clone(),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id,
                resume_token: resume_token.clone(),
                correlation_id,
                input_ref: stale_ref.clone(),
            }),
            auth_resume: None,
        })
        .await
        .expect("resume reconstitutes from store despite a stale loop input_ref");
    assert!(
        matches!(&first, Resolution::Done(o) if o.verdict.is_success()),
        "resume completes from the store payload, got {first:?}"
    );
    let requests = runtime.resume_requests();
    assert_eq!(requests.len(), 1, "resume dispatched to the runtime once");
    assert_eq!(
        requests[0].4, original_input,
        "resume must dispatch the STORE-reconstituted input, not re-resolve the stale loop ref"
    );

    // A second resume differing ONLY in the advisory loop-supplied input_ref
    // derives the SAME store input_ref → SAME idempotency key → replays the
    // cached outcome (no re-dispatch). Under the pre-fix behavior the key
    // varied with the loop ref and this would re-dispatch (count 2).
    let other_stale_ref =
        CapabilityInputRef::new("input:other-stale-loop-supplied").expect("valid input ref");
    let replayed = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: other_stale_ref.clone(),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id,
                resume_token,
                correlation_id,
                input_ref: other_stale_ref,
            }),
            auth_resume: None,
        })
        .await
        .expect("second resume replays the cached outcome");
    assert!(
        matches!(&replayed, Resolution::Done(o) if o.verdict.is_success()),
        "second resume replays completion, got {replayed:?}"
    );
    assert_eq!(
        runtime.resume_request_count(),
        1,
        "byte-stable key: a differing advisory loop input_ref must NOT re-dispatch"
    );
}
