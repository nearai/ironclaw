use super::*;

#[tokio::test]
async fn duplicate_provider_tool_call_registration_reuses_activity_id_and_cached_invocation() {
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
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-provider-duplicate-activity",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let provider_call = provider_tool_call();
    let first = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call.clone()))
        .await
        .expect("first provider tool call registers");
    let second = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
        .await
        .expect("duplicate provider tool call registers");

    assert_eq!(
        second.input_ref, first.input_ref,
        "duplicate provider calls canonicalize to the same staged input"
    );
    assert_eq!(
        second.activity_id, first.activity_id,
        "duplicate provider calls must preserve the same activity identity"
    );

    let first_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: first.activity_id,
            surface_version: surface.version.clone(),
            capability_id: first.capability_id.clone(),
            input_ref: first.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("first invocation succeeds");
    let replayed_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: second.activity_id,
            surface_version: surface.version,
            capability_id: second.capability_id,
            input_ref: second.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("duplicate invocation replays cached outcome");

    assert!(matches!(&first_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert!(matches!(&replayed_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(
        runtime.take_requests().len(),
        1,
        "duplicate provider registration must not create a second runtime dispatch"
    );
}

#[tokio::test]
async fn provider_tool_call_registration_for_activity_records_requested_activity() {
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
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-provider-requested-activity",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let provider_call = provider_tool_call();
    let activity_id = CapabilityActivityId::new();
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
            provider_call.clone(),
            activity_id,
        ))
        .await
        .expect("provider tool call registers with requested activity");

    assert_eq!(candidate.activity_id, activity_id);

    let duplicate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
        .await
        .expect("duplicate provider tool call registers");
    assert_eq!(
        duplicate.activity_id, activity_id,
        "ordinary duplicate registration must reuse the requested activity"
    );

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id,
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("requested registered activity should dispatch");

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(runtime.take_requests().len(), 1);
}

#[tokio::test]
async fn provider_tool_call_registration_accepts_password_and_traceback_reasoning_text() {
    // W4-PROVIDER-VALIDATE (#5001 caller gap): the crude
    // `SENSITIVE_PROVIDER_TEXT_MARKERS` substring scan on provider
    // reasoning/response_reasoning/signature text was removed in favor of
    // the entropy-based `LeakDetector` (#5001, PinchBench bucket D) --
    // bare English words like "password"/"traceback" in legitimate
    // analysis reasoning must be ACCEPTED, not rejected (the old scan
    // false-positived on exactly this kind of text and drove
    // retry/give-up loops). `capability_port/provider_validation.rs`'s
    // own unit test pins this at the private free-function level
    // (`validate_provider_tool_call` called directly); this drives it
    // through the REAL production caller instead --
    // `LoopCapabilityPort::validate_provider_tool_call` /
    // `register_provider_tool_call` / `invoke_capability` on
    // `HostRuntimeLoopCapabilityPort`, the same port the agent loop
    // calls -- per the test-through-the-caller rule.
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
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-provider-password-traceback",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    let mut call = provider_tool_call();
    call.response_reasoning =
        Some("provider error included a traceback; the user's password had expired".to_string());
    call.reasoning = Some("checked the traceback output for a leaked password field".to_string());
    call.signature = Some("password-traceback-review".to_string());

    port.validate_provider_tool_call(&call)
        .expect("password/traceback reasoning text must be accepted, not rejected (#5001)");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("password/traceback reasoning text must register, not be staged as a failure");

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("accepted call should dispatch, not error");
    assert!(
        matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()),
        "expected a real Completed dispatch (proving the call was genuinely accepted, not \
         silently downgraded to a model-visible failure), got {outcome:?}"
    );
    assert_eq!(runtime.take_requests().len(), 1);
}

