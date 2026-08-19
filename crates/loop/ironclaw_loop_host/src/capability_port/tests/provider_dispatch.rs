use super::*;

#[tokio::test]
async fn provider_tool_call_input_resolver_stages_arguments() {
    let run_context = loop_run_context(&execution_context("thread-provider-input")).await;
    let resolver = ProviderToolCallInputResolver::new(Arc::new(FallbackInputResolver));
    let call = provider_tool_call();

    let input_ref = resolver
        .register_provider_tool_call_input(&run_context, &call)
        .await
        .expect("provider input should stage");
    let resolved = resolver
        .resolve_capability_input(&run_context, &input_ref)
        .await
        .expect("provider input should resolve");

    assert!(input_ref.as_str().starts_with("input:provider-tool-"));
    assert_eq!(resolved, serde_json::json!({"message":"hello"}));
}

/// Regression (#activity-card-args): the decorator bypasses the inner
/// `register_provider_tool_call_input`, so it MUST forward the
/// display-preview hook to the inner resolver — and key it by the resolved
/// dotted capability id (`nearai.web_search`), not the lossy provider tool
/// name (`nearai__web_search`). Otherwise the activity card renders the
/// wrong name and the per-tool summary/subtitle matchers miss.
#[tokio::test]
async fn provider_tool_call_input_resolver_forwards_display_input_hook_with_capability_id() {
    let run_context = loop_run_context(&execution_context("thread-display-input")).await;
    let inner = Arc::new(DisplayInputRecordingResolver::default());
    let resolver = ProviderToolCallInputResolver::new(inner.clone());
    let call = provider_tool_call();
    let input_ref = provider_tool_call_input_ref(&run_context, &call).expect("ref");
    let capability_id = CapabilityId::new("nearai.web_search").expect("capability id");

    resolver.record_provider_tool_call_display_input(
        &run_context,
        &input_ref,
        &capability_id,
        &call,
    );

    let recorded = inner.recorded.lock().expect("recorded lock").clone();
    assert_eq!(recorded.len(), 1, "display input forwarded exactly once");
    let (recorded_ref, recorded_capability, recorded_args) = &recorded[0];
    assert_eq!(
        recorded_ref,
        input_ref.as_str(),
        "display input must be recorded under the canonical ref the result write later uses",
    );
    assert_eq!(
        recorded_capability, "nearai.web_search",
        "display input must be keyed by the resolved dotted capability id",
    );
    assert_eq!(recorded_args, &call.arguments);
}

#[tokio::test]
async fn invoke_capability_forwards_resolved_input_to_trajectory_observer() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let observer = Arc::new(RecordingTrajectoryObserver::default());

    // Mirror `runtime_capability_port`, but attach the trajectory observer
    // to the factory via `with_trajectory_observer` so the port forwards the
    // resolved tool-call input when a capability is invoked.
    let mut context = execution_context("thread-trajectory-observer-input");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )])),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id.clone(),
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
    )
    .with_trajectory_observer(Some(
        observer.clone() as Arc<dyn CapabilityTrajectoryObserver>
    ))
    .port_for_run_context(run_context);

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("capability invocation succeeds");
    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));

    let inputs = observer.inputs.lock().expect("inputs lock");
    assert_eq!(
        inputs.len(),
        1,
        "observer should see exactly one capability input"
    );
    let (call_id, observed_capability, arguments) = &inputs[0];
    assert!(!call_id.is_empty(), "call_id (input ref) should be present");
    assert_eq!(
        observed_capability,
        capability_id.as_str(),
        "observer should receive the resolved capability id"
    );
    assert_eq!(
        arguments,
        &serde_json::json!({"message": "hello"}),
        "observer should receive the resolved tool-call arguments"
    );
}

#[tokio::test]
async fn versioned_union_schema_is_advertised_and_dispatches_through_runtime_port() {
    let capability_id = CapabilityId::new("evm-rpc.invoke").expect("valid capability id");
    let provider_id = ExtensionId::new("evm-rpc").expect("valid provider id");
    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "EvmRpcAction",
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "const": "eth_block_number" },
                    "chain": { "type": ["string", "null"], "default": null }
                },
                "required": ["action"]
            },
            {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "const": "eth_get_balance" },
                    "address": { "type": "string" },
                    "chain": { "type": ["string", "null"], "default": null }
                },
                "required": ["action", "address"]
            }
        ]
    });
    let arguments = serde_json::json!({
        "action": "eth_get_balance",
        "address": "0x0000000000000000000000000000000000000000",
        "chain": "ethereum"
    });
    let mut capability = visible_capability(capability_id.clone(), provider_id.clone());
    capability.descriptor.parameters_schema = schema.clone();
    let runtime = Arc::new(RecordingHostRuntime::new(vec![capability]));
    let mut context = execution_context("thread-versioned-union-schema");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        Arc::new(JsonInputResolver(arguments.clone())),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    let definition = port
        .tool_definitions()
        .expect("tool definitions")
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("versioned union schema capability must be advertised");
    assert_eq!(definition.name.as_str(), "evm-rpc__invoke");
    assert_eq!(definition.parameters, schema);

    let mut call = provider_tool_call();
    call.name = definition.name;
    call.arguments = arguments.clone();
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("union-valid provider call should register");
    assert_eq!(candidate.capability_id, capability_id);

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
        .expect("union-valid capability call should dispatch");
    assert!(matches!(&outcome, Resolution::Done(done) if done.verdict.is_success()));
    let requests = runtime.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].3, arguments);
}

