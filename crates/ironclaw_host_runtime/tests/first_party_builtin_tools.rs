use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::Arc,
    thread,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, CapabilitySurfacePolicy, CapabilitySurfaceVersion,
    ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HostRuntime,
    HostRuntimeServices, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind, SurfaceKind,
    TIME_CAPABILITY_ID, VisibleCapabilityAccess, VisibleCapabilityRequest,
    WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpResponse, NetworkHttpTransport,
    NetworkResolver, NetworkTransportRequest, NetworkUsage, PolicyNetworkHttpEgress,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn builtin_first_party_package_declares_expected_capabilities() {
    let package = builtin_first_party_package().unwrap();
    assert_eq!(package.id, provider_id());
    assert_eq!(package.manifest.runtime_kind(), RuntimeKind::FirstParty);

    let ids = package
        .capabilities
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, all_builtin_capability_ids().to_vec());

    let handlers = builtin_first_party_handlers().unwrap();
    for id in all_builtin_capability_ids() {
        assert!(handlers.contains_handler(&capability_id(id)));
    }
}

#[tokio::test]
async fn builtin_first_party_surface_lists_allowed_tools_in_registry_order() {
    let runtime = runtime();
    let request = VisibleCapabilityRequest::new(
        execution_context(all_builtin_capability_ids()),
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust());

    let surface = runtime.visible_capabilities(request).await.unwrap();

    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, all_builtin_capability_ids().to_vec());
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.access == VisibleCapabilityAccess::Available)
    );
    assert!(
        surface
            .capabilities
            .iter()
            .all(|capability| capability.estimated_resources.output_bytes.is_some())
    );
}

#[tokio::test]
async fn builtin_first_party_surface_hides_runtime_policy_impossible_tools() {
    let runtime = runtime_with_policy(network_denied_policy());
    let request = VisibleCapabilityRequest::new(
        execution_context(all_builtin_capability_ids()),
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust());

    let surface = runtime.visible_capabilities(request).await.unwrap();

    let ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();
    assert!(!ids.contains(&HTTP_CAPABILITY_ID));
    assert!(ids.contains(&ECHO_CAPABILITY_ID));
}

#[tokio::test]
async fn builtin_rejects_oversized_inputs_before_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "validate", "data": "x".repeat(1_048_577)}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected resource failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_rejects_oversized_outputs_before_return() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"operation": "stringify", "data": {"items": vec!["xxxxxxxx"; 80_000]}}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected output-too-large failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::OutputTooLarge);
}

#[tokio::test]
async fn builtin_echo_invokes_through_host_runtime() {
    let output = invoke(ECHO_CAPABILITY_ID, json!({"message": "hello reborn"}))
        .await
        .unwrap();
    assert_eq!(output, Value::String("hello reborn".to_string()));
}

#[tokio::test]
async fn builtin_time_parse_convert_and_diff_are_deterministic() {
    let parsed = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12T13:00:00Z"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["unix"], json!(1778590800));

    let converted = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "convert",
            "input": "2026-05-12T13:00:00Z",
            "to_timezone": "America/New_York"
        }),
    )
    .await
    .unwrap();
    assert_eq!(converted["output"], json!("2026-05-12T09:00:00-04:00"));

    let diff = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "diff",
            "input": "2026-05-12T13:00:00Z",
            "timestamp2": "2026-05-12T15:30:00Z"
        }),
    )
    .await
    .unwrap();
    assert_eq!(diff["minutes"], json!(150));
}