#[tokio::test]
async fn provider_tool_call_registration_rejects_capability_remap_for_same_input() {
    let first_capability_id = CapabilityId::new("demo.a__b").expect("valid capability id");
    let remapped_capability_id = CapabilityId::new("demo.a.b").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let mut context = execution_context("thread-provider-capability-remap");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.extend([
        dispatch_capability_grant(&first_capability_id, &loop_driver_extension),
        dispatch_capability_grant(&remapped_capability_id, &loop_driver_extension),
    ]);
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        first_capability_id.clone(),
        provider_id.clone(),
    )]));
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id.clone(),
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);

    port.visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("first visible surface loads");
    let mut provider_call = provider_tool_call();
    provider_call.name = ProviderToolName::new("demo__a__b").expect("provider tool name");
    let first = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call.clone()))
        .await
        .expect("first provider call registers");
    assert_eq!(first.capability_id, first_capability_id);

    runtime.set_capabilities(vec![visible_capability(
        remapped_capability_id,
        provider_id,
    )]);
    port.visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("remapped visible surface loads");
    let error = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
        .await
        .expect_err("same provider input remapped to another capability must fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error.safe_summary.contains("capability identity"),
        "error should name capability identity drift: {:?}",
        error.safe_summary
    );
}

#[tokio::test]
async fn runtime_provider_call_rejects_registered_activity_mismatch_without_replay_poisoning() {
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
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-provider-runtime-activity-mismatch",
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
        let candidate_id = CapabilityActivityId::new();
        if candidate_id != candidate.activity_id {
            break candidate_id;
        }
    };

    let error = port
        .invoke_capability(LoopRequest {
            activity_id: mismatched_activity_id,
            surface_version: surface.version.clone(),
            capability_id: candidate.capability_id.clone(),
            input_ref: candidate.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect_err("registered activity mismatch must be rejected before dispatch");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        runtime.take_requests().is_empty(),
        "mismatched activity must not reach runtime dispatch"
    );

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("correct registered activity should still dispatch");

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(
        runtime.take_requests().len(),
        1,
        "failed mismatched attempt must not poison the correct invocation"
    );
}

#[tokio::test]
async fn provider_tool_call_registration_reuses_activity_after_many_other_calls() {
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
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-provider-activity-after-many-calls",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let provider_call = provider_tool_call();
    let first = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call.clone()))
        .await
        .expect("first provider tool call registers");
    let first_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: first.activity_id,
            surface_version: surface.version.clone(),
            capability_id: first.capability_id.clone(),
            input_ref: first.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("first invocation succeeds");

    for index in 0..160 {
        let mut call = provider_tool_call();
        call.id = format!("call_distinct_{index}");
        call.arguments = serde_json::json!({ "message": format!("distinct-{index}") });
        port.register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect("distinct provider tool call registers");
    }

    let second = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
        .await
        .expect("original provider tool call registers again");

    assert_eq!(
        second.input_ref, first.input_ref,
        "duplicate provider calls canonicalize to the same staged input"
    );
    assert_eq!(
        second.activity_id, first.activity_id,
        "duplicate provider calls must reuse the activity id from their registration record"
    );

    let replayed_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: second.activity_id,
            surface_version: surface.version,
            capability_id: second.capability_id,
            input_ref: second.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("duplicate invocation replays cached outcome");

    assert!(matches!(&first_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert!(matches!(&replayed_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(
        runtime.take_requests().len(),
        1,
        "cached replay for the duplicate provider call must not dispatch again"
    );
}

#[tokio::test]
async fn capability_info_output_rejects_registered_activity_mismatch() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-activity-mismatch");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        Arc::new(JsonInputResolver(serde_json::json!({
            "name": capability_id.as_str(),
            "detail": "schema"
        }))),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let input_ref =
        CapabilityInputRef::new("input:capability-info-activity-mismatch").expect("input ref");
    let capability_info_id =
        CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id");
    let registered_activity_id = port
        .record_provider_tool_call_registration(
            &input_ref,
            &capability_info_id,
            None,
            Some(
                [capability_info_id.clone(), capability_id]
                    .into_iter()
                    .collect(),
            ),
        )
        .expect("registered provider tool call");
    let mismatched_activity_id = loop {
        let candidate = CapabilityActivityId::new();
        if candidate != registered_activity_id {
            break candidate;
        }
    };

    let error = port
        .invoke_capability(LoopRequest {
            activity_id: mismatched_activity_id,
            surface_version: surface.version.clone(),
            capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                .expect("synthetic capability id"),
            input_ref: input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect_err("registered activity mismatch must be rejected");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(
        error.safe_summary.contains("activity identity"),
        "error should name the activity identity mismatch: {:?}",
        error.safe_summary
    );
    assert!(result_writer.records().is_empty());
    assert!(runtime.take_requests().is_empty());

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: registered_activity_id,
            surface_version: surface.version,
            capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                .expect("synthetic capability id"),
            input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("correct registered activity should still succeed");

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert!(
        !result_writer.records().is_empty(),
        "correct activity should write capability_info output"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info should remain synthetic after mismatch retry"
    );
}

#[test]
fn provider_tool_call_registration_store_keeps_activity_and_effective_ids_together() {
    let mut store = ProviderToolCallRegistrationStore::default();
    let input_ref =
        CapabilityInputRef::new("input:registered-capability").expect("valid input ref");
    let capability_id = CapabilityId::new("capability.info").expect("valid capability id");
    let effective_ids = [
        capability_id.clone(),
        CapabilityId::new("demo.echo").expect("valid capability id"),
    ]
    .into_iter()
    .collect::<HashSet<_>>();

    let first_activity_id = store
        .record(
            &input_ref,
            &capability_id,
            None,
            Some(effective_ids.clone()),
        )
        .expect("first registration");
    let second_activity_id = store
        .record(&input_ref, &capability_id, None, None)
        .expect("duplicate registration");

    assert_eq!(second_activity_id, first_activity_id);
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .effective_capability_ids,
        Some(effective_ids)
    );
}

