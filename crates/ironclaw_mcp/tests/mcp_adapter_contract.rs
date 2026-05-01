use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_extensions::*;
use ironclaw_host_api::*;
use ironclaw_mcp::*;
use ironclaw_resources::*;
use serde_json::json;

#[tokio::test]
async fn mcp_runtime_reserves_calls_adapter_and_reconciles_success() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput::json(json!({
        "items": ["issue-1"]
    }))));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(1),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let result = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    process_count: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation {
                    input: json!({"query": "ironclaw"}),
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(result.result.output, json!({"items": ["issue-1"]}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
    assert_eq!(governor.usage_for(&account).process_count, 0);

    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].provider,
        ExtensionId::new("github-mcp").unwrap()
    );
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("github-mcp.search").unwrap()
    );
    assert_eq!(requests[0].transport, "http");
    assert_eq!(requests[0].command, None);
    assert!(requests[0].args.is_empty());
    assert_eq!(
        requests[0].url.as_deref(),
        Some("https://mcp.example.test/mcp")
    );
    assert_eq!(requests[0].input, json!({"query": "ironclaw"}));
    assert_eq!(
        requests[0].max_output_bytes,
        McpRuntimeConfig::for_testing().max_output_bytes
    );
}

#[tokio::test]
async fn mcp_runtime_requires_host_mediated_egress_for_http_transports() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::direct_network(Ok(McpClientOutput::json(json!({
        "items": ["issue-1"]
    }))));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope(),
                estimate: ResourceEstimate::default(),
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::HostHttpEgressRequired { .. }));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[test]
fn mcp_host_http_adapter_returns_sanitized_shared_egress_errors() {
    let adapter = McpRuntimeHttpAdapter::new(Arc::new(SecretEchoRuntimeEgress));

    let error = adapter
        .request(McpHostHttpRequest {
            scope: sample_scope(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            method: NetworkMethod::Get,
            url: "https://mcp.example.test/mcp".to_string(),
            headers: vec![],
            body: vec![],
            network_policy: mcp_http_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        })
        .expect_err("MCP HTTP adapter errors should be sanitized before runtime visibility");

    let rendered = error.to_string();
    assert!(rendered.contains("network_error"));
    assert!(!rendered.contains("sk-test-secret"));
    assert!(!rendered.contains("10.0.0.7"));
}

#[tokio::test]
async fn concrete_mcp_http_client_routes_json_rpc_through_shared_egress() {
    let scope = sample_scope();
    let plan = host_http_plan();
    let egress = RecordingRuntimeEgress::json_rpc();
    let planner = RecordingEgressPlanner::new(plan.clone());
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        planner.clone(),
    );

    assert!(client.uses_host_mediated_http_egress());

    let output = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: scope.clone(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({
                "query": "ironclaw",
                "credential_injections": [{"handle": "evil-token"}]
            }),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    assert_eq!(
        output.output,
        json!({"content":[{"type":"text","text":"ok"}],"isError":false})
    );

    let requests = egress.requests();
    assert_eq!(
        requests.len(),
        3,
        "initialize, initialized notification, tools/call"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.runtime == RuntimeKind::Mcp)
    );
    assert!(requests.iter().all(|request| request.scope == scope));
    assert!(
        requests
            .iter()
            .all(|request| request.network_policy == plan.network_policy)
    );
    assert!(
        requests
            .iter()
            .all(|request| request.credential_injections == plan.credential_injections)
    );
    assert!(
        requests
            .iter()
            .all(|request| request.response_body_limit == Some(4096))
    );
    assert!(
        requests
            .iter()
            .all(|request| request.timeout_ms == Some(2_500))
    );
    assert_eq!(json_rpc_method(&requests[0].body), "initialize");
    assert_eq!(
        json_rpc_method(&requests[1].body),
        "notifications/initialized"
    );
    assert_eq!(json_rpc_method(&requests[2].body), "tools/call");
    assert_eq!(json_rpc_param(&requests[2].body, "name"), json!("search"));
    assert_eq!(
        json_rpc_param(&requests[2].body, "arguments"),
        json!({"query":"ironclaw","credential_injections":[{"handle":"evil-token"}]})
    );
    assert!(
        requests[2]
            .headers
            .iter()
            .any(|(name, value)| name == "Mcp-Session-Id" && value == "session-123")
    );
    assert!(requests.iter().all(|request| {
        !request
            .credential_injections
            .iter()
            .any(|injection| injection.handle.as_str() == "evil-token")
    }));

    let planner_calls = planner.calls();
    assert_eq!(planner_calls.len(), 3);
    assert!(planner_calls.iter().all(|call| call.scope == scope));
    assert!(
        planner_calls
            .iter()
            .all(|call| call.url == "https://mcp.example.test/mcp")
    );
}