#[tokio::test]
async fn builtin_time_rejects_naive_without_timezone_and_ambiguous_local_time() {
    let missing_timezone = invoke(
        TIME_CAPABILITY_ID,
        json!({"operation": "parse", "input": "2026-05-12 13:00:00"}),
    )
    .await
    .unwrap_err();
    assert_eq!(missing_timezone, RuntimeFailureKind::InvalidInput);

    let ambiguous = invoke(
        TIME_CAPABILITY_ID,
        json!({
            "operation": "parse",
            "input": "2026-11-01 01:30:00",
            "timezone": "America/New_York"
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(ambiguous, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_parse_query_stringify_and_validate_work() {
    let parsed = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "parse", "data": "{\"items\":[{\"name\":\"alpha\"}]}"}),
    )
    .await
    .unwrap();
    assert_eq!(parsed["items"][0]["name"], json!("alpha"));

    let queried = invoke(
        JSON_CAPABILITY_ID,
        json!({
            "operation": "query",
            "data": {"items":[{"name":"alpha"}]},
            "path": "items[0].name"
        }),
    )
    .await
    .unwrap();
    assert_eq!(queried, json!("alpha"));

    let valid = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "validate", "data": "{\"ok\":true}"}),
    )
    .await
    .unwrap();
    assert_eq!(valid, json!({"valid": true}));

    let stringified = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": {"ok": true}}),
    )
    .await
    .unwrap();
    assert!(stringified.as_str().unwrap().contains("\"ok\": true"));
}

#[tokio::test]
async fn builtin_json_stringify_rejects_invalid_json_strings() {
    let error = invoke(
        JSON_CAPABILITY_ID,
        json!({"operation": "stringify", "data": "not json"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_json_rejects_v1_tool_output_stash_refs_without_leaking_input() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([JSON_CAPABILITY_ID]),
            capability_id(JSON_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({
                "operation": "parse",
                "source_tool_call_id": "call_RAW_SECRET_sk-provider-secret"
            }),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected sanitized failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    let debug = format!("{failure:?}");
    assert!(!debug.contains("RAW_SECRET"));
    assert!(!debug.contains("sk-provider-secret"));
}

#[tokio::test]
async fn builtin_http_invokes_through_host_runtime_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"accepted":true}"#.to_vec(),
    ));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "method": "post",
            "url": "https://api.example.test/v1/items",
            "headers": {
                "content-type": "application/json",
                "x-request-id": "first-party-http"
            },
            "body": {"ok": true},
            "response_body_limit": 4096,
            "timeout_ms": 2500
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_or_else(|error| {
        panic!(
            "expected HTTP egress success, got {error:?}; recorded requests: {:?}",
            egress.requests()
        )
    });

    assert_eq!(output["status"], json!(200));
    assert_eq!(output["body_text"], json!(r#"{"accepted":true}"#));
    assert_eq!(output["request_bytes"], json!(11));
    assert_eq!(output["response_bytes"], json!(17));

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.runtime, RuntimeKind::FirstParty);
    assert_eq!(request.capability_id, capability_id(HTTP_CAPABILITY_ID));
    assert_eq!(request.method, NetworkMethod::Post);
    assert_eq!(request.url, "https://api.example.test/v1/items");
    assert_eq!(request.body, br#"{"ok":true}"#);
    assert_eq!(request.response_body_limit, Some(4096));
    assert_eq!(request.timeout_ms, Some(2500));
    assert!(request.credential_injections.is_empty());
}

#[tokio::test]
async fn builtin_http_runtime_policy_denial_stops_before_egress() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"ok":true}"#.to_vec(),
    ));
    let runtime = runtime_with_http_egress_and_policy(Arc::clone(&egress), network_denied_policy());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
            capability_id(HTTP_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"url": "https://api.example.test/v1/items"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected runtime-policy failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert_eq!(failure.capability_id, capability_id(HTTP_CAPABILITY_ID));
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("NetworkMode::Deny")
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_defaults_json_body_content_type() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(b"ok".to_vec()));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "body": {"ok": true}
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    let requests = egress.requests();
    assert_eq!(
        requests[0].headers,
        vec![("content-type".to_string(), "application/json".to_string())]
    );
}

#[tokio::test]
async fn builtin_http_fails_closed_without_runtime_egress() {
    let runtime = runtime();
    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Network);
}

