use super::*;

#[tokio::test]
async fn capability_info_is_advertised_and_returns_lazy_schema_on_request() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = Arc::new(
        HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context),
    );

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    assert!(
        surface.descriptors.iter().any(|descriptor| {
            descriptor.capability_id.as_str() == capability_info::CAPABILITY_ID
        })
    );
    let visible_filter = CapabilitySurfaceVisibleFilter::new(
        port.clone(),
        surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone()),
    );
    let filtered_tool_definitions = visible_filter
        .tool_definitions()
        .expect("filtered tool definitions");
    assert!(
        filtered_tool_definitions
            .iter()
            .any(|definition| definition.name.as_str() == capability_info::TOOL_NAME),
        "capability_info must survive the ordinary model-visible capability filter"
    );
    let tool_definitions = port.tool_definitions().expect("tool definitions");
    assert!(
        tool_definitions
            .iter()
            .any(|definition| definition.name.as_str() == capability_info::TOOL_NAME)
    );
    let capability_info_definition = tool_definitions
        .iter()
        .find(|definition| definition.name.as_str() == capability_info::TOOL_NAME)
        .expect("capability_info definition is advertised");
    assert_eq!(
        capability_info_definition.parameters["required"],
        serde_json::json!(["name"])
    );
    assert!(
        tool_definitions
            .iter()
            .any(|definition| definition.capability_id == capability_id)
    );

    let mut call = provider_tool_call();
    call.name = capability_info::provider_tool_name().expect("provider tool name");
    call.arguments = serde_json::json!({
        "capability_id": capability_id.as_str(),
        "include_schema": true
    });
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("capability_info call should register");
    assert_eq!(
        candidate.capability_id.as_str(),
        capability_info::CAPABILITY_ID
    );

    let invocation = LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: surface.version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    };
    let outcome = port
        .invoke_capability(invocation.clone())
        .await
        .expect("capability_info invocation succeeds");
    let replayed_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: invocation.activity_id,
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability_info invocation replays");

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert!(matches!(&replayed_outcome, Resolution::Done(o) if o.verdict.is_success()));
    let records = result_writer.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0.as_str(), capability_info::CAPABILITY_ID);
    assert_eq!(records[0].1["capability_id"], capability_id.as_str());
    assert_eq!(records[0].1["schema"], serde_json::json!({"type":"object"}));
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info must be served by the loop port without dispatching to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_result_write_failure_is_retryable() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-retry-result-write");
    let run_context = loop_run_context(&context).await;
    let result_writer = Arc::new(FailOnceResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id,
        )])),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let mut call = provider_tool_call();
    call.name = capability_info::provider_tool_name().expect("provider tool name");
    call.arguments = serde_json::json!({ "name": capability_id.as_str() });
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("capability_info call should register");
    let invocation = LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: surface.version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    };

    let error = port
        .invoke_capability(invocation.clone())
        .await
        .expect_err("first result write should fail");
    assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
    let retried_outcome = port
        .invoke_capability(invocation)
        .await
        .expect("second invocation should retry the write");

    assert!(matches!(&retried_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(result_writer.attempts(), 2);
}

#[tokio::test]
async fn capability_info_accepts_visible_provider_tool_name() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-provider-name");
    let run_context = loop_run_context(&context).await;
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id,
        )])),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let provider_tool_name = port
        .tool_definitions()
        .expect("tool definitions")
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("runtime capability is advertised")
        .name;

    let mut call = provider_tool_call();
    call.name = capability_info::provider_tool_name().expect("provider tool name");
    call.arguments = serde_json::json!({
        "name": provider_tool_name,
        "detail": "summary"
    });
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("capability_info call should register by provider tool name");
    assert_eq!(
        candidate.effective_capability_ids,
        vec![
            CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id"),
            capability_id.clone(),
        ],
        "known target should include both capability_info and target ids"
    );
    port.invoke_capability(LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: surface.version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    })
    .await
    .expect("capability_info invocation succeeds");

    let records = result_writer.records();
    assert_eq!(records[0].1["capability_id"], capability_id.as_str());
    assert_eq!(
        records[0].1["summary"]["notes"],
        serde_json::json!(["runtime: first_party", "effects: dispatch_capability"])
    );
}