#[tokio::test]
async fn concrete_mcp_http_client_scopes_session_ids_per_invocation() {
    let egress = ScopedSessionRuntimeEgress::new();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    for user in ["user1", "user2"] {
        client
            .call_tool(McpClientRequest {
                provider: ExtensionId::new("github-mcp").unwrap(),
                capability_id: CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope_for_user(user),
                transport: "http".to_string(),
                command: None,
                args: vec![],
                url: Some("https://mcp.example.test/mcp".to_string()),
                input: json!({"query": user}),
                max_output_bytes: 4096,
            })
            .await
            .unwrap();
    }

    let requests = egress.requests();
    let user2_requests = requests
        .iter()
        .filter(|request| request.scope.user_id.as_str() == "user2")
        .collect::<Vec<_>>();
    assert_eq!(user2_requests.len(), 3);
    assert!(user2_requests.iter().all(|request| {
        !request
            .headers
            .iter()
            .any(|(_, value)| value == "session-user1")
    }));
    assert!(
        user2_requests
            .iter()
            .filter(|request| json_rpc_method(&request.body) == "tools/call")
            .all(|request| request
                .headers
                .iter()
                .any(|(name, value)| name == "Mcp-Session-Id" && value == "session-user2"))
    );
}

#[tokio::test]
async fn concrete_mcp_http_client_clears_session_ids_between_calls() {
    let egress = ScopedSessionRuntimeEgress::new();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );
    let scope = sample_scope();

    for query in ["first", "second"] {
        client
            .call_tool(McpClientRequest {
                provider: ExtensionId::new("github-mcp").unwrap(),
                capability_id: CapabilityId::new("github-mcp.search").unwrap(),
                scope: scope.clone(),
                transport: "http".to_string(),
                command: None,
                args: vec![],
                url: Some("https://mcp.example.test/mcp".to_string()),
                input: json!({"query": query}),
                max_output_bytes: 4096,
            })
            .await
            .unwrap();
    }

    let requests = egress.requests();
    let initialize_requests = requests
        .iter()
        .filter(|request| json_rpc_method(&request.body) == "initialize")
        .collect::<Vec<_>>();
    assert_eq!(initialize_requests.len(), 2);
    assert!(initialize_requests.iter().all(|request| {
        !request
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("Mcp-Session-Id"))
    }));
}

#[tokio::test]
async fn concrete_mcp_http_client_does_not_reuse_session_from_failed_initialize() {
    let egress = ErrorSessionRuntimeEgress::new();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );
    let scope = sample_scope();

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: scope.clone(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "first"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("failed initialize responses must fail the call");
    assert_eq!(error, "response_error");

    client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "second"}),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    let requests = egress.requests();
    let initialize_requests = requests
        .iter()
        .filter(|request| json_rpc_method(&request.body) == "initialize")
        .collect::<Vec<_>>();
    assert_eq!(initialize_requests.len(), 2);
    assert!(initialize_requests.iter().all(|request| {
        !request.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("Mcp-Session-Id") && value == "session-from-error"
        })
    }));
}

#[tokio::test]
async fn concrete_mcp_http_client_rejects_json_rpc_response_without_matching_id() {
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(MissingIdRuntimeEgress)),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("ID-bearing JSON-RPC requests must reject missing response ids");

    assert_eq!(error, "response_error");
}

#[tokio::test]
async fn mcp_runtime_with_concrete_http_client_consumes_shared_egress_end_to_end() {
    let package = package_from_manifest(MCP_MANIFEST);
    let egress = RecordingRuntimeEgress::json_rpc();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client);
    let governor = InMemoryResourceGovernor::new();

    let result = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope(),
                estimate: ResourceEstimate::default(),
                resource_reservation: None,
                invocation: McpInvocation {
                    input: json!({"query": "ironclaw"}),
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        result.result.output,
        json!({"content":[{"type":"text","text":"ok"}],"isError":false})
    );
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    let requests = egress.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.runtime == RuntimeKind::Mcp)
    );
}