#[tokio::test]
async fn builtin_http_rejects_ambiguous_body_zero_timeout_and_zero_response_limit() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(egress);
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "method": 123
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "body": "plain",
            "body_base64": "YmluYXJ5"
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "timeout_ms": 0
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "response_body_limit": 0
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_http_rejects_request_bodies_over_network_egress_cap() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(Arc::clone(&egress));
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let oversized = vec![b'a'; 256 * 1024 + 1];

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "body": String::from_utf8(oversized.clone()).unwrap()
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "body_base64": BASE64_STANDARD.encode(&oversized)
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }

    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn builtin_http_accounts_request_bytes_when_output_is_too_large() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![
        b'\\';
        700 * 1024
    ]));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let runtime = runtime_with_http_egress_and_governor(Arc::clone(&egress), Arc::clone(&governor));

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "method": "post",
            "url": "https://api.example.test/v1/items",
            "body": "paid",
            "response_body_limit": 700 * 1024
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::OutputTooLarge);
    let tenant_account = ResourceAccount::tenant(TenantId::new(LOCAL_DEFAULT_TENANT_ID).unwrap());
    assert_eq!(governor.usage_for(&tenant_account).network_egress_bytes, 4);
}

#[tokio::test]
async fn builtin_http_returns_binary_responses_as_base64() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(vec![0, 159, 255]));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["body_base64"], json!("AJ//"));
    assert!(output.get("body_text").is_none());
}

#[tokio::test]
async fn builtin_http_documents_mixed_utf8_response_as_binary() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        b"hello\xFFworld".to_vec(),
    ));
    let runtime = runtime_with_http_egress(Arc::clone(&egress));

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    // Invalid UTF-8 makes the whole response binary so callers never receive
    // split or lossy text/body fragments.
    assert_eq!(output["body_base64"], json!("aGVsbG//d29ybGQ="));
    assert!(output.get("body_text").is_none());
}

#[tokio::test]
async fn builtin_http_rejects_invalid_header_names_and_oversized_header_sets() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let runtime = runtime_with_http_egress(egress);
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let too_many_headers = (0..65)
        .map(|index| json!({"name": format!("x-{index}"), "value": "ok"}))
        .collect::<Vec<_>>();

    for input in [
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"x-bad": "value\0tail"}
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x\0bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x:bad", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": [{"name": "x-é", "value": "ok"}]
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": too_many_headers
        }),
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"x-large": "a".repeat(8 * 1024 + 1)}
        }),
    ] {
        let error = invoke_with_context(&runtime, HTTP_CAPABILITY_ID, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::InvalidInput);
    }
}

#[tokio::test]
async fn builtin_http_maps_runtime_egress_errors_by_source() {
    let context = execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy());
    let cases = [
        (
            RuntimeHttpEgressError::Credential {
                reason: "credential missing".to_string(),
            },
            RuntimeFailureKind::Backend,
        ),
        (
            RuntimeHttpEgressError::Request {
                reason: "sensitive_header_denied:authorization".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::InvalidInput,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "policy_denied".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
            RuntimeFailureKind::Network,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "response_body_limit_exceeded".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_body_limit_exceeded".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "response_decode_failed".to_string(),
                request_bytes: 4,
                response_bytes: 1024,
            },
            RuntimeFailureKind::InvalidInput,
        ),
    ];

    for (error, expected) in cases {
        let runtime =
            runtime_with_http_egress(Arc::new(RecordingRuntimeHttpEgress::with_error(error)));
        let actual = invoke_with_context(
            &runtime,
            HTTP_CAPABILITY_ID,
            json!({"url": "https://api.example.test/v1/items"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn builtin_http_rejects_sensitive_headers_through_host_validator() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({
            "url": "https://api.example.test/v1/items",
            "headers": {"authorization": "Bearer token"}
        }),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::InvalidInput);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn builtin_http_exercises_real_policy_private_ip_rejection() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let error = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Network);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn builtin_http_returns_redirects_without_following_private_location() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 302,
        headers: vec![(
            "location".to_string(),
            "http://169.254.169.254/latest/meta-data".to_string(),
        )],
        body: Vec::new(),
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let network = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );
    let runtime = runtime_with_host_http_egress(network);

    let output = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    )
    .await
    .unwrap();

    assert_eq!(output["status"], json!(302));
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, "https://api.example.test/v1/items");
}