#[tokio::test]
async fn capability_info_reports_invalid_detail_arguments_as_model_visible_failure() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-invalid-detail");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    for (index, (arguments, expected_summary)) in [
        (
            serde_json::json!({ "name": capability_id.as_str(), "include_schema": 1 }),
            "capability_info include_schema must be boolean",
        ),
        (
            serde_json::json!({ "name": capability_id.as_str(), "detail": "everything" }),
            "capability_info detail must be names, summary, or schema",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut call = provider_tool_call();
        call.id = format!("call_invalid_detail_{index}");
        call.name = capability_info::provider_tool_name().expect("provider tool name");
        call.arguments = arguments;

        port.validate_provider_tool_call(&call)
            .expect("invalid capability_info arguments should be staged for model-visible failure");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect("invalid capability_info arguments should stage");

        assert_eq!(
            candidate.effective_capability_ids,
            vec![
                CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id"),
                capability_id.clone()
            ]
        );

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: surface.version.clone(),
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("invalid arguments should return a capability failure, not a host error");

        assert!(matches!(
            &outcome,
            Resolution::Done(o)
                if o.verdict.error_kind() == Some(&FailureKind::InputEncode)
                    && o.summary.as_str() == expected_summary
        ));
    }
    assert!(
        result_writer.records().is_empty(),
        "failed capability_info calls are reported through the provider error-result path"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info failure must not dispatch to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_reports_invalid_name_inputs_as_model_visible_failure() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-invalid-name");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id,
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    for (index, arguments) in [
        serde_json::json!({}),
        serde_json::json!({ "name": "" }),
        serde_json::json!({ "name": "demo echo" }),
        serde_json::json!({ "name": "demo.echo!" }),
        serde_json::json!({ "name": "demo.écho" }),
        serde_json::json!({ "name": "a".repeat(161) }),
    ]
    .into_iter()
    .enumerate()
    {
        let mut call = provider_tool_call();
        call.id = format!("call_invalid_name_{index}");
        call.name = capability_info::provider_tool_name().expect("provider tool name");
        call.arguments = arguments;

        port.validate_provider_tool_call(&call)
            .expect("invalid capability_info names should be staged for model-visible failure");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect("invalid capability_info name should stage");

        assert_eq!(
            candidate.effective_capability_ids,
            vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
        );

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: surface.version.clone(),
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("invalid name should return a capability failure, not a host error");

        assert!(matches!(
            &outcome,
            Resolution::Done(o)
                if o.verdict.error_kind() == Some(&FailureKind::InputEncode)
        ));
    }
    assert!(
        result_writer.records().is_empty(),
        "failed capability_info calls are reported through the provider error-result path"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info failure must not dispatch to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_reports_unknown_targets_as_model_visible_failure() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-unknown-target");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id,
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    let mut call = provider_tool_call();
    call.name = capability_info::provider_tool_name().expect("provider tool name");
    call.arguments = serde_json::json!({ "name": "demo.missing" });
    assert_eq!(
        port.provider_tool_call_capability_ids(&call)
            .expect("unknown targets must remain stageable for a model-visible failure")
            .effective_capability_ids,
        vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
    );

    let mut malformed_call = provider_tool_call();
    malformed_call.id = "call_malformed_unknown_target".to_string();
    malformed_call.name = capability_info::provider_tool_name().expect("provider tool name");
    malformed_call.arguments =
        serde_json::json!({ "name": "demo.missing", "detail": "everything" });
    assert_eq!(
        port.provider_tool_call_capability_ids(&malformed_call)
            .expect("unknown targets must not pre-empt argument error reporting")
            .effective_capability_ids,
        vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
    );

    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("unknown target should stage so the model can observe the tool error");

    assert_eq!(
        candidate.effective_capability_ids,
        vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
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
        .expect("unknown target should return a capability failure, not a host error");

    assert!(matches!(
        &outcome,
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::InputEncode)
                && o.summary.as_str() == "capability_info target is not on the visible surface"
    ));
    assert!(
        result_writer.records().is_empty(),
        "failed capability_info calls are reported through the provider error-result path"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info failure must not dispatch to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_output_requires_registered_effective_target_for_visible_target() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-unstaged-target");
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
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "target should be visible even when the synthetic capability_info call is unstaged"
    );

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface.version,
            capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                .expect("synthetic capability id"),
            input_ref: CapabilityInputRef::new("input:direct-capability-info")
                .expect("test input ref"),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("unstaged synthetic invocation should return a model-visible failure");

    assert!(matches!(
        &outcome,
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::InputEncode)
                && o.summary.as_str() == "capability_info target is not on the visible surface"
    ));
    assert!(
        result_writer.records().is_empty(),
        "unstaged capability_info calls must not write hidden schema output"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info failure must not dispatch to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_output_rejects_visible_target_excluded_from_registered_effective_ids() {
    let allowed_capability_id =
        CapabilityId::new("demo.allowed").expect("valid allowed capability id");
    let denied_capability_id =
        CapabilityId::new("demo.denied").expect("valid denied capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-excluded-visible-target");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![
        visible_capability(allowed_capability_id.clone(), provider_id.clone()),
        visible_capability(denied_capability_id.clone(), provider_id),
    ]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        Arc::new(JsonInputResolver(serde_json::json!({
            "name": denied_capability_id.as_str(),
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
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == denied_capability_id),
        "target should be visible on the raw surface"
    );

    let input_ref =
        CapabilityInputRef::new("input:capability-info-excluded-target").expect("test input ref");
    let capability_info_id =
        CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id");
    let activity_id = port
        .record_provider_tool_call_registration(
            &input_ref,
            &capability_info_id,
            None,
            Some(
                [capability_info_id.clone(), allowed_capability_id]
                    .into_iter()
                    .collect(),
            ),
        )
        .expect("staged provider tool call");

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id,
            surface_version: surface.version,
            capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                .expect("synthetic capability id"),
            input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("excluded target should return a model-visible failure");

    assert!(matches!(
        &outcome,
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::InputEncode)
                && o.summary.as_str() == "capability_info target is not on the visible surface"
    ));
    assert!(
        result_writer.records().is_empty(),
        "excluded capability_info calls must not write schema output"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info failure must not dispatch to the host runtime"
    );
}

/// Regression: `capability_info` previously used `as_runtime()` for
/// surface lookup, which excluded synthetic capabilities. A model calling
/// `capability_info { name: "capability_info" }` (to introspect the tool
/// itself before using it) got `target is not on the visible surface` →
/// `InvalidInvocation` → terminal run failure instead of a helpful schema
/// response.
#[tokio::test]
async fn capability_info_can_describe_itself() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-self-lookup");
    let run_context = loop_run_context(&context).await;
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id,
        provider_id,
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    // Query by provider tool name
    let mut call = provider_tool_call();
    call.name = capability_info::provider_tool_name().expect("provider tool name");
    call.arguments = serde_json::json!({ "name": capability_info::TOOL_NAME });
    let by_tool_name = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("capability_info should be able to describe itself by tool name");
    let by_tool_name_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: by_tool_name.activity_id,
            surface_version: surface.version.clone(),
            capability_id: by_tool_name.capability_id,
            input_ref: by_tool_name.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability_info self-description by tool name succeeds");

    // Query by canonical capability id
    let mut call2 = provider_tool_call();
    call2.id = "call_2".to_string();
    call2.name = capability_info::provider_tool_name().expect("provider tool name");
    call2.arguments = serde_json::json!({ "name": capability_info::CAPABILITY_ID });
    let by_capability_id = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call2))
        .await
        .expect("capability_info should be able to describe itself by capability id");
    let by_capability_id_outcome = port
        .invoke_capability(LoopRequest {
            activity_id: by_capability_id.activity_id,
            surface_version: surface.version,
            capability_id: by_capability_id.capability_id,
            input_ref: by_capability_id.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability_info self-description by capability id succeeds");

    assert!(matches!(&by_tool_name_outcome, Resolution::Done(o) if o.verdict.is_success()));
    assert!(matches!(&by_capability_id_outcome, Resolution::Done(o) if o.verdict.is_success()));

    let records = result_writer.records();
    assert_eq!(records.len(), 2);
    for (capability_id, output) in &records {
        assert_eq!(capability_id.as_str(), capability_info::CAPABILITY_ID);
        assert_eq!(output["name"], capability_info::TOOL_NAME);
        assert_eq!(output["capability_id"], capability_info::CAPABILITY_ID);
        assert_eq!(
            output["parameters"],
            serde_json::json!(["capability_id", "detail", "include_schema", "name"])
        );
        assert!(
            output.get("summary").is_none(),
            "default detail level returns parameter names only"
        );
    }
    assert!(
        runtime.take_requests().is_empty(),
        "capability_info must be served by the loop port without dispatching to the host runtime"
    );
}

#[tokio::test]
async fn capability_info_returns_names_and_summary_details() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let context = execution_context("thread-capability-info-detail-modes");
    let run_context = loop_run_context(&context).await;
    let mut visible = visible_capability(capability_id.clone(), provider_id);
    visible.descriptor.parameters_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "count": { "type": "integer" },
            "message": { "type": "string" }
        },
        "required": ["message"],
        "allOf": [{
            "properties": {
                "limit": { "type": "integer" }
            },
            "required": ["limit"]
        }],
        "anyOf": [{
            "properties": {
                "mode": { "type": "string" }
            },
            "required": ["mode"]
        }]
    });
    let result_writer = Arc::new(RecordingResultWriter::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(RecordingHostRuntime::new(vec![visible])),
        visible_request(context),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    for (detail, expected_summary) in [(None, false), (Some("summary"), true)] {
        let mut call = provider_tool_call();
        call.name = capability_info::provider_tool_name().expect("provider tool name");
        call.arguments = serde_json::json!({ "name": capability_id.as_str() });
        if let Some(detail) = detail {
            call.arguments["detail"] = serde_json::json!(detail);
        }
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect("capability_info call should register");
        port.invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version.clone(),
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability_info invocation succeeds");

        let records = result_writer.records();
        let output = &records.last().expect("result was written").1;
        assert_eq!(
            output["parameters"],
            serde_json::json!(["count", "limit", "message", "mode"])
        );
        assert_eq!(output.get("summary").is_some(), expected_summary);
        if expected_summary {
            assert_eq!(
                output["summary"]["always_required"],
                serde_json::json!(["limit", "message"])
            );
            assert_eq!(
                output["summary"]["notes"],
                serde_json::json!(["runtime: first_party", "effects: dispatch_capability"])
            );
        }
    }
}