#[tokio::test]
async fn concrete_mcp_sse_client_parses_event_stream_through_shared_egress() {
    let egress = RecordingRuntimeEgress::sse();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let output = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "sse".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/sse".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    assert_eq!(
        output.output,
        json!({"content":[{"type":"text","text":"ok from sse"}],"isError":false})
    );
    assert_eq!(egress.requests().len(), 3);
}

#[tokio::test]
async fn concrete_mcp_http_client_caps_missing_plan_limit_to_client_output_limit() {
    let mut plan = host_http_plan();
    plan.response_body_limit = None;
    let egress = RecordingRuntimeEgress::json_rpc();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(plan),
    );

    client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 1_234,
        })
        .await
        .unwrap();

    assert!(
        egress
            .requests()
            .iter()
            .all(|request| request.response_body_limit == Some(1_234))
    );
}

#[tokio::test]
async fn concrete_mcp_http_client_rejects_invalid_session_id_before_reuse() {
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(InvalidSessionRuntimeEgress)),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("invalid upstream session ids must not be reused as request headers");

    assert_eq!(error, "response_error");
}

#[tokio::test]
async fn concrete_mcp_http_client_sanitizes_shared_egress_failures() {
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(SecretEchoRuntimeEgress)),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("raw shared-egress errors must not leak through the MCP client");

    assert_eq!(error, "network_error");
    assert!(!error.contains("sk-test-secret"));
    assert!(!error.contains("10.0.0.7"));
}

#[tokio::test]
async fn mcp_runtime_fails_closed_for_external_stdio_process_egress() {
    let package = package_from_manifest(STDIO_MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput::json(json!({"ok": true}))));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope(),
                estimate: ResourceEstimate::default(),
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::ExternalStdioTransportUnsupported));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_runtime_denies_budget_before_adapter_call() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput::json(json!({"ok": true}))));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(0),
            ..ResourceLimits::default()
        },
    );

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::Resource(_)));
    assert!(client.requests.lock().unwrap().is_empty());
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_runtime_releases_reservation_when_adapter_fails() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Err("server disconnected".to_string()));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::Client { .. }));
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_runtime_preserves_adapter_error_when_release_cleanup_fails() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Err("server disconnected".to_string()));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client);
    let governor = ReleaseFailingGovernor::new();

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope(),
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::Client { .. }));
}