#[tokio::test]
async fn builtin_http_runs_blocking_egress_off_tokio_worker() {
    let egress = Arc::new(SleepingRuntimeHttpEgress {
        delay: Duration::from_millis(100),
    });
    let runtime = runtime_with_http_egress(egress);
    let invocation = invoke_with_context(
        &runtime,
        HTTP_CAPABILITY_ID,
        json!({"url": "https://api.example.test/v1/items"}),
        execution_context_with_network([HTTP_CAPABILITY_ID], http_test_policy()),
    );
    tokio::pin!(invocation);

    tokio::select! {
        _ = &mut invocation => panic!("HTTP dispatch blocked the tokio worker"),
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
    }

    invocation.await.unwrap();
}

#[tokio::test]
async fn builtin_coding_tools_match_v1_read_write_list_glob_and_grep_shapes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\nbeta\n").unwrap();
    std::fs::write(
        temp.path().join("src/lib.rs"),
        "pub fn sample() {\n    // needle\n}\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts.clone());

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md", "offset": 2, "limit": 1}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     2│ beta"));
    assert_eq!(read["total_lines"], json!(2));
    assert_eq!(read["lines_shown"], json!(1));
    assert_eq!(read["truncated_by_default"], json!(false));

    let wrote = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/generated/deep.txt", "content": "created\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(wrote["bytes_written"], json!(8));
    assert_eq!(wrote["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("generated/deep.txt")).unwrap(),
        "created\n"
    );

    let wrote_subdir_readme = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/generated/README.md", "content": "ok\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(wrote_subdir_readme["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("generated/README.md")).unwrap(),
        "ok\n"
    );

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace", "recursive": true, "max_depth": 1}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["count"], json!(6));
    assert_eq!(listed["truncated"], json!(false));
    assert_eq!(
        listed["entries"],
        json!([
            "generated/",
            "src/",
            "README.md (11B)",
            "generated/README.md (3B)",
            "generated/deep.txt (8B)",
            "src/lib.rs (34B)"
        ])
    );

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "**/*.rs", "max_results": 5}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["src/lib.rs"]));
    assert_eq!(globbed["count"], json!(1));
    assert_eq!(globbed["truncated"], json!(false));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "NEEDLE", "case_insensitive": true}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["src/lib.rs"]));
    assert_eq!(grepped["count"], json!(1));
    assert_eq!(grepped["truncated"], json!(false));
}

#[tokio::test]
async fn builtin_coding_paths_are_relative_to_requested_root_and_zero_values_match_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src/nested")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();
    std::fs::write(temp.path().join("src/nested/mod.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/src/lib.rs", "offset": 0, "limit": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!(""));
    assert_eq!(read["lines_shown"], json!(0));

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace/src", "recursive": true, "max_depth": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["entries"], json!(["nested/", "lib.rs (7B)"]));

    let globbed_empty_page = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "*.rs", "max_results": 0}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed_empty_page["files"], json!([]));
    assert_eq!(globbed_empty_page["count"], json!(0));
    assert_eq!(globbed_empty_page["truncated"], json!(true));

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["lib.rs"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/src", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    let grep_files = grepped["files"].as_array().unwrap();
    assert_eq!(grep_files.len(), 2);
    assert!(
        grep_files
            .iter()
            .any(|file| file.as_str() == Some("lib.rs"))
    );
    assert!(
        grep_files
            .iter()
            .any(|file| file.as_str() == Some("nested/mod.rs"))
    );
}

