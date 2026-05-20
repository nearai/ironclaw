use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
    HostApiContractRegistry, ManifestSource,
};
use ironclaw_host_api::*;
use ironclaw_mcp::*;
use ironclaw_resources::*;
use serde_json::json;

#[tokio::test]
async fn mcp_lane_executes_manifest_transport_and_reconciles_resources() {
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"items":["issue-1"]}),
        usage: ResourceUsage {
            wall_clock_ms: 9,
            ..ResourceUsage::default()
        },
        output_bytes: None,
    }));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let (governor, account) = mcp_governor();

    let result = runtime
        .execute_extension_json(&governor, mcp_request(json!({"query":"ironclaw"})))
        .await
        .unwrap();

    assert_eq!(result.result.output, json!({"items":["issue-1"]}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(result.result.usage.process_count, 0);
    assert_eq!(result.result.usage.wall_clock_ms, 9);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transport, "http");
    assert_eq!(
        requests[0].url.as_deref(),
        Some("https://mcp.example.test/rpc")
    );
    assert_eq!(requests[0].command, None);
    assert!(requests[0].args.is_empty());
    assert_eq!(requests[0].input, json!({"query":"ironclaw"}));
}

#[tokio::test]
async fn mcp_lane_client_failure_releases_reservation() {
    let client = RecordingMcpClient::new(Err("server disconnected with raw stderr".to_string()));
    let runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client);
    let (governor, account) = mcp_governor();

    let err = runtime
        .execute_extension_json(&governor, mcp_request(json!({"query":"fail"})))
        .await
        .unwrap_err();

    assert!(matches!(err, McpError::Client { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn mcp_lane_output_limit_releases_reservation() {
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"large":"this output is too large for the adapter limit"}),
        usage: ResourceUsage::default(),
        output_bytes: Some(1_000),
    }));
    let runtime = McpRuntime::new(
        McpRuntimeConfig {
            max_output_bytes: 8,
        },
        client,
    );
    let (governor, account) = mcp_governor();

    let err = runtime
        .execute_extension_json(&governor, mcp_request(json!({"query":"large"})))
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
}

impl RecordingMcpClient {
    fn new(output: Result<McpClientOutput, String>) -> Self {
        Self {
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<McpClientRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl McpClient for RecordingMcpClient {
    fn uses_host_mediated_http_egress(&self) -> bool {
        true
    }

    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String> {
        self.requests.lock().unwrap().push(request);
        self.output.clone()
    }
}

fn mcp_governor() -> (InMemoryResourceGovernor, ResourceAccount) {
    let account = sample_account();
    let governor = governor_with_default_limit(account.clone());
    (governor, account)
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

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(10),
                max_process_count: Some(10),
                max_output_bytes: Some(100_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
}

fn mcp_request(input: serde_json::Value) -> McpExecutionRequest<'static> {
    let package = Box::leak(Box::new(package_from_manifest(MCP_MANIFEST)));
    let capability_id = Box::leak(Box::new(CapabilityId::new("github-mcp.search").unwrap()));
    McpExecutionRequest {
        package,
        capability_id,
        scope: sample_scope(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            process_count: Some(1),
            output_bytes: Some(10_000),
            ..ResourceEstimate::default()
        },
        resource_reservation: None,
        invocation: McpInvocation { input },
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

fn sample_account() -> ResourceAccount {
    ResourceAccount::tenant(TenantId::new("tenant-a").unwrap())
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
url = "https://mcp.example.test/rpc"

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
