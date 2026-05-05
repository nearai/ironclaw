use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRuntime};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::{
    PreparedWitTool, WasmRuntimeHttpAdapter, WitToolHost, WitToolRequest, WitToolRuntime,
};
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
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::clone(&adapter))
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
        adapter.prepare_count(),
        1,
        "dispatcher smoke should reuse one prepared component while proving fresh execution instances"
    );
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

#[tokio::test]
async fn wasm_lane_execution_failure_reconciles_preserved_usage_from_runtime() {
    let component = tool_component(&trap_after_http_wat());
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/http-trap.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_HTTP_TRAP_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let http = Arc::new(RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: Vec::new(),
        request_bytes: 5,
        response_bytes: 0,
        redaction_applied: false,
    }));
    let wasm_http = Arc::new(
        WasmRuntimeHttpAdapter::new(
            Arc::clone(&http),
            sample_scope(),
            CapabilityId::new("wasm-smoke.httptrap").unwrap(),
            wasm_http_policy(),
        )
        .with_response_body_limit(Some(4096)),
    );
    let adapter = Arc::new(WasmRuntimeAdapter::with_host(
        WitToolHost::deny_all().with_http(wasm_http),
    ));
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.httptrap",
            json!({"call":"http"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest
        }
    ));
    let http_requests = http.requests.lock().unwrap();
    assert_eq!(http_requests.len(), 1);
    assert_eq!(http_requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(http_requests[0].method, NetworkMethod::Post);
    assert_eq!(http_requests[0].url, "https://example.test/api");
    assert_eq!(http_requests[0].body, b"hello");
    assert_eq!(http_requests[0].response_body_limit, Some(4096));
    assert_eq!(
        governor.reserved_for(&sample_account()),
        ResourceTally::default()
    );
    assert_eq!(
        governor.usage_for(&sample_account()).network_egress_bytes,
        5,
        "request-body egress preserved by WasmError::ExecutionFailed must be reconciled"
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

#[tokio::test]
async fn wasm_lane_missing_module_file_returns_sanitized_filesystem_error() {
    let fs = mounted_empty_extension_root();
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::clone(&adapter))
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.count",
            json!({"call": "missing"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::FilesystemDenied
        }
    ));
    assert_eq!(adapter.prepare_count(), 0);
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
    assert_eq!(recorded[2].error_kind.as_deref(), Some("filesystem_denied"));
}