#[tokio::test]
async fn builtin_coding_glob_and_grep_files_with_matches_sort_newest_first() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old.rs"), "needle old\n").unwrap();
    thread::sleep(Duration::from_millis(1_100));
    std::fs::write(temp.path().join("new.rs"), "needle new\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["new.rs", "old.rs"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["new.rs", "old.rs"]));
}

#[tokio::test]
async fn builtin_coding_blocks_sensitive_scoped_paths_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".env"), "TOKEN=secret\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    for (capability, input) in [
        (READ_FILE_CAPABILITY_ID, json!({"path": "/workspace/.env"})),
        (
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": "/workspace/.env", "content": "changed"}),
        ),
        (
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/.env", "old_string": "TOKEN", "new_string": "X"}),
        ),
    ] {
        let error = invoke_with_context(&runtime, capability, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        context,
    )
    .await
    .unwrap();
    let entries = listed["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| entry.as_str() != Some(".env (13B)"))
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn builtin_coding_blocks_sensitive_resolved_libsql_paths() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("filesystem.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = LibSqlRootFilesystem::new(db);
    filesystem.run_migrations().await.unwrap();
    filesystem
        .create_dir_all(&VirtualPath::new("/projects/p").unwrap())
        .await
        .unwrap();
    filesystem
        .write_file(
            &VirtualPath::new("/projects/p/.env").unwrap(),
            b"TOKEN=secret\n",
        )
        .await
        .unwrap();

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/p/.env").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    for (capability, input) in [
        (READ_FILE_CAPABILITY_ID, json!({"path": "/workspace"})),
        (
            GREP_CAPABILITY_ID,
            json!({"path": "/workspace", "pattern": "TOKEN"}),
        ),
    ] {
        let error = invoke_with_context(&runtime, capability, input, context.clone())
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn builtin_coding_apply_patch_serializes_concurrent_edits_on_same_path() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "A\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = std::sync::Arc::new(runtime_with_filesystem(filesystem));
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    // Prime read-before-edit state so both patches start from a valid cached hash.
    invoke_with_context(
        &*runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let runtime_a = runtime.clone();
    let ctx_a = context.clone();
    let runtime_b = runtime.clone();
    let ctx_b = context.clone();
    let task_a = tokio::spawn(async move {
        invoke_with_context(
            &*runtime_a,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/code.rs", "old_string": "A", "new_string": "X"}),
            ctx_a,
        )
        .await
    });
    let task_b = tokio::spawn(async move {
        invoke_with_context(
            &*runtime_b,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": "/workspace/code.rs", "old_string": "A", "new_string": "Y"}),
            ctx_b,
        )
        .await
    });
    let result_a = task_a.await.unwrap();
    let result_b = task_b.await.unwrap();

    // Serialization guarantee: the second patch reads the post-write file and
    // its cached hash (taken before either write) no longer matches, so
    // exactly one apply_patch must succeed. Without the per-path edit lock,
    // both calls can pass `check_before_edit` concurrently and silently lose
    // an update.
    let outcomes = [result_a.is_ok(), result_b.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "expected exactly one concurrent apply_patch to succeed, got {:?}",
        outcomes
    );
    let final_content = std::fs::read_to_string(temp.path().join("code.rs")).unwrap();
    assert!(
        final_content == "X\n" || final_content == "Y\n",
        "unexpected final content {final_content:?}"
    );
}

#[tokio::test]
async fn builtin_coding_blocks_relative_workspace_protected_paths() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("daily")).unwrap();
    std::fs::create_dir_all(temp.path().join("context")).unwrap();
    std::fs::write(temp.path().join("README.md"), "keep\n").unwrap();
    std::fs::write(temp.path().join("daily/note.md"), "keep\n").unwrap();
    std::fs::write(temp.path().join("context/session.md"), "keep\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let root_readme_write = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "./README.md", "content": "changed\n"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(root_readme_write["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "changed\n"
    );

    for (tool_path, host_path) in [
        ("./daily/note.md", "daily/note.md"),
        ("./context/session.md", "context/session.md"),
    ] {
        let write_error = invoke_with_context(
            &runtime,
            WRITE_FILE_CAPABILITY_ID,
            json!({"path": tool_path, "content": "changed\n"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(write_error, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(host_path)).unwrap(),
            "keep\n"
        );

        invoke_with_context(
            &runtime,
            READ_FILE_CAPABILITY_ID,
            json!({"path": tool_path}),
            context.clone(),
        )
        .await
        .unwrap();

        let patch_error = invoke_with_context(
            &runtime,
            APPLY_PATCH_CAPABILITY_ID,
            json!({"path": tool_path, "old_string": "keep", "new_string": "changed"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(patch_error, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(host_path)).unwrap(),
            "keep\n"
        );
    }
}

#[tokio::test]
async fn builtin_coding_grep_searches_single_file_paths_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/src/lib.rs", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["lib.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_applies_filters_before_loading_large_files() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "needle\n").unwrap();
    std::fs::write(
        temp.path().join("huge.txt"),
        vec![b'x'; 10 * 1024 * 1024 + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle", "glob": "**/*.rs"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["src/lib.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_multiline_reports_matched_lines_and_count() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("doc.txt"), "alpha\nbeta\ngamma\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let content = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "alpha\\nbeta",
            "output_mode": "content",
            "multiline": true
        }),
        context.clone(),
    )
    .await
    .unwrap();
    let output = content["content"].as_str().unwrap();
    assert!(output.contains("doc.txt:1:alpha"));
    assert!(output.contains("doc.txt:2:beta"));

    let counts = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({
            "path": "/workspace",
            "pattern": "alpha\\nbeta",
            "output_mode": "count",
            "multiline": true
        }),
        context,
    )
    .await
    .unwrap();
    assert_eq!(counts["total"], json!(1));
    assert_eq!(counts["counts"], json!([{ "file": "doc.txt", "count": 1 }]));
}

#[tokio::test]
async fn builtin_coding_write_allows_v1_sized_payloads_past_default_input_cap() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);
    let content = "x".repeat(2 * 1024 * 1024);

    let wrote = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/large.txt", "content": content}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(wrote["bytes_written"], json!(2 * 1024 * 1024));
}

#[cfg(unix)]
#[tokio::test]
async fn builtin_coding_grep_skips_sensitive_files_and_symlink_targets() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".env"), "TOKEN=secret\n").unwrap();
    std::os::unix::fs::symlink(temp.path().join(".env"), temp.path().join("safe_link.rs")).unwrap();
    std::fs::write(temp.path().join("visible.rs"), "TOKEN=visible\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "TOKEN", "output_mode": "content"}),
        context.clone(),
    )
    .await
    .unwrap();
    let content = grepped["content"].as_str().unwrap();
    assert!(content.contains("visible.rs"));
    assert!(!content.contains("secret"));
    assert!(!content.contains("safe_link"));

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        context,
    )
    .await
    .unwrap();
    let entries = listed["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| !entry.as_str().unwrap_or_default().contains(".env"))
    );
    assert!(
        entries
            .iter()
            .all(|entry| !entry.as_str().unwrap_or_default().contains("safe_link"))
    );
}

#[tokio::test]
async fn builtin_coding_grep_requires_read_permission_but_glob_only_requires_list() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(
        temp.path().join("src/secret.rs"),
        "let token = \"secret\";\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: false,
            write: false,
            delete: false,
            list: true,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let globbed = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "**/*.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(globbed["files"], json!(["src/secret.rs"]));

    let error = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "secret"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(error, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn builtin_coding_list_and_glob_require_list_permission() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(
        temp.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/README.md"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["content"], json!("     1│ alpha"));

    for capability in [LIST_DIR_CAPABILITY_ID, GLOB_CAPABILITY_ID] {
        let error = invoke_with_context(
            &runtime,
            capability,
            json!({"path": "/workspace", "pattern": "**/*.md"}),
            context.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, RuntimeFailureKind::Authorization);
    }
}

#[tokio::test]
async fn builtin_coding_list_truncates_like_v1_after_500_entries() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..501 {
        std::fs::write(temp.path().join(format!("file-{index:03}.txt")), "a").unwrap();
    }

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap();

    assert_eq!(listed["count"], json!(500));
    assert_eq!(listed["truncated"], json!(true));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 500);
}

#[tokio::test]
async fn builtin_coding_read_rejects_files_larger_than_v1_limit_before_loading_contents() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("big.txt"),
        vec![b'x'; 10 * 1024 * 1024 + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/big.txt"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_apply_patch_matches_v1_exact_unique_and_replace_all_behavior() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\nold\nunique\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let duplicate = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(duplicate, RuntimeFailureKind::Backend);

    let replaced = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/code.rs",
            "old_string": "old",
            "new_string": "new",
            "replace_all": true
        }),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(replaced["success"], json!(true));
    assert_eq!(replaced["replacements"], json!(2));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("code.rs")).unwrap(),
        "new\nnew\nunique\n"
    );

    let no_op = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "new", "new_string": "new"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(no_op, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_apply_patch_requires_full_fresh_read_like_v1() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    let unread = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(unread, RuntimeFailureKind::Backend);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "limit": 1}),
        context.clone(),
    )
    .await
    .unwrap();
    let partial = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(partial, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_apply_patch_rejects_same_second_content_changes_after_read() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(all_builtin_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs"}),
        context.clone(),
    )
    .await
    .unwrap();
    std::fs::write(temp.path().join("code.rs"), "old\nchanged\n").unwrap();

    let stale = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/code.rs", "old_string": "old", "new_string": "new"}),
        context,
    )
    .await
    .unwrap_err();
    assert_eq!(stale, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_coding_write_is_denied_by_read_only_mount() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("README.md"), "alpha\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let error = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/blocked.txt", "content": "nope"}),
        execution_context_with_mounts(all_builtin_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Authorization);
    assert!(!temp.path().join("blocked.txt").exists());
}