#[tokio::test]
async fn mcp_runtime_rejects_non_mcp_or_undeclared_capability_before_reserving() {
    let non_mcp = package_from_manifest(SCRIPT_MANIFEST);
    let mcp = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput::json(json!({"ok": true}))));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(0),
            ..ResourceLimits::default()
        },
    );

    let non_mcp_err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &non_mcp,
                capability_id: &CapabilityId::new("script.echo").unwrap(),
                scope: scope.clone(),
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        non_mcp_err,
        McpError::ExtensionRuntimeMismatch {
            actual: RuntimeKind::Script,
            ..
        }
    ));

    let missing_err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &mcp,
                capability_id: &CapabilityId::new("github-mcp.missing").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing_err,
        McpError::CapabilityNotDeclared { .. }
    ));
    assert!(client.requests.lock().unwrap().is_empty());
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_runtime_enforces_output_limit_and_releases_reservation() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput::json(json!({
        "large": "this output is intentionally too large"
    }))));
    let runtime = McpRuntime::new(
        McpRuntimeConfig {
            max_output_bytes: 8,
        },
        client,
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::OutputLimitExceeded { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_runtime_can_enforce_client_reported_output_size_without_serializing_for_size() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"small": true}),
        usage: ResourceUsage::default(),
        output_bytes: Some(1_000),
    }));
    let runtime = McpRuntime::new(
        McpRuntimeConfig {
            max_output_bytes: 8,
        },
        client,
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        McpError::OutputLimitExceeded { actual: 1_000, .. }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_runtime_rejects_output_when_adapter_under_reports_size() {
    let package = package_from_manifest(MCP_MANIFEST);
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"large": "this output exceeds the configured limit"}),
        usage: ResourceUsage::default(),
        output_bytes: Some(1),
    }));
    let runtime = McpRuntime::new(
        McpRuntimeConfig {
            max_output_bytes: 8,
        },
        client,
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: McpInvocation { input: json!({}) },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::OutputLimitExceeded { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[derive(Clone)]
struct RecordingMcpClient {
    output: Result<McpClientOutput, String>,
    requests: Arc<Mutex<Vec<McpClientRequest>>>,
    host_mediated_http: bool,
}

impl RecordingMcpClient {
    fn new(output: Result<McpClientOutput, String>) -> Self {
        Self {
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
            host_mediated_http: true,
        }
    }

    fn direct_network(output: Result<McpClientOutput, String>) -> Self {
        Self {
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
            host_mediated_http: false,
        }
    }
}

#[async_trait]
impl McpClient for RecordingMcpClient {
    fn uses_host_mediated_http_egress(&self) -> bool {
        self.host_mediated_http
    }

    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String> {
        self.requests.lock().unwrap().push(request);
        self.output.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordedResponseMode {
    Json,
    Sse,
}

#[derive(Debug, Clone)]
struct RecordingRuntimeEgress {
    mode: RecordedResponseMode,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeEgress {
    fn json_rpc() -> Self {
        Self {
            mode: RecordedResponseMode::Json,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn sse() -> Self {
        Self {
            mode: RecordedResponseMode::Sse,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for RecordingRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        match method.as_str() {
            "initialize" => Ok(runtime_json_response(
                Some(1),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-123".to_string())],
            )),
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            }),
            "tools/call" => {
                let id = json_rpc_id(&request.body);
                match self.mode {
                    RecordedResponseMode::Json => Ok(runtime_json_response(
                        id,
                        json!({"content":[{"type":"text","text":"ok"}],"isError":false}),
                        vec![],
                    )),
                    RecordedResponseMode::Sse => Ok(runtime_sse_response(
                        id,
                        json!({"content":[{"type":"text","text":"ok from sse"}],"isError":false}),
                    )),
                }
            }
            other => panic!("unexpected MCP JSON-RPC method {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedPlanCall {
    scope: ResourceScope,
    method: NetworkMethod,
    url: String,
}

#[derive(Debug, Clone)]
struct RecordingEgressPlanner {
    plan: McpHostHttpEgressPlan,
    calls: Arc<Mutex<Vec<RecordedPlanCall>>>,
}

impl RecordingEgressPlanner {
    fn new(plan: McpHostHttpEgressPlan) -> Self {
        Self {
            plan,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<RecordedPlanCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl McpHostHttpEgressPlanner for RecordingEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        self.calls.lock().unwrap().push(RecordedPlanCall {
            scope: request.scope.clone(),
            method: request.method,
            url: request.url.to_string(),
        });
        self.plan.clone()
    }
}

fn host_http_plan() -> McpHostHttpEgressPlan {
    McpHostHttpEgressPlan {
        network_policy: mcp_http_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle: SecretHandle::new("github-token").unwrap(),
            source: RuntimeCredentialSource::SecretStoreLease,
            target: RuntimeCredentialTarget::Header {
                name: "Authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        response_body_limit: Some(4096),
        timeout_ms: Some(2_500),
    }
}

fn json_rpc_method(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .unwrap()
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_string()
}

fn json_rpc_id(body: &[u8]) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(body)
        .unwrap()
        .get("id")
        .and_then(serde_json::Value::as_u64)
}

fn json_rpc_param(body: &[u8], key: &str) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(body).unwrap()["params"][key].clone()
}

fn runtime_json_response(
    id: Option<u64>,
    result: serde_json::Value,
    extra_headers: Vec<(String, String)>,
) -> RuntimeHttpEgressResponse {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    headers.extend(extra_headers);
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .unwrap();
    RuntimeHttpEgressResponse {
        status: 200,
        headers,
        response_bytes: body.len() as u64,
        body,
        request_bytes: 0,
        redaction_applied: false,
    }
}

fn runtime_sse_response(id: Option<u64>, result: serde_json::Value) -> RuntimeHttpEgressResponse {
    let event = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let body = format!("event: message\ndata: {event}\n\n").into_bytes();
    RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
        response_bytes: body.len() as u64,
        body,
        request_bytes: 0,
        redaction_applied: false,
    }
}

#[derive(Debug, Clone)]
struct ScopedSessionRuntimeEgress {
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl ScopedSessionRuntimeEgress {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for ScopedSessionRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        match method.as_str() {
            "initialize" => Ok(runtime_json_response(
                Some(json_rpc_id(&request.body).unwrap()),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }),
                vec![(
                    "Mcp-Session-Id".to_string(),
                    format!("session-{}", request.scope.user_id.as_str()),
                )],
            )),
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            }),
            "tools/call" => Ok(runtime_json_response(
                json_rpc_id(&request.body),
                json!({"content":[{"type":"text","text":"ok"}],"isError":false}),
                vec![],
            )),
            other => panic!("unexpected MCP JSON-RPC method {other}"),
        }
    }
}

#[derive(Debug)]
struct InvalidSessionRuntimeEgress;

impl RuntimeHttpEgress for InvalidSessionRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        assert_eq!(json_rpc_method(&request.body), "initialize");
        Ok(runtime_json_response(
            Some(1),
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
            }),
            vec![(
                "Mcp-Session-Id".to_string(),
                "bad\r\nInjected: yes".to_string(),
            )],
        ))
    }
}

