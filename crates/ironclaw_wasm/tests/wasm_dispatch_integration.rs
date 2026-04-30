use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRuntime};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::{PreparedWitTool, WitToolHost, WitToolRequest, WitToolRuntime};
use serde_json::{Value, json};
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

#[tokio::test]
async fn wasm_lane_loads_component_from_root_filesystem_and_uses_fresh_instances() {
    let component = tool_component(COUNTER_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::new(WasmRuntimeAdapter::new()))
        .with_event_sink_arc(Arc::new(events.clone()));

    let first = dispatcher
        .dispatch_json(dispatch_request("wasm-smoke.count", json!({"call":1})))
        .await
        .unwrap();
    let second = dispatcher
        .dispatch_json(dispatch_request("wasm-smoke.count", json!({"call":2})))
        .await
        .unwrap();

    assert_eq!(first.runtime, RuntimeKind::Wasm);
    assert_eq!(first.output, json!(1));
    assert_eq!(
        second.output,
        json!(1),
        "fresh component instance per dispatch should reset guest globals"
    );
    assert_eq!(first.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(second.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(
        governor.reserved_for(&sample_account()),
        ResourceTally::default()
    );
    assert!(governor.usage_for(&sample_account()).output_bytes >= 2);

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

#[tokio::test]
async fn wasm_lane_guest_trap_releases_reservation_and_preserves_dispatch_failure() {
    let component = tool_component(TRAP_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/trap.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_TRAP_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::new(WasmRuntimeAdapter::new()))
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request("wasm-smoke.trap", json!({"call":"trap"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest
        }
    ));
    assert_eq!(
        governor.reserved_for(&sample_account()),
        ResourceTally::default()
    );
    assert_eq!(
        governor.usage_for(&sample_account()),
        ResourceTally::default()
    );
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
    let recorded = events.events();
    assert_eq!(recorded[2].error_kind.as_deref(), Some("guest"));
}

struct WasmRuntimeAdapter {
    runtime: WitToolRuntime,
}

impl WasmRuntimeAdapter {
    fn new() -> Self {
        Self {
            runtime: WitToolRuntime::new(ironclaw_wasm::WitToolRuntimeConfig::for_testing())
                .unwrap(),
        }
    }
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for WasmRuntimeAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let module_path = match &request.package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module
                .resolve_under(&request.package.root)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Manifest,
                })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                });
            }
        };
        let wasm_bytes = request
            .filesystem
            .read_file(&module_path)
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
            })?;
        let prepared = self
            .runtime
            .prepare(request.capability_id.as_str(), &wasm_bytes)
            .map_err(|error| DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            })?;
        execute_prepared_wasm(&self.runtime, &prepared, request)
    }
}

fn execute_prepared_wasm(
    runtime: &WitToolRuntime,
    prepared: &PreparedWitTool,
    request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
) -> Result<RuntimeAdapterResult, DispatchError> {
    let input_json = serde_json::to_string(&request.input).map_err(|_| DispatchError::Wasm {
        kind: RuntimeDispatchErrorKind::InputEncode,
    })?;
    let reservation = match request.resource_reservation {
        Some(reservation) => reservation,
        None => request
            .governor
            .reserve(request.scope, request.estimate)
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?,
    };
    let execution = match runtime.execute(
        prepared,
        WitToolHost::deny_all(),
        WitToolRequest::new(input_json),
    ) {
        Ok(execution) => execution,
        Err(error) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            });
        }
    };
    if execution.error.is_some() {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        });
    }
    let Some(output_json) = execution.output_json else {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
        });
    };
    let output = match serde_json::from_str::<Value>(&output_json) {
        Ok(output) => output,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            });
        }
    };
    let receipt = match request
        .governor
        .reconcile(reservation.id, execution.usage.clone())
    {
        Ok(receipt) => receipt,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            });
        }
    };
    Ok(RuntimeAdapterResult {
        output,
        output_bytes: execution.usage.output_bytes,
        usage: execution.usage,
        receipt,
    })
}

fn release_wasm_reservation(
    governor: &InMemoryResourceGovernor,
    reservation_id: ResourceReservationId,
) {
    let _ = governor.release(reservation_id);
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

async fn filesystem_with_wasm_component(
    extension_id: &str,
    module_path: &str,
    wasm_bytes: &[u8],
) -> LocalFilesystem {
    let fs = mounted_empty_extension_root();
    let path =
        VirtualPath::new(format!("/system/extensions/{extension_id}/{module_path}")).unwrap();
    fs.write_file(&path, wasm_bytes).await.unwrap();
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

fn dispatch_request(capability: &str, input: Value) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        capability_id: CapabilityId::new(capability).unwrap(),
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

fn wasm_error_kind(error: &ironclaw_wasm::WasmError) -> RuntimeDispatchErrorKind {
    match error {
        ironclaw_wasm::WasmError::EngineCreationFailed(_) => RuntimeDispatchErrorKind::Executor,
        ironclaw_wasm::WasmError::CompilationFailed(_) => RuntimeDispatchErrorKind::Manifest,
        ironclaw_wasm::WasmError::StoreConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        ironclaw_wasm::WasmError::LinkerConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        ironclaw_wasm::WasmError::InstantiationFailed(_) => RuntimeDispatchErrorKind::MethodMissing,
        ironclaw_wasm::WasmError::ExecutionFailed { .. } => RuntimeDispatchErrorKind::Guest,
        ironclaw_wasm::WasmError::InvalidSchema(_) => RuntimeDispatchErrorKind::Manifest,
    }
}

fn tool_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).expect("fixture WAT must parse");
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("tool.wit", include_str!("../../../wit/tool.wit"))
        .expect("tool WIT must parse");
    let world = resolve
        .select_world(&[package], Some("sandboxed-tool"))
        .expect("sandboxed-tool world must exist");

    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .expect("component metadata must embed");

    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .expect("fixture module must decode")
        .validate(true);
    encoder.encode().expect("component must encode")
}

const COUNTER_TOOL_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "counter fixture")
  (data (i32.const 3072) "1")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 15
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    global.get $count
    i32.const 1
    i32.add
    global.set $count
    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    i32.const 3072
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    i32.const 4096)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const TRAP_TOOL_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "trap fixture")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 12
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    unreachable)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    i32.const 4096)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const WASM_MANIFEST: &str = r#"
id = "wasm-smoke"
name = "WASM Smoke"
version = "0.1.0"
description = "WASM runtime lane smoke extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/counter.wasm"

[[capabilities]]
id = "wasm-smoke.count"
description = "Count through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_TRAP_MANIFEST: &str = r#"
id = "wasm-smoke"
name = "WASM Trap"
version = "0.1.0"
description = "WASM runtime lane trap extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/trap.wasm"

[[capabilities]]
id = "wasm-smoke.trap"
description = "Trap through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