#[tokio::test]
async fn builtin_missing_grant_denies_before_handler_dispatch() {
    let outcome = runtime()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context([]),
            capability_id(ECHO_CAPABILITY_ID),
            ResourceEstimate::default(),
            json!({"message":"must not run"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected authorization failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
}

async fn invoke(capability: &str, input: Value) -> Result<Value, RuntimeFailureKind> {
    let runtime = runtime();
    invoke_with_context(&runtime, capability, input, execution_context([capability])).await
}

async fn invoke_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

fn runtime() -> impl HostRuntime {
    runtime_with_filesystem(LocalFilesystem::new())
}

fn runtime_with_filesystem<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_policy(policy: EffectiveRuntimePolicy) -> impl HostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .with_trust_policy(Arc::new(trust_policy()))
    .with_runtime_policy(policy)
    .host_runtime_for_local_testing()
}

fn runtime_with_http_egress<T>(egress: Arc<T>) -> impl HostRuntime
where
    T: RuntimeHttpEgress + 'static,
{
    runtime_with_http_egress_and_governor(egress, Arc::new(InMemoryResourceGovernor::new()))
}

fn runtime_with_http_egress_and_policy<T>(
    egress: Arc<T>,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    T: RuntimeHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .with_runtime_http_egress(egress)
    .with_trust_policy(Arc::new(trust_policy()))
    .with_runtime_policy(policy)
    .host_runtime_for_local_testing()
}

fn runtime_with_http_egress_and_governor<T>(
    egress: Arc<T>,
    governor: Arc<InMemoryResourceGovernor>,
) -> impl HostRuntime
where
    T: RuntimeHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        governor,
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .with_runtime_http_egress(egress)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_host_http_egress<N>(network: N) -> impl HostRuntime
where
    N: NetworkHttpEgress + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers().unwrap()))
    .try_with_host_http_egress(network)
    .unwrap()
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("builtin").unwrap()
}