#[tokio::test]
async fn provider_runtime_tool_call_schema_failure_is_model_visible() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
    visible.descriptor.parameters_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"]
    });
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let context = execution_context("thread-provider-runtime-schema-validation");
    let run_context = loop_run_context(&context).await;
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let tool_definition = port
        .tool_definitions()
        .expect("tool definitions")
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("runtime capability advertised to provider");

    let mut call = provider_tool_call();
    call.name = tool_definition.name;
    call.arguments = serde_json::json!({});
    port.validate_provider_tool_call(&call)
        .expect("schema-invalid provider calls should stage for model-visible failure");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("schema-invalid provider calls should register");
    assert!(
        candidate
            .input_ref
            .as_str()
            .starts_with("input:provider-tool-")
    );

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("schema-invalid provider calls should produce a capability failure");

    let Resolution::Done(o) = outcome else {
        panic!("expected schema-invalid provider call to fail");
    };
    let ToolVerdict::RecoverableFailure {
        error_kind,
        diagnostic,
    } = &o.verdict
    else {
        panic!("expected schema-invalid provider call to fail");
    };
    assert_eq!(error_kind, &FailureKind::InputEncode);
    assert!(o.summary.as_str().contains("schema validation"));
    let ModelFailureDiagnostic::InvalidInput { issues } = diagnostic else {
        panic!("schema-invalid provider call should include invalid input detail");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path.as_str(), "message");
    assert_eq!(issues[0].code, DispatchInputIssueCode::MissingRequired);
    assert_eq!(
        issues[0].expected.as_ref().map(SafeSummary::as_str),
        Some("required field")
    );
    assert!(
        runtime.take_requests().is_empty(),
        "schema-invalid provider input must not reach the runtime"
    );
    assert!(
        result_writer.records().is_empty(),
        "schema-invalid provider calls should report through the provider error-result path"
    );
}

#[tokio::test]
async fn provider_runtime_tool_call_schema_failure_preserves_type_mismatch_detail() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
    visible.descriptor.parameters_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": { "type": "string" },
            "limit": { "type": "integer" }
        },
        "required": ["message"]
    });
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let context = execution_context("thread-provider-runtime-schema-detail-validation");
    let run_context = loop_run_context(&context).await;
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let tool_definition = port
        .tool_definitions()
        .expect("tool definitions")
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("runtime capability advertised to provider");

    let mut call = provider_tool_call();
    call.name = tool_definition.name;
    call.arguments = serde_json::json!({
        "message": 123
    });
    port.validate_provider_tool_call(&call)
        .expect("schema-invalid provider calls should stage for model-visible failure");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("schema-invalid provider calls should register");

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("schema-invalid provider calls should produce a capability failure");

    let Resolution::Done(o) = outcome else {
        panic!("expected schema-invalid provider call to fail");
    };
    let ToolVerdict::RecoverableFailure {
        error_kind,
        diagnostic,
    } = &o.verdict
    else {
        panic!("expected schema-invalid provider call to fail");
    };
    assert_eq!(error_kind, &FailureKind::InputEncode);
    let ModelFailureDiagnostic::InvalidInput { issues } = diagnostic else {
        panic!("schema-invalid provider call should include invalid input detail");
    };
    assert!(
        issues.as_slice().iter().any(|issue| {
            issue.path.as_str() == "message"
                && issue.code == DispatchInputIssueCode::TypeMismatch
                && issue.expected.as_ref().map(SafeSummary::as_str) == Some("string")
                && issue.received.as_ref().map(SafeSummary::as_str) == Some("integer")
        }),
        "type mismatch issue should identify the mismatched field"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "schema-invalid provider input must not reach the runtime"
    );
    assert!(
        result_writer.records().is_empty(),
        "schema-invalid provider calls should report through the provider error-result path"
    );
}

#[tokio::test]
async fn provider_runtime_tool_call_schema_failure_preserves_unexpected_field_detail() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
    visible.descriptor.parameters_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"]
    });
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let context = execution_context("thread-provider-runtime-unexpected-field-validation");
    let run_context = loop_run_context(&context).await;
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        result_writer.clone(),
        dummy_milestone_sink(),
    )
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let tool_definition = port
        .tool_definitions()
        .expect("tool definitions")
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("runtime capability advertised to provider");

    let mut call = provider_tool_call();
    call.name = tool_definition.name;
    call.arguments = serde_json::json!({
        "message": "hello",
        "unexpected": true
    });
    port.validate_provider_tool_call(&call)
        .expect("schema-invalid provider calls should stage for model-visible failure");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
        .await
        .expect("schema-invalid provider calls should register");

    let outcome = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: surface.version,
            capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("schema-invalid provider calls should produce a capability failure");

    let Resolution::Done(o) = outcome else {
        panic!("expected schema-invalid provider call to fail");
    };
    let ToolVerdict::RecoverableFailure {
        error_kind,
        diagnostic,
    } = &o.verdict
    else {
        panic!("expected schema-invalid provider call to fail");
    };
    assert_eq!(error_kind, &FailureKind::InputEncode);
    let ModelFailureDiagnostic::InvalidInput { issues } = diagnostic else {
        panic!("schema-invalid provider call should include invalid input detail");
    };
    assert!(
        issues.as_slice().iter().any(|issue| {
            issue.path.as_str() == "unexpected"
                && issue.code == DispatchInputIssueCode::UnexpectedField
        }),
        "unexpected field issue should identify the field to remove"
    );
    assert!(
        runtime.take_requests().is_empty(),
        "schema-invalid provider input must not reach the runtime"
    );
    assert!(
        result_writer.records().is_empty(),
        "schema-invalid provider calls should report through the provider error-result path"
    );
}
