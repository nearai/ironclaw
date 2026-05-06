use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::{Value, json};
use tracing::Instrument;
use tracing_test::traced_test;

#[tokio::test]
async fn dispatcher_emits_events_for_wasm_and_script_success() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_adapter = EchoAdapter::new(RuntimeKind::Wasm);
    let script_adapter = EchoAdapter::new(RuntimeKind::Script);
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Wasm, &wasm_adapter)
        .with_runtime_adapter(RuntimeKind::Script, &script_adapter)
        .with_event_sink(&events);

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello wasm"}),
        })
        .await
        .unwrap();

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello script"}),
        })
        .await
        .unwrap();

    let recorded = events.events();
    let kinds = recorded.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    assert_eq!(
        recorded[0].capability_id,
        CapabilityId::new("echo-wasm.say").unwrap()
    );
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(recorded[2].output_bytes, Some(24));
    assert_eq!(
        recorded[3].capability_id,
        CapabilityId::new("echo-script.say").unwrap()
    );
    assert_eq!(recorded[4].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[5].provider,
        Some(ExtensionId::new("echo-script").unwrap())
    );
}

#[tokio::test]
async fn dispatcher_ignores_event_sink_failures_on_success() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_adapter = EchoAdapter::new(RuntimeKind::Wasm);
    let events = FailingEventSink;
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Wasm, &wasm_adapter)
        .with_event_sink(&events);

    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "event sink fails"}),
        })
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "event sink fails"}));
}

#[tokio::test]
async fn dispatcher_preserves_original_error_when_failure_event_sink_fails() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let events = FailingEventSink;
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "missing backend"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script
        }
    ));
}

#[tokio::test]
#[traced_test]
async fn dispatcher_logs_release_failure_without_masking_dispatch_error() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let reservation = ResourceReservation {
        id: ResourceReservationId::new(),
        scope: scope.clone(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            process_count: Some(1),
            ..ResourceEstimate::default()
        },
    };
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: Some(reservation.clone()),
            input: json!({"message": "missing backend"}),
        })
        .instrument(tracing::info_span!(
            "dispatcher_logs_release_failure_without_masking_dispatch_error"
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script
        }
    ));
    assert!(logs_contain(
        "failed to release prepared resource reservation after dispatcher validation failure"
    ));
    assert!(logs_contain(&reservation.id.to_string()));
}

#[tokio::test]
async fn dispatcher_emits_redacted_runtime_error_kind_for_adapter_failure() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let script_adapter =
        FailingRuntimeAdapter::new(RuntimeKind::Script, RuntimeDispatchErrorKind::ExitFailure);
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Script, &script_adapter)
        .with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "adapter fails"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure
        }
    ));

    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[2].error_kind.as_deref(), Some("exit_failure"));
}

#[tokio::test]
async fn dispatcher_emits_events_for_mcp_success() {
    let fs = filesystem_with_echo_extensions();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let mcp_adapter = StaticAdapter::new(RuntimeKind::Mcp, json!({"matches": ["ironclaw"]}));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Mcp, &mcp_adapter)
        .with_event_sink(&events);

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
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
            input: json!({"query": "ironclaw"}),
        })
        .await
        .unwrap();

    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Mcp));
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchSucceeded);
    assert_eq!(
        recorded[2].provider,
        Some(ExtensionId::new("github-mcp").unwrap())
    );
    assert!(recorded[2].output_bytes.unwrap() > 0);
}

#[tokio::test]
async fn dispatcher_emits_failed_event_for_missing_backend_without_reserving() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());

    let recorded = events.events();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[1].error_kind.as_deref(),
        Some("missing_runtime_backend")
    );
}

struct FailingEventSink;

#[async_trait]
impl EventSink for FailingEventSink {
    async fn emit(&self, _event: RuntimeEvent) -> Result<(), EventError> {
        Err(EventError::Sink {
            reason: "event sink unavailable".to_string(),
        })
    }
}

#[derive(Clone)]
struct EchoAdapter {
    runtime: RuntimeKind,
}

impl EchoAdapter {
    fn new(runtime: RuntimeKind) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for EchoAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        adapter_result(
            self.runtime,
            request.governor,
            request.scope,
            request.estimate,
            request.input,
        )
    }
}

#[derive(Clone)]
struct StaticAdapter {
    runtime: RuntimeKind,
    output: Value,
}

impl StaticAdapter {
    fn new(runtime: RuntimeKind, output: Value) -> Self {
        Self { runtime, output }
    }
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for StaticAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        adapter_result(
            self.runtime,
            request.governor,
            request.scope,
            request.estimate,
            self.output.clone(),
        )
    }
}

#[derive(Clone)]
struct FailingRuntimeAdapter {
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
}

impl FailingRuntimeAdapter {
    fn new(runtime: RuntimeKind, kind: RuntimeDispatchErrorKind) -> Self {
        Self { runtime, kind }
    }
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for FailingRuntimeAdapter {
    async fn dispatch_json(
        &self,
        _request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        Err(dispatch_error_for_runtime(self.runtime, self.kind))
    }
}

fn adapter_result(
    runtime: RuntimeKind,
    governor: &InMemoryResourceGovernor,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    output: Value,
) -> Result<RuntimeAdapterResult, DispatchError> {
    let usage = ResourceUsage {
        output_bytes: serde_json::to_vec(&output).unwrap().len() as u64,
        process_count: u32::from(matches!(runtime, RuntimeKind::Script | RuntimeKind::Mcp)),
        ..ResourceUsage::default()
    };
    let reservation = governor
        .reserve(scope, estimate)
        .map_err(|_| dispatch_error_for_runtime(runtime, RuntimeDispatchErrorKind::Resource))?;
    let receipt = governor
        .reconcile(reservation.id, usage.clone())
        .map_err(|_| dispatch_error_for_runtime(runtime, RuntimeDispatchErrorKind::Resource))?;
    Ok(RuntimeAdapterResult {
        output,
        output_bytes: usage.output_bytes,
        usage,
        receipt,
    })
}

fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Wasm => DispatchError::Wasm { kind },
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::UnsupportedRuntime {
            capability: CapabilityId::new("system.unsupported").unwrap(),
            runtime,
        },
    }
}

fn filesystem_with_echo_extensions() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    write_echo_extensions(&storage);

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn write_echo_extensions(root: &std::path::Path) {
    let wasm_root = root.join("echo-wasm");
    std::fs::create_dir_all(wasm_root).unwrap();
    std::fs::write(root.join("echo-wasm/manifest.toml"), WASM_MANIFEST).unwrap();

    let script_root = root.join("echo-script");
    std::fs::create_dir_all(&script_root).unwrap();
    std::fs::write(script_root.join("manifest.toml"), SCRIPT_MANIFEST).unwrap();
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

const WASM_MANIFEST: &str = r#"
id = "echo-wasm"
name = "Echo WASM"
version = "0.1.0"
description = "Echo WASM demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo-wasm.say"
description = "Echo WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "untrusted"

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
id = "echo-script"
name = "Echo Script"
version = "0.1.0"
description = "Echo Script demo extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "echo-script.say"
description = "Echo script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
