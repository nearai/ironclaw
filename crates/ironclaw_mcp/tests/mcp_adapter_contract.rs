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