#[test]
fn provider_tool_call_registration_store_rejects_activity_changes() {
    let mut store = ProviderToolCallRegistrationStore::default();
    let input_ref =
        CapabilityInputRef::new("input:registered-activity-conflict").expect("input ref");
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let first_activity_id = CapabilityActivityId::new();
    let second_activity_id = loop {
        let candidate = CapabilityActivityId::new();
        if candidate != first_activity_id {
            break candidate;
        }
    };

    store
        .record(&input_ref, &capability_id, Some(first_activity_id), None)
        .expect("first registration");
    let error = store
        .record(&input_ref, &capability_id, Some(second_activity_id), None)
        .expect_err("conflicting duplicate activity must fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .activity_id,
        first_activity_id
    );
}

#[test]
fn provider_tool_call_registration_store_rejects_capability_changes() {
    let mut store = ProviderToolCallRegistrationStore::default();
    let input_ref = CapabilityInputRef::new("input:registered-provider-remap").expect("input ref");
    let first_capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let second_capability_id = CapabilityId::new("demo.other").expect("valid capability id");

    let activity_id = store
        .record(&input_ref, &first_capability_id, None, None)
        .expect("first registration");
    let error = store
        .record(&input_ref, &second_capability_id, None, None)
        .expect_err("conflicting duplicate capability must fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .activity_id,
        activity_id
    );
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .capability_id,
        first_capability_id
    );
}

#[test]
fn provider_tool_call_registration_store_rejects_effective_id_changes() {
    let mut store = ProviderToolCallRegistrationStore::default();
    let input_ref =
        CapabilityInputRef::new("input:registered-capability-conflict").expect("input ref");
    let capability_id = CapabilityId::new("capability.info").expect("valid capability id");
    let first_ids = [
        capability_id.clone(),
        CapabilityId::new("demo.echo").expect("valid capability id"),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    let second_ids = [
        CapabilityId::new("capability.info").expect("valid capability id"),
        CapabilityId::new("demo.files").expect("valid capability id"),
    ]
    .into_iter()
    .collect::<HashSet<_>>();

    let activity_id = store
        .record(&input_ref, &capability_id, None, Some(first_ids.clone()))
        .expect("first registration");
    let error = store
        .record(&input_ref, &capability_id, None, Some(second_ids))
        .expect_err("conflicting duplicate registration must fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .activity_id,
        activity_id
    );
    assert_eq!(
        store
            .registration_for(&input_ref)
            .expect("registration")
            .effective_capability_ids,
        Some(first_ids)
    );
}