fn all_builtin_capability_ids() -> [&'static str; 10] {
    [
        ECHO_CAPABILITY_ID,
        TIME_CAPABILITY_ID,
        JSON_CAPABILITY_ID,
        HTTP_CAPABILITY_ID,
        READ_FILE_CAPABILITY_ID,
        WRITE_FILE_CAPABILITY_ID,
        LIST_DIR_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
        APPLY_PATCH_CAPABILITY_ID,
    ]
}

fn mounted_filesystem(path: &Path, permissions: MountPermissions) -> (LocalFilesystem, MountView) {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/coding-pack").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/coding-pack").unwrap(),
        permissions,
    )])
    .unwrap();
    (filesystem, mounts)
}

#[derive(Debug, Clone, Default)]
struct RecordingRuntimeHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>>,
    body: Vec<u8>,
    error: Option<RuntimeHttpEgressError>,
}

impl RecordingRuntimeHttpEgress {
    fn with_body(body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            body,
            error: None,
        }
    }

    fn with_error(error: RuntimeHttpEgressError) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            body: Vec::new(),
            error: Some(error),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: self.body.clone(),
            request_bytes: request.body.len() as u64,
            response_bytes: self.body.len() as u64,
            redaction_applied: false,
        })
    }
}

#[derive(Debug, Clone)]
struct SleepingRuntimeHttpEgress {
    delay: Duration,
}

