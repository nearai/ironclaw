use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::{Value, json};

#[tokio::test]
async fn runtime_dispatcher_routes_already_authorized_request_through_public_trait_object() {
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(RecordingAdapter::new(
        RuntimeKind::Wasm,
        json!({"reply": "from adapter"}),
    ));
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project-a").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();

    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher = RuntimeDispatcher::from_arcs(
        Arc::clone(&registry),
        Arc::clone(&filesystem),
        Arc::clone(&governor),
    )
    .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::clone(&adapter))
    .with_event_sink_arc(Arc::new(events.clone()));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;

    let result = dispatch_port
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: Some(mounts.clone()),
            resource_reservation: None,
            input: json!({"message": "hello through public seam"}),
        })
        .await
        .unwrap();

    assert_eq!(result.capability_id, CapabilityId::new("echo.say").unwrap());
    assert_eq!(result.provider, ExtensionId::new("echo").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(result.output, json!({"reply": "from adapter"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = adapter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, ExtensionId::new("echo").unwrap());
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].mounts, Some(mounts));
    assert_eq!(
        requests[0].input,
        json!({"message": "hello through public seam"})
    );

    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(
        recorded[1].provider,
        Some(ExtensionId::new("echo").unwrap())
    );
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchSucceeded);
    assert_eq!(recorded[2].output_bytes, Some(result.usage.output_bytes));
}

#[tokio::test]
async fn runtime_dispatcher_fails_closed_for_missing_backend_before_reservation_or_adapter_call() {
    let registry = Arc::new(registry_with_package(SCRIPT_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::from_arcs(registry, filesystem, Arc::clone(&governor))
        .with_event_sink_arc(Arc::new(events.clone()));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;

    let err = dispatch_port
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
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

#[tokio::test]
async fn registry_rejects_descriptor_package_runtime_mismatch_before_dispatcher_construction() {
    let manifest = ExtensionManifest::parse(WASM_MANIFEST).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let mut package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    package.capabilities[0].runtime = RuntimeKind::Script;

    let err = ExtensionRegistry::new().insert(package).unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidManifest { reason }
            if reason.contains("package capability descriptors do not match")
    ));
}

#[derive(Clone)]
struct RecordingAdapter {
    runtime: RuntimeKind,
    output: Value,
    requests: Arc<Mutex<Vec<RecordedAdapterRequest>>>,
}

impl RecordingAdapter {
    fn new(runtime: RuntimeKind, output: Value) -> Self {
        Self {
            runtime,
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedAdapterRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedAdapterRequest {
    provider: ExtensionId,
    capability_id: CapabilityId,
    runtime: RuntimeKind,
    scope: ResourceScope,
    mounts: Option<MountView>,
    input: Value,
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for RecordingAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(RecordedAdapterRequest {
            provider: request.package.id.clone(),
            capability_id: request.capability_id.clone(),
            runtime: request.descriptor.runtime,
            scope: request.scope.clone(),
            mounts: request.mounts.clone(),
            input: request.input.clone(),
        });

        let output_bytes = serde_json::to_vec(&self.output).unwrap().len() as u64;
        let usage = ResourceUsage {
            output_bytes,
            process_count: u32::from(matches!(
                self.runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )),
            ..ResourceUsage::default()
        };
        let reservation = request
            .governor
            .reserve(request.scope, request.estimate)
            .map_err(|_| {
                dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
            })?;
        let receipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| {
                dispatch_error_for_runtime(self.runtime, RuntimeDispatchErrorKind::Resource)
            })?;

        Ok(RuntimeAdapterResult {
            output: self.output.clone(),
            usage,
            receipt,
            output_bytes,
        })
    }
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

fn registry_with_package(manifest: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
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

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo WASM"
version = "0.1.0"
description = "Echo WASM integration extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script integration extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "docker"
image = "example/script:latest"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "execute_code"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;