#[derive(Debug)]
struct MissingIdRuntimeEgress;

impl RuntimeHttpEgress for MissingIdRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        match json_rpc_method(&request.body).as_str() {
            "initialize" => Ok(runtime_json_response(
                json_rpc_id(&request.body),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }),
                vec![],
            )),
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            }),
            "tools/call" => Ok(runtime_json_response(
                None,
                json!({"content":[{"type":"text","text":"missing id"}],"isError":false}),
                vec![],
            )),
            other => panic!("unexpected MCP JSON-RPC method {other}"),
        }
    }
}

#[derive(Debug, Clone)]
struct ErrorSessionRuntimeEgress {
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
    initialize_count: Arc<Mutex<u32>>,
}

impl ErrorSessionRuntimeEgress {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            initialize_count: Arc::new(Mutex::new(0)),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for ErrorSessionRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        match method.as_str() {
            "initialize" => {
                let mut initialize_count = self.initialize_count.lock().unwrap();
                *initialize_count += 1;
                if *initialize_count == 1 {
                    return Ok(RuntimeHttpEgressResponse {
                        status: 500,
                        headers: vec![(
                            "Mcp-Session-Id".to_string(),
                            "session-from-error".to_string(),
                        )],
                        body: b"server error".to_vec(),
                        request_bytes: request.body.len() as u64,
                        response_bytes: "server error".len() as u64,
                        redaction_applied: false,
                    });
                }
                Ok(runtime_json_response(
                    json_rpc_id(&request.body),
                    json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                    }),
                    vec![("Mcp-Session-Id".to_string(), "session-good".to_string())],
                ))
            }
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            }),
            "tools/call" => Ok(runtime_json_response(
                json_rpc_id(&request.body),
                json!({"content":[{"type":"text","text":"ok"}],"isError":false}),
                vec![],
            )),
            other => panic!("unexpected MCP JSON-RPC method {other}"),
        }
    }
}

#[derive(Debug)]
struct SecretEchoRuntimeEgress;

impl RuntimeHttpEgress for SecretEchoRuntimeEgress {
    fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        Err(RuntimeHttpEgressError::Network {
            reason: "private target 10.0.0.7 denied for sk-test-secret".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })
    }
}

struct ReleaseFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ReleaseFailingGovernor {
    fn new() -> Self {
        Self {
            inner: InMemoryResourceGovernor::new(),
        }
    }
}

impl ResourceGovernor for ReleaseFailingGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        self.inner.set_limit(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.inner.reserve(scope, estimate)
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        self.inner.reserve_with_id(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        self.inner.reconcile(reservation_id, actual)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::UnknownReservation { id: reservation_id })
    }
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_scope_for_user(user_id: &str) -> ResourceScope {
    let mut scope = sample_scope();
    scope.user_id = UserId::new(user_id).unwrap();
    scope.invocation_id = InvocationId::new();
    scope
}

fn mcp_http_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "mcp.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}

const MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/mcp"

[[capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const STDIO_MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
command = "github-mcp"
args = ["--stdio"]

[[capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script demo extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "script-echo"
args = ["--json"]

[[capabilities]]
id = "script.echo"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
