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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_process_count(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let result = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate::default()
                    .set_concurrency_slots(1)
                    .set_process_count(1)
                    .set_output_bytes(10_000),
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

#[tokio::test]
async fn mcp_host_http_adapter_returns_sanitized_shared_egress_errors() {
    let adapter = McpRuntimeHttpAdapter::new(Arc::new(SecretEchoRuntimeEgress));

    let error = adapter
        .request(CapabilityHostHttpRequest {
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
        .await
        .expect_err("MCP HTTP adapter errors should be sanitized before runtime visibility");

    let rendered = error.to_string();
    assert!(rendered.contains("network_error"));
    assert!(!rendered.contains("sk-test-secret"));
    assert!(!rendered.contains("10.0.0.7"));
}

#[tokio::test]
async fn mcp_host_http_adapter_maps_panicking_runtime_egress_to_sanitized_error() {
    let adapter = McpRuntimeHttpAdapter::new(Arc::new(PanickingRuntimeEgress));

    let error = adapter
        .request(CapabilityHostHttpRequest {
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
        .await
        .expect_err("runtime egress panics should be contained at the MCP host boundary");

    let rendered = error.to_string();
    assert!(rendered.contains("runtime_http_egress_panicked"));
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
            .all(|request| request.credential_injections == plan.credential_injections),
        "host-staged MCP credentials must cover initialize, initialized, and tools/call"
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
        json_rpc_param(&requests[0].body, "protocolVersion"),
        json!("2025-06-18")
    );
    assert_eq!(
        json_rpc_method(&requests[1].body),
        "notifications/initialized"
    );
    assert_eq!(json_rpc_method(&requests[2].body), "tools/call");
    assert_eq!(
        header_value(&requests[0].headers, "MCP-Protocol-Version"),
        None,
        "initialize is the negotiation request and must not carry stale protocol metadata"
    );
    assert_eq!(
        header_value(&requests[1].headers, "MCP-Protocol-Version"),
        Some("2025-06-18")
    );
    assert_eq!(
        header_value(&requests[2].headers, "MCP-Protocol-Version"),
        Some("2025-06-18")
    );
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
    assert_eq!(
        planner_calls
            .iter()
            .map(|call| call.json_rpc_method.as_str())
            .collect::<Vec<_>>(),
        vec!["tools/call", "initialize", "notifications/initialized"]
    );
    assert_eq!(planner_calls[0].json_rpc_id, json_rpc_id(&requests[2].body));
}

#[tokio::test]
async fn concrete_mcp_http_client_maps_upstream_auth_status_to_auth_required() {
    let egress = RecordingRuntimeEgress::auth_required();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
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
        .expect_err("upstream MCP auth failures must become auth-required errors");

    assert!(matches!(error, McpClientError::AuthRequired));
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(json_rpc_method(&requests[0].body), "initialize");
}

#[tokio::test]
async fn concrete_mcp_http_client_uses_negotiated_protocol_version_header() {
    let egress = RecordingRuntimeEgress::json_rpc_with_protocol_version("2025-03-26");
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
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
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(
        json_rpc_param(&requests[0].body, "protocolVersion"),
        json!("2025-06-18")
    );
    assert_eq!(
        header_value(&requests[1].headers, "MCP-Protocol-Version"),
        Some("2025-03-26")
    );
    assert_eq!(
        header_value(&requests[2].headers, "MCP-Protocol-Version"),
        Some("2025-03-26")
    );
}

#[tokio::test]
async fn concrete_mcp_http_client_reuses_rotated_session_id_after_initialized() {
    let egress = RotatingSessionRuntimeEgress::new();
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
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
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(json_rpc_method(&requests[2].body), "tools/call");
    assert_eq!(
        header_value(&requests[2].headers, "Mcp-Session-Id"),
        Some("session-rotated")
    );
}

#[tokio::test]
async fn concrete_mcp_http_client_rejects_missing_or_unsafe_initialize_protocol_version() {
    for egress in [
        RecordingRuntimeEgress::json_rpc_without_protocol_version(),
        RecordingRuntimeEgress::json_rpc_with_protocol_version(""),
        RecordingRuntimeEgress::json_rpc_with_protocol_version("2025/06/18"),
        RecordingRuntimeEgress::json_rpc_with_protocol_version(
            "2025-06-18-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    ] {
        let client = McpHostHttpClient::new(
            McpRuntimeHttpAdapter::new(Arc::new(egress)),
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
            .expect_err("unsafe initialize protocol versions must fail the call");

        assert_eq!(error.stable_reason(), "mcp_invalid_protocol_version");
    }
}

#[tokio::test]
async fn concrete_mcp_http_client_sends_credentials_only_for_tool_call_exchange() {
    let scope = sample_scope();
    let mut plan = host_http_plan();
    let secret_store_lease = RuntimeCredentialInjection {
        handle: SecretHandle::new("legacy-token").unwrap(),
        source: RuntimeCredentialSource::SecretStoreLease,
        target: RuntimeCredentialTarget::Header {
            name: "Authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    };
    plan.credential_injections = vec![secret_store_lease];
    let egress = RecordingRuntimeEgress::json_rpc();
    let planner = RecordingEgressPlanner::new(plan.clone());
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        planner.clone(),
    );

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: scope.clone(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("direct secret-store leases must fail before MCP transport");

    assert_eq!(error.stable_reason(), "mcp_denied_credential_source");
    assert!(
        egress.requests().is_empty(),
        "direct leases must be rejected before initialize or tools/call transport"
    );
    assert_eq!(planner.calls().len(), 1);

    planner.set_plan(host_http_plan());
    client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect("failed direct-lease preflight must not poison later MCP session state");

    let requests = egress.requests();
    assert_eq!(
        requests.len(),
        3,
        "legitimate call should perform initialize, initialized, tools/call after failed preflight"
    );
    assert_eq!(json_rpc_method(&requests[0].body), "initialize");
    assert!(
        requests[0]
            .headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("Mcp-Session-Id")),
        "failed preflight must not leave a stale session id for the next initialize"
    );
    assert_eq!(planner.calls().len(), 4);
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
    assert_eq!(error.stable_reason(), "mcp_http_status_500");

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

    assert_eq!(error.stable_reason(), "mcp_jsonrpc_id_mismatch");
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
async fn concrete_mcp_http_client_discovers_tool_schemas_through_shared_egress() {
    let egress = RecordingRuntimeEgress::json_rpc();
    let planner = RecordingEgressPlanner::new(host_http_plan());
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(egress.clone())),
        planner.clone(),
    );

    let output = client
        .discover_tools(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({}),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    assert_eq!(output.tools.len(), 2);
    assert_eq!(output.tools[0].name, "search");
    assert_eq!(
        output.tools[0].description,
        "Search GitHub issues\nacross repositories"
    );
    assert!(output.tools[0].annotations.read_only_hint);
    assert!(!output.tools[0].annotations.side_effects_hint);
    assert_eq!(
        output.tools[0].input_schema,
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        })
    );
    assert_eq!(output.tools[1].name, "issue.create");
    assert!(output.tools[1].annotations.side_effects_hint);

    let requests = egress.requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| json_rpc_method(&request.body))
            .collect::<Vec<_>>(),
        vec!["initialize", "notifications/initialized", "tools/list"]
    );
    assert!(
        requests
            .iter()
            .all(|request| request.credential_injections == host_http_plan().credential_injections),
        "host-staged MCP credentials must cover initialize, initialized, and tools/list discovery"
    );

    let planner_calls = planner.calls();
    assert_eq!(
        planner_calls
            .iter()
            .map(|call| call.json_rpc_method.as_str())
            .collect::<Vec<_>>(),
        vec!["tools/list", "initialize", "notifications/initialized"]
    );
}

/// Coverage gap noted in review of the SSE contract tests: the loopback MCP
/// path predeclares its tool schemas, so `discover_tools` (host-mediated HTTP
/// path) is only ever exercised over JSON-framed `tools/list` responses
/// elsewhere in this file. A real MCP server is free to answer `tools/list`
/// as a single SSE event (same as `tools/call` in
/// `concrete_mcp_sse_client_parses_event_stream_through_shared_egress`), so
/// this drives discovery against an SSE-framed response and asserts the
/// parsed tool schemas are byte-identical to the JSON-framed discovery result.
#[tokio::test]
async fn concrete_mcp_http_client_discovers_tool_schemas_over_sse_framing() {
    let sse_egress = RecordingRuntimeEgress::sse();
    let sse_client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(sse_egress.clone())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let sse_output = sse_client
        .discover_tools(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "sse".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/sse".to_string()),
            input: json!({}),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    let json_client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(RecordingRuntimeEgress::json_rpc())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );
    let json_output = json_client
        .discover_tools(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({}),
            max_output_bytes: 4096,
        })
        .await
        .unwrap();

    assert_eq!(
        sse_output.tools, json_output.tools,
        "SSE-framed tools/list must parse to the same tool schemas as JSON-framed tools/list"
    );
    assert_eq!(sse_egress.requests().len(), 3);
}

#[tokio::test]
async fn concrete_mcp_http_client_maps_discovery_auth_status_to_auth_required() {
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(RecordingRuntimeEgress::auth_required())),
        StaticMcpHostHttpEgressPlanner::new(host_http_plan()),
    );

    let error = client
        .discover_tools(McpClientRequest {
            provider: ExtensionId::new("github-mcp").unwrap(),
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://mcp.example.test/mcp".to_string()),
            input: json!({}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("upstream MCP discovery auth failures must stay typed");

    assert!(matches!(error, McpClientError::AuthRequired));
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

    assert_eq!(error.stable_reason(), "mcp_invalid_session_id");
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

    assert_eq!(error.stable_reason(), "network_error");
    assert!(!format!("{error:?}").contains("sk-test-secret"));
    assert!(!format!("{error:?}").contains("10.0.0.7"));
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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_output_bytes(1),
        )
        .unwrap();

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope,
                estimate: ResourceEstimate::default().set_output_bytes(10_000),
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
    let client = RecordingMcpClient::new(Err(McpClientError::client("server disconnected")));
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
                estimate: ResourceEstimate::default().set_concurrency_slots(1),
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
    let client = RecordingMcpClient::new(Err(McpClientError::client("server disconnected")));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client);
    let governor = ReleaseFailingGovernor::new();

    let err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &package,
                capability_id: &CapabilityId::new("github-mcp.search").unwrap(),
                scope: sample_scope(),
                estimate: ResourceEstimate::default().set_concurrency_slots(1),
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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_concurrency_slots(0),
        )
        .unwrap();

    let non_mcp_err = runtime
        .execute_extension_json(
            &governor,
            McpExecutionRequest {
                package: &non_mcp,
                capability_id: &CapabilityId::new("script.echo").unwrap(),
                scope: scope.clone(),
                estimate: ResourceEstimate::default().set_concurrency_slots(1),
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
                estimate: ResourceEstimate::default().set_concurrency_slots(1),
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
                estimate: ResourceEstimate::default()
                    .set_concurrency_slots(1)
                    .set_output_bytes(10_000),
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
                estimate: ResourceEstimate::default()
                    .set_concurrency_slots(1)
                    .set_output_bytes(10_000),
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
                estimate: ResourceEstimate::default()
                    .set_concurrency_slots(1)
                    .set_output_bytes(10_000),
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
    output: Result<McpClientOutput, McpClientError>,
    requests: Arc<Mutex<Vec<McpClientRequest>>>,
    host_mediated_http: bool,
}

impl RecordingMcpClient {
    fn new(output: Result<McpClientOutput, McpClientError>) -> Self {
        Self {
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
            host_mediated_http: true,
        }
    }

    fn direct_network(output: Result<McpClientOutput, McpClientError>) -> Self {
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

    async fn call_tool(
        &self,
        request: McpClientRequest,
    ) -> Result<McpClientOutput, McpClientError> {
        self.requests.lock().unwrap().push(request);
        self.output.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordedResponseMode {
    Json,
    AuthRequired,
    JsonMissingProtocolVersion,
    Sse,
}

#[derive(Debug, Clone)]
struct RecordingRuntimeEgress {
    mode: RecordedResponseMode,
    protocol_version: &'static str,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeEgress {
    fn json_rpc() -> Self {
        Self::json_rpc_with_protocol_version("2025-06-18")
    }

    fn json_rpc_with_protocol_version(protocol_version: &'static str) -> Self {
        Self {
            mode: RecordedResponseMode::Json,
            protocol_version,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn auth_required() -> Self {
        Self {
            mode: RecordedResponseMode::AuthRequired,
            protocol_version: "2025-06-18",
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn json_rpc_without_protocol_version() -> Self {
        Self {
            mode: RecordedResponseMode::JsonMissingProtocolVersion,
            protocol_version: "2025-06-18",
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn sse() -> Self {
        Self {
            mode: RecordedResponseMode::Sse,
            protocol_version: "2025-06-18",
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        if self.mode == RecordedResponseMode::AuthRequired {
            return Ok(RuntimeHttpEgressResponse {
                status: 401,
                headers: vec![],
                body: br#"{"error":"unauthorized"}"#.to_vec(),
                saved_body: None,
                request_bytes: request.body.len() as u64,
                response_bytes: 24,
                redaction_applied: false,
            });
        }
        match method.as_str() {
            "initialize" => {
                let mut result = json!({
                    "protocolVersion": self.protocol_version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                });
                if self.mode == RecordedResponseMode::JsonMissingProtocolVersion {
                    result
                        .as_object_mut()
                        .expect("initialize result is an object")
                        .remove("protocolVersion");
                }
                Ok(runtime_json_response(
                    json_rpc_id(&request.body),
                    result,
                    vec![("Mcp-Session-Id".to_string(), "session-123".to_string())],
                ))
            }
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                saved_body: None,
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            }),
            "tools/call" => {
                let id = json_rpc_id(&request.body);
                match self.mode {
                    RecordedResponseMode::Json
                    | RecordedResponseMode::JsonMissingProtocolVersion => {
                        Ok(runtime_json_response(
                            id,
                            json!({"content":[{"type":"text","text":"ok"}],"isError":false}),
                            vec![],
                        ))
                    }
                    RecordedResponseMode::Sse => Ok(runtime_sse_response(
                        id,
                        json!({"content":[{"type":"text","text":"ok from sse"}],"isError":false}),
                    )),
                    RecordedResponseMode::AuthRequired => {
                        unreachable!("auth-required mode returns before JSON-RPC method dispatch")
                    }
                }
            }
            "tools/list" => {
                let id = json_rpc_id(&request.body);
                let result = json!({
                    "tools": [
                        {
                            "name": "search",
                            "description": "Search GitHub issues\nacross repositories",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": {"type": "string"}
                                },
                                "required": ["query"]
                            },
                            "annotations": {
                                "readOnlyHint": true
                            }
                        },
                        {
                            "name": "issue.create",
                            "description": "Create an issue",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "title": {"type": "string"}
                                }
                            },
                            "annotations": {
                                "sideEffectsHint": true
                            }
                        }
                    ]
                });
                match self.mode {
                    RecordedResponseMode::Json
                    | RecordedResponseMode::JsonMissingProtocolVersion => {
                        Ok(runtime_json_response(id, result, vec![]))
                    }
                    RecordedResponseMode::Sse => Ok(runtime_sse_response(id, result)),
                    RecordedResponseMode::AuthRequired => {
                        unreachable!("auth-required mode returns before JSON-RPC method dispatch")
                    }
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
    json_rpc_method: String,
    json_rpc_id: Option<u64>,
}

#[derive(Debug, Clone)]
struct RecordingEgressPlanner {
    plan: Arc<Mutex<McpHostHttpEgressPlan>>,
    calls: Arc<Mutex<Vec<RecordedPlanCall>>>,
}

impl RecordingEgressPlanner {
    fn new(plan: McpHostHttpEgressPlan) -> Self {
        Self {
            plan: Arc::new(Mutex::new(plan)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<RecordedPlanCall> {
        self.calls.lock().unwrap().clone()
    }

    fn set_plan(&self, plan: McpHostHttpEgressPlan) {
        *self.plan.lock().unwrap() = plan;
    }
}

impl McpHostHttpEgressPlanner for RecordingEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        self.calls.lock().unwrap().push(RecordedPlanCall {
            scope: request.scope.clone(),
            method: request.method,
            url: request.url.to_string(),
            json_rpc_method: json_rpc_method(request.body),
            json_rpc_id: json_rpc_id(request.body),
        });
        self.plan.lock().unwrap().clone()
    }
}

fn host_http_plan() -> McpHostHttpEgressPlan {
    McpHostHttpEgressPlan {
        network_policy: mcp_http_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle: SecretHandle::new("github-token").unwrap(),
            source: RuntimeCredentialSource::StagedObligation {
                capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            },
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

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
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
        saved_body: None,
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
        saved_body: None,
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for ScopedSessionRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        match method.as_str() {
            "initialize" => Ok(runtime_json_response(
                Some(json_rpc_id(&request.body).unwrap()),
                json!({
                    "protocolVersion": "2025-06-18",
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
                saved_body: None,
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

#[derive(Debug, Clone)]
struct RotatingSessionRuntimeEgress {
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RotatingSessionRuntimeEgress {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RotatingSessionRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = json_rpc_method(&request.body);
        self.requests.lock().unwrap().push(request.clone());
        match method.as_str() {
            "initialize" => Ok(runtime_json_response(
                Some(json_rpc_id(&request.body).unwrap()),
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-initial".to_string())],
            )),
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![("Mcp-Session-Id".to_string(), "session-rotated".to_string())],
                body: vec![],
                saved_body: None,
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for InvalidSessionRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        assert_eq!(json_rpc_method(&request.body), "initialize");
        Ok(runtime_json_response(
            Some(1),
            json!({
                "protocolVersion": "2025-06-18",
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for MissingIdRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        match json_rpc_method(&request.body).as_str() {
            "initialize" => Ok(runtime_json_response(
                json_rpc_id(&request.body),
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "mock-mcp", "version": "1.0.0"}
                }),
                vec![],
            )),
            "notifications/initialized" => Ok(RuntimeHttpEgressResponse {
                status: 202,
                headers: vec![],
                body: vec![],
                saved_body: None,
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for ErrorSessionRuntimeEgress {
    async fn execute(
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
                        saved_body: None,
                        request_bytes: request.body.len() as u64,
                        response_bytes: "server error".len() as u64,
                        redaction_applied: false,
                    });
                }
                Ok(runtime_json_response(
                    json_rpc_id(&request.body),
                    json!({
                        "protocolVersion": "2025-06-18",
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
                saved_body: None,
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for SecretEchoRuntimeEgress {
    async fn execute(
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

#[derive(Debug)]
struct PanickingRuntimeEgress;

#[async_trait]
impl RuntimeHttpEgress for PanickingRuntimeEgress {
    async fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        panic!("runtime HTTP egress should not unwind through MCP host");
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
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        self.inner.set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
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

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<ironclaw_resources::AccountSnapshot>, ResourceError> {
        self.inner.account_snapshot(account)
    }
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn capability_provider_contracts() -> HostApiContractRegistry {
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(Arc::new(CapabilityProviderHostApiContract::new().unwrap()))
        .unwrap();
    contracts
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

const MCP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
visibility = "api"
input_schema_ref = "schemas/github-mcp/search.input.v1.json"
output_schema_ref = "schemas/github-mcp/search.output.v1.json"
"#;

const STDIO_MCP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
visibility = "api"
input_schema_ref = "schemas/github-mcp/search.input.v1.json"
output_schema_ref = "schemas/github-mcp/search.output.v1.json"
"#;

const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "script.echo"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/script/echo.input.v1.json"
output_schema_ref = "schemas/script/echo.output.v1.json"
"#;
