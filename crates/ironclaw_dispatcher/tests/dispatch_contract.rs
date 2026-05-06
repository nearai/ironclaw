use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::{Value, json};

#[tokio::test]
async fn dispatcher_routes_wasm_capability_through_registered_adapter() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let adapter = RecordingAdapter::new(RuntimeKind::Wasm, json!({"message": "hello adapter"}));
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Wasm, &adapter);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello dispatcher"}),
        })
        .await
        .unwrap();

    assert_eq!(result.capability_id, CapabilityId::new("echo.say").unwrap());
    assert_eq!(result.provider, ExtensionId::new("echo").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(result.output, json!({"message": "hello adapter"}));
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
    assert_eq!(requests[0].input, json!({"message": "hello dispatcher"}));
}

#[tokio::test]
async fn dispatcher_routes_script_capability_through_registered_adapter() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let adapter = RecordingAdapter::new(
        RuntimeKind::Script,
        json!({
            "message": "hello script adapter"
        }),
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(10),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Script, &adapter);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello script dispatcher"}),
        })
        .await
        .unwrap();

    assert_eq!(
        result.capability_id,
        CapabilityId::new("script.echo").unwrap()
    );
    assert_eq!(result.provider, ExtensionId::new("script").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Script);
    assert_eq!(result.output, json!({"message": "hello script adapter"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).process_count, 1);
}

#[tokio::test]
async fn dispatcher_redacts_runtime_adapter_failure_details() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let adapter =
        RecordingAdapter::failing(RuntimeKind::Script, RuntimeDispatchErrorKind::ExitFailure);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    governor.set_limit(
        ResourceAccount::tenant(scope.tenant_id.clone()),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(10),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Script, &adapter);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "redact stderr"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure
        }
    ));
    let message = err.to_string();
    assert!(!message.contains("secret token"));
    assert!(!message.contains("/tmp/private"));
}

#[tokio::test]
async fn dispatcher_routes_mcp_capability_through_registered_adapter() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let adapter = RecordingAdapter::new(
        RuntimeKind::Mcp,
        json!({
            "matches": ["ironclaw"]
        }),
    );
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

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Mcp, &adapter);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
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

    assert_eq!(
        result.capability_id,
        CapabilityId::new("github-mcp.search").unwrap()
    );
    assert_eq!(result.provider, ExtensionId::new("github-mcp").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Mcp);
    assert_eq!(result.output, json!({"matches": ["ironclaw"]}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
}

#[tokio::test]
async fn dispatcher_fails_unknown_capability_without_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let registry = ExtensionRegistry::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let adapter = RecordingAdapter::new(RuntimeKind::Wasm, json!({}));

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_runtime_adapter(RuntimeKind::Wasm, &adapter);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "nope"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(adapter.requests().is_empty());
}

#[tokio::test]
async fn dispatcher_releases_prepared_reservation_when_validation_fails_before_adapter() {
    let fs = mounted_empty_extension_root();
    let registry = ExtensionRegistry::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({"message": "release on validation failure"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_requires_mcp_backend_before_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"query": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Mcp
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_requires_script_backend_before_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
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
}

#[tokio::test]
async fn dispatcher_requires_wasm_backend_before_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
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
            runtime: RuntimeKind::Wasm
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[derive(Clone)]
struct RecordingAdapter {
    runtime: RuntimeKind,
    output: Value,
    failure: Option<RuntimeDispatchErrorKind>,
    requests: Arc<Mutex<Vec<RecordedAdapterRequest>>>,
}

impl RecordingAdapter {
    fn new(runtime: RuntimeKind, output: Value) -> Self {
        Self {
            runtime,
            output,
            failure: None,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing(runtime: RuntimeKind, failure: RuntimeDispatchErrorKind) -> Self {
        Self {
            runtime,
            output: json!(null),
            failure: Some(failure),
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
            input: request.input.clone(),
        });
        if let Some(kind) = self.failure {
            return Err(dispatch_error_for_runtime(self.runtime, kind));
        }

        let usage = ResourceUsage {
            output_bytes: serde_json::to_vec(&self.output).unwrap().len() as u64,
            process_count: u32::from(matches!(
                self.runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )),
            ..ResourceUsage::default()
        };
        let reservation = request
            .governor
            .reserve(request.scope.clone(), request.estimate.clone())
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
            output_bytes: usage.output_bytes,
            usage,
            receipt,
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
id = "echo"
name = "Echo WASM"
version = "0.1.0"
description = "Echo WASM demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
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
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script Echo demo extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "script.echo"
description = "Echo script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