impl RuntimeHttpEgress for SleepingRuntimeHttpEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        thread::sleep(self.delay);
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: b"ok".to_vec(),
            request_bytes: request.body.len() as u64,
            response_bytes: 2,
            redaction_applied: false,
        })
    }
}

#[derive(Debug, Clone)]
struct RecordingTransport {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<std::sync::Mutex<Vec<NetworkTransportRequest>>>,
}

impl RecordingTransport {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl NetworkHttpTransport for RecordingTransport {
    fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

#[derive(Debug, Clone)]
struct StaticResolver {
    ips: Vec<IpAddr>,
}

impl StaticResolver {
    fn new(ips: Vec<IpAddr>) -> Self {
        Self { ips }
    }
}

impl NetworkResolver for StaticResolver {
    fn resolve_ips(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        Ok(self.ips.clone())
    }
}

fn execution_context<const N: usize>(grants: [&str; N]) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants.into_iter().map(dispatch_grant).collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

fn execution_context_with_mounts<const N: usize>(
    grants: [&str; N],
    mounts: MountView,
) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant_with_mounts(grant, mounts.clone()))
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap()
}

fn execution_context_with_network<const N: usize>(
    grants: [&str; N],
    network: NetworkPolicy,
) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant_with_network(grant, network.clone()))
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

fn dispatch_grant(capability: &str) -> CapabilityGrant {
    dispatch_grant_with_mounts(capability, MountView::default())
}

fn dispatch_grant_with_mounts(capability: &str, mounts: MountView) -> CapabilityGrant {
    dispatch_grant_with_mounts_and_network(capability, mounts, NetworkPolicy::default())
}

fn dispatch_grant_with_network(capability: &str, network: NetworkPolicy) -> CapabilityGrant {
    dispatch_grant_with_mounts_and_network(capability, MountView::default(), network)
}

fn dispatch_grant_with_mounts_and_network(
    capability: &str,
    mounts: MountView,
    network: NetworkPolicy,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts,
            network,
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::Network,
    ]
}

fn network_denied_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn http_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn provider_trust() -> BTreeMap<ExtensionId, TrustDecision> {
    BTreeMap::from([(provider_id(), trust_decision())])
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: builtin_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
