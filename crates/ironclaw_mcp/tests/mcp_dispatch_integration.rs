use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_mcp::*;
use ironclaw_resources::*;
use serde_json::json;

#[tokio::test]
async fn mcp_lane_dispatches_manifest_transport_and_reconciles_through_dispatcher() {
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"items":["issue-1"]}),
        usage: ResourceUsage {
            wall_clock_ms: 9,
            ..ResourceUsage::default()
        },
        output_bytes: None,
    }));
    let adapter = Arc::new(McpRuntimeAdapter::new(client.clone()));
    let (dispatcher, governor, events, account) = dispatcher_with_mcp_adapter(adapter);

    let result = dispatcher
        .dispatch_json(dispatch_request(json!({"query":"ironclaw"})))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Mcp);
    assert_eq!(result.output, json!({"items":["issue-1"]}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(result.usage.process_count, 0);
    assert_eq!(result.usage.wall_clock_ms, 9);
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

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

#[tokio::test]
async fn mcp_lane_client_failure_releases_reservation_and_emits_sanitized_failure() {
    let client = RecordingMcpClient::new(Err("server disconnected with raw stderr".to_string()));
    let adapter = Arc::new(McpRuntimeAdapter::new(client));
    let (dispatcher, governor, events, account) = dispatcher_with_mcp_adapter(adapter);

    let err = dispatcher
        .dispatch_json(dispatch_request(json!({"query":"fail"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::Client
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
    let recorded = events.events();
    assert_eq!(recorded[2].error_kind.as_deref(), Some("client"));
}

#[tokio::test]
async fn mcp_lane_output_limit_releases_reservation_and_emits_output_too_large_failure() {
    let client = RecordingMcpClient::new(Ok(McpClientOutput {
        output: json!({"large":"this output is too large for the adapter limit"}),
        usage: ResourceUsage::default(),
        output_bytes: Some(1_000),
    }));
    let adapter = Arc::new(McpRuntimeAdapter::with_config(
        McpRuntimeConfig {
            max_output_bytes: 8,
        },
        client,
    ));
    let (dispatcher, governor, events, account) = dispatcher_with_mcp_adapter(adapter);

    let err = dispatcher
        .dispatch_json(dispatch_request(json!({"query":"large"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::OutputTooLarge
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
    let recorded = events.events();
    assert_eq!(recorded[2].error_kind.as_deref(), Some("output_too_large"));
}

#[derive(Clone)]
struct McpRuntimeAdapter<C> {
    runtime: McpRuntime<C>,
}

impl<C> McpRuntimeAdapter<C>
where
    C: McpClient,
{
    fn new(client: C) -> Self {
        Self::with_config(McpRuntimeConfig::for_testing(), client)
    }

    fn with_config(config: McpRuntimeConfig, client: C) -> Self {
        Self {
            runtime: McpRuntime::new(config, client),
        }
    }
}

#[async_trait]
impl<C> RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for McpRuntimeAdapter<C>
where
    C: McpClient + Send + Sync,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    resource_reservation: request.resource_reservation,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(|error| DispatchError::Mcp {
                kind: mcp_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
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

fn dispatcher_with_mcp_adapter<T>(
    adapter: Arc<T>,
) -> (
    RuntimeDispatcher<'static, LocalFilesystem, InMemoryResourceGovernor>,
    Arc<InMemoryResourceGovernor>,
    InMemoryEventSink,
    ResourceAccount,
)
where
    T: RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> + 'static,
{
    let account = sample_account();
    let registry = Arc::new(registry_with_package(MCP_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(governor_with_default_limit(account.clone()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, filesystem, Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Mcp, adapter)
        .with_event_sink_arc(Arc::new(events.clone()));
    (dispatcher, governor, events, account)
}

fn registry_with_package(manifest: &str) -> ironclaw_extensions::ExtensionRegistry {
    let mut registry = ironclaw_extensions::ExtensionRegistry::new();
    registry.insert(package_from_manifest(manifest)).unwrap();
    registry
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn mounted_empty_extension_root() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor.set_limit(
        account,
        ResourceLimits {
            max_concurrency_slots: Some(10),
            max_process_count: Some(10),
            max_output_bytes: Some(100_000),
            ..ResourceLimits::default()
        },
    );
    governor
}

fn dispatch_request(input: serde_json::Value) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        capability_id: CapabilityId::new("github-mcp.search").unwrap(),
        scope: sample_scope(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            process_count: Some(1),
            output_bytes: Some(10_000),
            ..ResourceEstimate::default()
        },
        mounts: None,
        resource_reservation: None,
        input,
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

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::UnsupportedTransport { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::HostHttpEgressRequired { .. } => RuntimeDispatchErrorKind::NetworkDenied,
        McpError::ExternalStdioTransportUnsupported => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        McpError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        McpError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        McpError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        McpError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
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
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;