#[tokio::test]
async fn wasm_lane_malformed_module_returns_sanitized_manifest_error() {
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", b"not wasm").await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::new(WasmRuntimeAdapter::new()))
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.count",
            json!({"call": "malformed"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Manifest
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
    assert_eq!(recorded[2].error_kind.as_deref(), Some("manifest"));
}

#[tokio::test]
async fn wasm_lane_invalid_output_json_returns_sanitized_output_error() {
    let invalid_output_wat = COUNTER_TOOL_WAT
        .replace(
            r#"(data (i32.const 3072) "1")"#,
            r#"(data (i32.const 3072) "not-json")"#,
        )
        .replace(
            "i32.const 56\n    i32.const 1\n    i32.store",
            "i32.const 56\n    i32.const 8\n    i32.store",
        );
    assert_ne!(
        invalid_output_wat, COUNTER_TOOL_WAT,
        "invalid output WAT mutation should match the fixture"
    );
    let component = tool_component(&invalid_output_wat);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(registry, Arc::new(fs), Arc::clone(&governor))
        .with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::new(WasmRuntimeAdapter::new()))
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.count",
            json!({"call": "invalid-output"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::OutputDecode
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
    assert_eq!(recorded[2].error_kind.as_deref(), Some("output_decode"));
}

#[derive(Clone)]
struct RecordingRuntimeEgress {
    response: Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeEgress {
    fn ok(response: RuntimeHttpEgressResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl RuntimeHttpEgress for RecordingRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

struct WasmRuntimeAdapter {
    runtime: WitToolRuntime,
    host: WitToolHost,
    prepared: Mutex<HashMap<String, Arc<PreparedWitTool>>>,
    prepare_count: AtomicUsize,
}

impl WasmRuntimeAdapter {
    fn new() -> Self {
        Self::with_host(WitToolHost::deny_all())
    }

    fn with_host(host: WitToolHost) -> Self {
        Self {
            runtime: WitToolRuntime::new(ironclaw_wasm::WitToolRuntimeConfig::for_testing())
                .unwrap(),
            host,
            prepared: Mutex::new(HashMap::new()),
            prepare_count: AtomicUsize::new(0),
        }
    }

    fn prepare_count(&self) -> usize {
        self.prepare_count.load(Ordering::SeqCst)
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
        let cache_key = format!(
            "{}:{}",
            request.capability_id.as_str(),
            module_path.as_str()
        );
        if let Some(prepared) = self.prepared.lock().unwrap().get(&cache_key).cloned() {
            return execute_prepared_wasm(&self.runtime, &prepared, self.host.clone(), request);
        }

        let wasm_bytes = request
            .filesystem
            .read_file(&module_path)
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
            })?;
        let prepared = Arc::new(
            self.runtime
                .prepare(request.capability_id.as_str(), &wasm_bytes)
                .map_err(|error| DispatchError::Wasm {
                    kind: wasm_error_kind(&error),
                })?,
        );
        let prepared = {
            let mut prepared_cache = self.prepared.lock().unwrap();
            if let Some(existing) = prepared_cache.get(&cache_key).cloned() {
                existing
            } else {
                self.prepare_count.fetch_add(1, Ordering::SeqCst);
                prepared_cache.insert(cache_key, Arc::clone(&prepared));
                prepared
            }
        };
        execute_prepared_wasm(&self.runtime, &prepared, self.host.clone(), request)
    }
}

fn execute_prepared_wasm(
    runtime: &WitToolRuntime,
    prepared: &PreparedWitTool,
    host: WitToolHost,
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
    let execution = match runtime.execute(prepared, host, WitToolRequest::new(input_json)) {
        Ok(execution) => execution,
        Err(error) => {
            if let Some(usage) = preserved_wasm_error_usage(&error) {
                if request.governor.reconcile(reservation.id, usage).is_err() {
                    release_wasm_reservation(request.governor, reservation.id);
                    return Err(DispatchError::Wasm {
                        kind: RuntimeDispatchErrorKind::Resource,
                    });
                }
            } else {
                release_wasm_reservation(request.governor, reservation.id);
            }
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

fn preserved_wasm_error_usage(error: &ironclaw_wasm::WasmError) -> Option<ResourceUsage> {
    if let ironclaw_wasm::WasmError::ExecutionFailed { usage, .. } = error
        && has_accountable_effects(usage)
    {
        Some(usage.clone())
    } else {
        None
    }
}

fn has_accountable_effects(usage: &ResourceUsage) -> bool {
    usage.usd != Default::default()
        || usage.input_tokens > 0
        || usage.output_tokens > 0
        || usage.output_bytes > 0
        || usage.network_egress_bytes > 0
        || usage.process_count > 0
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

fn wasm_http_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
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

const HTTP_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 128) "POST")
  (data (i32.const 160) "https://example.test/api")
  (data (i32.const 224) "{}")
  (data (i32.const 256) "hello")
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "fixture description")
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
    i32.const 19
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 128
    i32.const 4
    i32.const 160
    i32.const 24
    i32.const 224
    i32.const 2
    i32.const 1
    i32.const 256
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 512
    call $http_request

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
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
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

fn trap_after_http_wat() -> String {
    HTTP_TOOL_WAT.replace(
        "i32.const 48\n    i32.const 1\n    i32.store",
        "unreachable\n\n    i32.const 48\n    i32.const 1\n    i32.store",
    )
}

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

const WASM_HTTP_TRAP_MANIFEST: &str = r#"
id = "wasm-smoke"
name = "WASM HTTP Trap"
version = "0.1.0"
description = "WASM runtime lane HTTP trap extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/http-trap.wasm"

[[capabilities]]
id = "wasm-smoke.httptrap"
description = "Trap after host HTTP through WASM"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
