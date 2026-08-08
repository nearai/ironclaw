use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_capabilities::{
    BoundCapabilityAdapter, ResolvedCapability, RuntimeAdapterResult, RuntimeDispatcher,
    ToolResolver,
};
use ironclaw_event_log::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extension_contracts::runtime::ExtensionRuntime;
use ironclaw_extension_registry::{
    CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
    HostApiContractRegistry, ManifestSource,
};
use ironclaw_filesystem::{DiskFilesystem, RootFilesystem};
use ironclaw_host_api::{
    Timestamp,
    action::{NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    authorized::Authorized,
    dispatch::{CapabilityDispatchRequest, DispatchError, RuntimeDispatchErrorKind},
    host_port::HostPortCatalog,
    http::{
        RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse,
    },
    ids::{
        ActivityId, AgentId, CapabilityId, CorrelationId, InvocationId, MissionId, ProductKind,
        ProjectId, ResourceReservationId, TenantId, ThreadId, UserId,
    },
    invocation::{Actor, Invocation, InvocationOrigin},
    lane::RuntimeLane,
    mount::MountView,
    path::{HostPath, VirtualPath},
    resource::{
        ReservationStatus, ResourceEstimate, ResourceReservation, ResourceScope, ResourceUsage,
    },
    runtime::RuntimeKind,
};
use ironclaw_resources::*;
use ironclaw_wasm::wasm_sandbox_core::SandboxLimits;
use ironclaw_wasm::{
    PreparedWitTool, WasmHostError, WasmRuntimeHttpAdapter, WitToolHost, WitToolRequest, WitToolRuntime,
    WitToolRuntimeConfig,
};
use serde_json::{Value, json};
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_loads_component_from_root_filesystem_and_uses_fresh_instances() {
    let component = tool_component(COUNTER_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
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

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_guest_trap_releases_reservation_and_preserves_dispatch_failure() {
    let component = tool_component(TRAP_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/trap.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_TRAP_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request("wasm-smoke.trap", json!({"call":"trap"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
            ..
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

#[tokio::test(flavor = "multi_thread")]
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
        saved_body: None,
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
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
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
            kind: RuntimeDispatchErrorKind::Guest,
            ..
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

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_missing_module_file_returns_sanitized_filesystem_error() {
    let fs = mounted_empty_extension_root();
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
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
            kind: RuntimeDispatchErrorKind::FilesystemDenied,
            ..
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

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_malformed_module_returns_sanitized_manifest_error() {
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", b"not wasm").await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
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
            kind: RuntimeDispatchErrorKind::Manifest,
            ..
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

#[tokio::test(flavor = "multi_thread")]
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
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
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
            kind: RuntimeDispatchErrorKind::OutputDecode,
            ..
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

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_rejects_unsupported_import_through_dispatcher_without_reservation_leak() {
    let raw_unsupported_import = wat::parse_str(UNSUPPORTED_IMPORT_MODULE_WAT).unwrap();
    let fs =
        filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", &raw_unsupported_import)
            .await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(WasmRuntimeAdapter::new());
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.count",
            json!({"sentinel":"UNSUPPORTED_IMPORT_SENTINEL_3067"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Manifest
                | RuntimeDispatchErrorKind::MethodMissing
                | RuntimeDispatchErrorKind::Executor,
            ..
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
    let serialized_events = serde_json::to_string(&events.events()).unwrap();
    assert!(!serialized_events.contains("UNSUPPORTED_IMPORT_SENTINEL_3067"));
}

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_enforces_memory_growth_budget_through_dispatcher() {
    let memory_growth = COUNTER_TOOL_WAT.replace(
        "global.get $count\n    i32.const 1\n    i32.add",
        "i32.const 1\n    memory.grow\n    i32.const -1\n    i32.eq\n    if\n      unreachable\n    end\n\n    global.get $count\n    i32.const 1\n    i32.add",
    );
    assert_ne!(memory_growth, COUNTER_TOOL_WAT);
    let component = tool_component(&memory_growth);
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/counter.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let adapter = WasmRuntimeAdapter::with_config(WitToolRuntimeConfig {
        default_limits: SandboxLimits::default()
            .with_memory_bytes(64 * 1024)
            .with_fuel(100_000)
            .with_timeout(Duration::from_secs(5)),
    });
    let adapter = Arc::new(adapter);
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.count",
            json!({"sentinel":"MEMORY_BOUND_SENTINEL_3067"}),
        ))
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Guest | RuntimeDispatchErrorKind::Memory,
                ..
            }
        ),
        "unexpected memory-growth-bound dispatch error: {err:?}"
    );
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
    let serialized_events = serde_json::to_string(&events.events()).unwrap();
    assert!(!serialized_events.contains("MEMORY_BOUND_SENTINEL_3067"));
}

#[tokio::test(flavor = "multi_thread")]
async fn wasm_lane_caps_overdue_host_import_at_dispatch_execution_deadline() {
    let component = tool_component(&trap_after_http_wat());
    let fs = filesystem_with_wasm_component("wasm-smoke", "wasm/http-trap.wasm", &component).await;
    let registry = Arc::new(registry_with_package(WASM_HTTP_TRAP_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();
    let http = Arc::new(SlowRuntimeEgress::new(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: Vec::new(),
        saved_body: None,
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
    let adapter = WasmRuntimeAdapter::with_host_and_config(
        WitToolHost::deny_all().with_http(wasm_http),
        WitToolRuntimeConfig {
            default_limits: SandboxLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_millis(20)),
        },
    );
    let adapter = Arc::new(adapter);
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-smoke.httptrap",
            json!({"sentinel":"DEADLINE_SENTINEL_3067"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
            ..
        }
    ));
    assert_eq!(http.requests.lock().unwrap().len(), 1);
    assert_eq!(
        governor.reserved_for(&sample_account()),
        ResourceTally::default()
    );
    assert_eq!(
        governor.usage_for(&sample_account()).network_egress_bytes,
        5,
        "overdue host import should preserve accountable request egress through dispatcher"
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
    let serialized_events = serde_json::to_string(&recorded).unwrap();
    assert!(!serialized_events.contains("DEADLINE_SENTINEL_3067"));
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

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

#[derive(Clone)]
struct SlowRuntimeEgress {
    response: RuntimeHttpEgressResponse,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl SlowRuntimeEgress {
    fn new(response: RuntimeHttpEgressResponse) -> Self {
        Self {
            response,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for SlowRuntimeEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        std::thread::sleep(Duration::from_millis(50));
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
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
        Self::with_host_and_config(host, WitToolRuntimeConfig::for_testing())
    }

    fn with_config(config: WitToolRuntimeConfig) -> Self {
        Self::with_host_and_config(WitToolHost::deny_all(), config)
    }

    fn with_host_and_config(host: WitToolHost, config: WitToolRuntimeConfig) -> Self {
        Self {
            runtime: WitToolRuntime::new(config).unwrap(),
            host,
            prepared: Mutex::new(HashMap::new()),
            prepare_count: AtomicUsize::new(0),
        }
    }

    fn prepare_count(&self) -> usize {
        self.prepare_count.load(Ordering::SeqCst)
    }
}

impl WasmRuntimeAdapter {
    async fn dispatch_lane(
        &self,
        request: LocalLaneRequest<'_>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let module_path = match &request.package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => ironclaw_extension_registry::resolve_asset_under(
                module,
                request
                    .package
                    .materialized_root()
                    .map_err(|_| DispatchError::Wasm {
                        kind: RuntimeDispatchErrorKind::Manifest,
                        model_visible_cause: None,
                    })?,
            )
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Manifest,
                model_visible_cause: None,
            })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                    model_visible_cause: None,
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
                model_visible_cause: None,
            })?;
        let prepared = Arc::new(
            self.runtime
                .prepare(request.capability_id.as_str(), &wasm_bytes)
                .map_err(|error| DispatchError::Wasm {
                    kind: wasm_error_kind(&error),
                    model_visible_cause: Some(error.to_string()),
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

/// The per-invocation slice of the old lane request: everything else is
/// captured by the prebound `RegistryBoundWasmCapability` at binding time.
struct LocalLaneRequest<'a> {
    package: &'a ExtensionPackage,
    capability_id: &'a CapabilityId,
    filesystem: &'a DiskFilesystem,
    governor: &'a InMemoryResourceGovernor,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    resource_reservation: Option<ResourceReservation>,
    input: Value,
}

/// Prebinds every registry capability to the file-local WASM lane adapter,
/// mirroring the production registry-lane resolver's shape at test scale.
fn dispatcher_for(
    registry: &ironclaw_extension_registry::ExtensionRegistry,
    filesystem: Arc<DiskFilesystem>,
    governor: Arc<InMemoryResourceGovernor>,
    adapter: &Arc<WasmRuntimeAdapter>,
) -> RuntimeDispatcher<'static, InMemoryResourceGovernor> {
    let bindings = registry
        .capabilities()
        .map(|descriptor| {
            let package = registry
                .get_extension(&descriptor.provider)
                .expect("registry package for descriptor");
            (
                descriptor.id.clone(),
                ResolvedCapability {
                    provider: descriptor.provider.clone(),
                    runtime: descriptor.runtime,
                    adapter: Arc::new(RegistryBoundWasmCapability {
                        package: Arc::new(package.clone()),
                        adapter: Arc::clone(adapter),
                        filesystem: Arc::clone(&filesystem),
                        governor: Arc::clone(&governor),
                    }) as Arc<dyn BoundCapabilityAdapter>,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let resolver: Arc<dyn ToolResolver> = Arc::new(MapResolver { bindings });
    RuntimeDispatcher::from_arcs(resolver, governor)
}

struct MapResolver {
    bindings: HashMap<CapabilityId, ResolvedCapability>,
}

impl ToolResolver for MapResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        self.bindings.get(capability_id).cloned()
    }
}

struct RegistryBoundWasmCapability {
    package: Arc<ExtensionPackage>,
    adapter: Arc<WasmRuntimeAdapter>,
    filesystem: Arc<DiskFilesystem>,
    governor: Arc<InMemoryResourceGovernor>,
}

#[async_trait]
impl BoundCapabilityAdapter for RegistryBoundWasmCapability {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.adapter
            .dispatch_lane(LocalLaneRequest {
                package: &self.package,
                capability_id: &request.capability_id,
                filesystem: self.filesystem.as_ref(),
                governor: self.governor.as_ref(),
                scope: request.scope,
                estimate: request.estimate,
                resource_reservation: request.resource_reservation,
                input: request.input,
            })
            .await
    }
}

fn execute_prepared_wasm(
    runtime: &WitToolRuntime,
    prepared: &PreparedWitTool,
    host: WitToolHost,
    request: LocalLaneRequest<'_>,
) -> Result<RuntimeAdapterResult, DispatchError> {
    let input_json = serde_json::to_string(&request.input).map_err(|_| DispatchError::Wasm {
        kind: RuntimeDispatchErrorKind::InputEncode,
        model_visible_cause: None,
    })?;
    let reservation = match request.resource_reservation {
        Some(reservation) => reservation,
        None => request
            .governor
            .reserve(request.scope, request.estimate)
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
                model_visible_cause: None,
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
                        model_visible_cause: None,
                    });
                }
            } else {
                release_wasm_reservation(request.governor, reservation.id);
            }
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
                model_visible_cause: None,
            });
        }
    };
    if execution.error.is_some() {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
            model_visible_cause: None,
        });
    }
    let Some(output_json) = execution.output_json else {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
            model_visible_cause: None,
        });
    };
    let output = match serde_json::from_str::<Value>(&output_json) {
        Ok(output) => output,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::OutputDecode,
                model_visible_cause: None,
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
                model_visible_cause: None,
            });
        }
    };
    Ok(RuntimeAdapterResult {
        output,
        display_preview: None,
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

fn registry_with_package(manifest: &str) -> ironclaw_extension_registry::ExtensionRegistry {
    let mut registry = ironclaw_extension_registry::ExtensionRegistry::new();
    registry.insert(package_from_manifest(manifest)).unwrap();
    registry
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(
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

fn mounted_empty_extension_root() -> DiskFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = DiskFilesystem::new();
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
) -> DiskFilesystem {
    let fs = mounted_empty_extension_root();
    let path =
        VirtualPath::new(format!("/system/extensions/{extension_id}/{module_path}")).unwrap();
    fs.write_file(&path, wasm_bytes).await.unwrap();
    fs
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits::default()
                .set_max_concurrency_slots(10)
                .set_max_process_count(10)
                .set_max_output_bytes(100_000),
        )
        .unwrap();
    governor
}

fn dispatch_request(capability: &str, input: Value) -> Authorized {
    let estimate = ResourceEstimate::default()
        .set_concurrency_slots(1)
        .set_process_count(1)
        .set_output_bytes(10_000);
    Authorized::seal_for_test(
        Invocation {
            activity_id: ActivityId::new(),
            capability: CapabilityId::new(capability).unwrap(),
            input,
            scope: sample_scope(),
            actor: Actor::System,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            estimate,
            correlation_id: CorrelationId::new(),
            process_id: None,
            parent_process_id: None,
        },
        RuntimeLane::Wasm,
        MountView::default(),
        None,
        Timestamp::MAX_UTC,
    )
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
        .push_str("tool.wit", ironclaw_wasm::TOOL_WIT)
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

const UNSUPPORTED_IMPORT_MODULE_WAT: &str = r#"
(module
  (import "near:agent/unsupported@0.4.0" "do-secret-thing" (func $unsupported))
  (func (export "run")
    call $unsupported)
)
"#;

const COUNTER_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32 i32)))
  (type (;7;) (func (param i32 i32 i32 i32 i32 i32)))
  (import "near:agent/host@0.4.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.4.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.4.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.4.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.4.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.4.0" "secret-exists" (func $secret_exists (type 4)))
  (import "near:agent/host@0.4.0" "nostr-sign-event" (func $nostr_sign_event (type 5)))
  (import "near:agent/host@0.4.0" "nostr-publish-event" (func $nostr_publish_event (type 6)))
  (import "near:agent/host@0.4.0" "nostr-subscribe-events" (func $nostr_subscribe_events (type 7)))
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
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
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
  (import "near:agent/host@0.4.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.4.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.4.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.4.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.4.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.4.0" "secret-exists" (func $secret_exists (type 4)))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32 i32)))
  (type (;7;) (func (param i32 i32 i32 i32 i32 i32)))
  (import "near:agent/host@0.4.0" "nostr-sign-event" (func $nostr_sign_event (type 5)))
  (import "near:agent/host@0.4.0" "nostr-publish-event" (func $nostr_publish_event (type 6)))
  (import "near:agent/host@0.4.0" "nostr-subscribe-events" (func $nostr_subscribe_events (type 7)))
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
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
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
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const WASM_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-smoke"
name = "WASM Smoke"
version = "0.1.0"
description = "WASM runtime lane smoke extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/counter.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-smoke.count"
description = "Count through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-smoke/count.input.v1.json"
output_schema_ref = "schemas/wasm-smoke/count.output.v1.json"
"#;

const WASM_TRAP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-smoke"
name = "WASM Trap"
version = "0.1.0"
description = "WASM runtime lane trap extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/trap.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-smoke.trap"
description = "Trap through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-smoke/trap.input.v1.json"
output_schema_ref = "schemas/wasm-smoke/trap.output.v1.json"
"#;

const WASM_HTTP_TRAP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-smoke"
name = "WASM HTTP Trap"
version = "0.1.0"
description = "WASM runtime lane HTTP trap extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/http-trap.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-smoke.httptrap"
description = "Trap after host HTTP through WASM"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-smoke/httptrap.input.v1.json"
output_schema_ref = "schemas/wasm-smoke/httptrap.output.v1.json"
"#;

// ---------------------------------------------------------------------------
// Nostr host-function integration tests
// ---------------------------------------------------------------------------

/// WAT fixture: copy of counter that also calls `nostr-sign-event` in execute.
/// Uses identical structure to COUNTER_TOOL_WAT (known to work) with minimal changes.
const NOSTR_SIGN_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32 i32)))
  (type (;7;) (func (param i32 i32 i32 i32 i32 i32)))
  (import "near:agent/host@0.4.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.4.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.4.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.4.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.4.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.4.0" "secret-exists" (func $secret_exists (type 4)))
  (import "near:agent/host@0.4.0" "nostr-sign-event" (func $nostr_sign_event (type 5)))
  (import "near:agent/host@0.4.0" "nostr-publish-event" (func $nostr_publish_event (type 6)))
  (import "near:agent/host@0.4.0" "nostr-subscribe-events" (func $nostr_subscribe_events (type 7)))
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "nostr sign tool")
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
  ;; execute: call nostr-sign-event with params, then return counter value.
  ;; The params_ptr/len are the first two args. We call nostr-sign-event,
  ;; ignore the result, then return the counter like the original.
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    ;; Allocate a large output buffer for result<string, string>:
    ;; The host writes {disc: i32, ptr: i32, len: i32} plus string content.
    global.get $count
    i32.const 256
    i32.add
    global.set $count   ;; bump heap
    local.get 0         ;; params_ptr
    local.get 1         ;; params_len
    global.get $count   ;; pass heap-256 as result_buf
    i32.const 256
    i32.sub
    call $nostr_sign_event  ;; sign-event(params_ptr, params_len, result_buf)

    ;; Now build response (same as counter — just return the count)
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
  ;; Bump allocator: use $count as bump pointer (initialized to 0 by global $count)
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    local.get $new_size
    i32.const 7
    i32.add
    i32.const -8
    i32.and          ;; align up to 8
    local.set $new_size
    global.get $count  ;; pre-bump value = allocation start
    global.get $count
    local.get $new_size
    i32.add
    global.set $count)
  (func $_initialize)
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const NOSTR_SIGN_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-nostr-test"
name = "WASM Nostr Test"
version = "0.1.0"
description = "Nostr sign integration test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/nostr_sign.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-nostr-test.sign"
description = "Sign a Nostr event through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-nostr-test/sign.input.v1.json"
output_schema_ref = "schemas/wasm-nostr-test/sign.output.v1.json"
"#;

/// Mock Nostr host that records calls and returns canned responses.
struct MockWasmHostNostr {
    sign_result: Mutex<Option<Result<String, WasmHostError>>>,
    publish_result: Mutex<Option<Result<String, WasmHostError>>>,
    subscribe_result: Mutex<Option<Result<String, WasmHostError>>>,
    sign_calls: Mutex<Vec<String>>,
    publish_calls: Mutex<Vec<(String, String)>>,
    subscribe_calls: Mutex<Vec<(String, String, u32)>>,
}

impl MockWasmHostNostr {
    fn new() -> Self {
        Self {
            sign_result: Mutex::new(None),
            publish_result: Mutex::new(None),
            subscribe_result: Mutex::new(None),
            sign_calls: Mutex::new(Vec::new()),
            publish_calls: Mutex::new(Vec::new()),
            subscribe_calls: Mutex::new(Vec::new()),
        }
    }

    fn with_sign_ok(signed_json: &str) -> Self {
        let mock = Self::new();
        *mock.sign_result.lock().unwrap() = Some(Ok(signed_json.to_string()));
        mock
    }
}

impl ironclaw_wasm::WasmHostNostr for MockWasmHostNostr {
    fn sign_event(&self, unsigned_event_json: &str) -> Result<String, WasmHostError> {
        self.sign_calls.lock().unwrap().push(unsigned_event_json.to_string());
        // Return custom result if set, otherwise a default signed event
        // Use try_lock to avoid poisoning; clone so we don't consume
        let result = self.sign_result.lock().unwrap();
        if let Some(ref val) = *result {
            val.clone()
        } else {
            Ok(r#"{"id":"test-id","pubkey":"test-pk","sig":"test-sig"}"#.to_string())
        }
    }

    fn publish_event(
        &self,
        relay_url: &str,
        signed_event_json: &str,
        _remaining_deadline_ms: Option<u32>,
    ) -> Result<String, WasmHostError> {
        self.publish_calls.lock().unwrap().push((relay_url.to_string(), signed_event_json.to_string()));
        let result = self.publish_result.lock().unwrap();
        if let Some(ref val) = *result {
            val.clone()
        } else {
            Ok("published-event-id".to_string())
        }
    }

    fn subscribe_events(
        &self,
        relay_url: &str,
        filter_json: &str,
        timeout_ms: u32,
        _remaining_deadline_ms: Option<u32>,
    ) -> Result<String, WasmHostError> {
        self.subscribe_calls.lock().unwrap().push((relay_url.to_string(), filter_json.to_string(), timeout_ms));
        let result = self.subscribe_result.lock().unwrap();
        if let Some(ref val) = *result {
            val.clone()
        } else {
            Ok(r#"{"events":[],"truncated":false}"#.to_string())
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn wasm_nostr_sign_event_flows_through_host_pipeline() {
    let signed_event = json!({
        "id": "abc123",
        "pubkey": "test-pubkey",
        "created_at": 1690000000,
        "kind": 1,
        "tags": [],
        "content": "hello from nostr",
        "sig": "deadbeef1234"
    });
    let unsigned_event = json!({
        "pubkey": "test-pubkey",
        "created_at": 1690000000,
        "kind": 1,
        "tags": [],
        "content": "hello from nostr"
    });

    let component = tool_component(NOSTR_SIGN_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-nostr-test", "wasm/nostr_sign.wasm", &component).await;
    let registry = Arc::new(registry_with_package(NOSTR_SIGN_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();

    let mock_nostr = Arc::new(MockWasmHostNostr::with_sign_ok(&signed_event.to_string()));
    let adapter = Arc::new(WasmRuntimeAdapter::with_host(
        WitToolHost::deny_all().with_nostr(Arc::clone(&mock_nostr)),
    ));
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let result = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-nostr-test.sign",
            json!({"params": unsigned_event.to_string()}),
        ))
        .await
        .expect("dispatch should succeed");

    assert_eq!(result.runtime, RuntimeKind::Wasm);
    // The tool calls nostr-sign-event but returns a fixed "1" counter output.
    // Verify the mock was called with the unsigned event JSON.
    let calls = mock_nostr.sign_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "expected exactly 1 nostr-sign-event call, got {}", calls.len());
    assert!(calls[0].contains("hello from nostr"), "expected content in nostr input, got: {}", calls[0]);

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

// ---------------------------------------------------------------------------
// Nostr publish-event WAT integration test
// ---------------------------------------------------------------------------

const NOSTR_PUBLISH_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32 i32)))
  (type (;7;) (func (param i32 i32 i32 i32 i32 i32)))
  (import "near:agent/host@0.4.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.4.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.4.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.4.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.4.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.4.0" "secret-exists" (func $secret_exists (type 4)))
  (import "near:agent/host@0.4.0" "nostr-sign-event" (func $nostr_sign_event (type 5)))
  (import "near:agent/host@0.4.0" "nostr-publish-event" (func $nostr_publish_event (type 6)))
  (import "near:agent/host@0.4.0" "nostr-subscribe-events" (func $nostr_subscribe_events (type 7)))
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "nostr publish tool")
  (data (i32.const 3072) "1")
  (data (i32.const 4096) "wss://relay.example.com")
  (data (i32.const 5120) "{\22id\22:\22publish-test\22,\22sig\22:\22test\22}")
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
    i32.const 18
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    ;; Allocate output buffer (256 bytes) for result<string, string>
    global.get $count
    i32.const 256
    i32.add
    global.set $count

    ;; Call nostr-publish-event(relay_ptr, relay_len, event_ptr, event_len, out_ptr)
    i32.const 4096          ;; relay_ptr
    i32.const 23            ;; relay_len = len("wss://relay.example.com")
    i32.const 5120          ;; event_ptr
    i32.const 34            ;; event_len = len({"id":"publish-test","sig":"test"})
    global.get $count
    i32.const 256
    i32.sub                ;; out_ptr
    call $nostr_publish_event

    ;; Build response: output="1", error=null
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
  ;; Bump allocator: use $count as bump pointer
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    local.get $new_size
    i32.const 7
    i32.add
    i32.const -8
    i32.and          ;; align up to 8
    local.set $new_size
    global.get $count  ;; pre-bump value = allocation start
    global.get $count
    local.get $new_size
    i32.add
    global.set $count)
  (func $_initialize)
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const NOSTR_PUBLISH_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-nostr-pub-test"
name = "WASM Nostr Publish Test"
version = "0.1.0"
description = "Nostr publish integration test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/nostr_publish.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-nostr-pub-test.publish"
description = "Publish a Nostr event through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-nostr-pub-test/publish.input.v1.json"
output_schema_ref = "schemas/wasm-nostr-pub-test/publish.output.v1.json"
"#;

#[tokio::test(flavor = "multi_thread")]
async fn wasm_nostr_publish_event_flows_through_host_pipeline() {
    let component = tool_component(NOSTR_PUBLISH_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-nostr-pub-test", "wasm/nostr_publish.wasm", &component).await;
    let registry = Arc::new(registry_with_package(NOSTR_PUBLISH_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();

    let mock_nostr = Arc::new(MockWasmHostNostr::new());
    let adapter = Arc::new(WasmRuntimeAdapter::with_host(
        WitToolHost::deny_all().with_nostr(Arc::clone(&mock_nostr)),
    ));
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let result = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-nostr-pub-test.publish",
            json!({}),
        ))
        .await
        .expect("dispatch should succeed");

    assert_eq!(result.runtime, RuntimeKind::Wasm);

    let calls = mock_nostr.publish_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "expected exactly 1 nostr-publish-event call, got {}", calls.len());
    assert_eq!(calls[0].0, "wss://relay.example.com", "relay URL mismatch");
    assert!(calls[0].1.contains("publish-test"), "expected publish-test in event JSON, got: {}", calls[0].1);

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

// ---------------------------------------------------------------------------
// Nostr subscribe-events WAT integration test
// ---------------------------------------------------------------------------

const NOSTR_SUBSCRIBE_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32 i32)))
  (type (;7;) (func (param i32 i32 i32 i32 i32 i32)))
  (import "near:agent/host@0.4.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.4.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.4.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.4.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.4.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.4.0" "secret-exists" (func $secret_exists (type 4)))
  (import "near:agent/host@0.4.0" "nostr-sign-event" (func $nostr_sign_event (type 5)))
  (import "near:agent/host@0.4.0" "nostr-publish-event" (func $nostr_publish_event (type 6)))
  (import "near:agent/host@0.4.0" "nostr-subscribe-events" (func $nostr_subscribe_events (type 7)))
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "nostr subscribe tool")
  (data (i32.const 3072) "1")
  (data (i32.const 4096) "wss://relay.example.com")
  (data (i32.const 5120) "{\22kinds\22:[1]}")
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
    i32.const 20
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    ;; Allocate output buffer (256 bytes) for result<string, string>
    global.get $count
    i32.const 256
    i32.add
    global.set $count

    ;; Call nostr-subscribe-events(relay_ptr, relay_len, filter_ptr, filter_len, timeout_ms, out_ptr)
    i32.const 4096          ;; relay_ptr
    i32.const 23            ;; relay_len = len("wss://relay.example.com")
    i32.const 5120          ;; filter_ptr
    i32.const 13            ;; filter_len = len({"kinds":[1]})
    i32.const 3000          ;; timeout_ms
    global.get $count
    i32.const 256
    i32.sub                ;; out_ptr
    call $nostr_subscribe_events

    ;; Build response: output="1", error=null
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
  ;; Bump allocator: use $count as bump pointer
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    local.get $new_size
    i32.const 7
    i32.add
    i32.const -8
    i32.and          ;; align up to 8
    local.set $new_size
    global.get $count  ;; pre-bump value = allocation start
    global.get $count
    local.get $new_size
    i32.add
    global.set $count)
  (func $_initialize)
  (export "near:agent/tool@0.4.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.4.0#execute" (func $post))
  (export "near:agent/tool@0.4.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.4.0#schema" (func $post))
  (export "near:agent/tool@0.4.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.4.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const NOSTR_SUBSCRIBE_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "wasm-nostr-sub-test"
name = "WASM Nostr Subscribe Test"
version = "0.1.0"
description = "Nostr subscribe integration test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/nostr_subscribe.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "wasm-nostr-sub-test.subscribe"
description = "Subscribe to Nostr events through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/wasm-nostr-sub-test/subscribe.input.v1.json"
output_schema_ref = "schemas/wasm-nostr-sub-test/subscribe.output.v1.json"
"#;

#[tokio::test(flavor = "multi_thread")]
async fn wasm_nostr_subscribe_events_flows_through_host_pipeline() {
    let component = tool_component(NOSTR_SUBSCRIBE_TOOL_WAT);
    let fs = filesystem_with_wasm_component("wasm-nostr-sub-test", "wasm/nostr_subscribe.wasm", &component).await;
    let registry = Arc::new(registry_with_package(NOSTR_SUBSCRIBE_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();

    let mock_nostr = Arc::new(MockWasmHostNostr::new());
    let adapter = Arc::new(WasmRuntimeAdapter::with_host(
        WitToolHost::deny_all().with_nostr(Arc::clone(&mock_nostr)),
    ));
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let result = dispatcher
        .dispatch_json(dispatch_request(
            "wasm-nostr-sub-test.subscribe",
            json!({}),
        ))
        .await
        .expect("dispatch should succeed");

    assert_eq!(result.runtime, RuntimeKind::Wasm);

    let calls = mock_nostr.subscribe_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "expected exactly 1 nostr-subscribe-events call, got {}", calls.len());
    assert_eq!(calls[0].0, "wss://relay.example.com", "relay URL mismatch");
    assert!(calls[0].1.contains("kinds"), "expected filter to contain kinds, got: {}", calls[0].1);
    assert_eq!(calls[0].2, 3000, "timeout_ms mismatch");

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

const BUZZ_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "buzz"
name = "Buzz"
version = "0.2.3"
description = "Nostr Buzz channel tool"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/buzz_tool.component.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "buzz.subscribe"
description = "Subscribe to a Buzz channel"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/buzz/subscribe.input.v1.json"
output_schema_ref = "schemas/buzz/subscribe.output.v1.json"

[[capability_provider.tools.capabilities]]
id = "buzz.send_message"
description = "Send a message to a Buzz channel"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/buzz/send_message.input.v1.json"
output_schema_ref = "schemas/buzz/send_message.output.v1.json"
"#;

/// Load the pre-built Buzz component from the tools-src tree.
/// Only available after `cargo build --release --target wasm32-unknown-unknown` in
/// `tools-src/buzz`. Tests that depend on it are gated by a file-existence check so
/// they pass on fresh checkouts without the build artifact.
fn buzz_component_path() -> Option<std::path::PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let path = std::path::Path::new(&manifest_dir)
        .join("../../tools-src/buzz/target/wasm32-unknown-unknown/release/buzz_tool.component.wasm");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn buzz_subscribe_channel_dispatches_through_host_pipeline() {
    let Some(buzz_path) = buzz_component_path() else {
        eprintln!("Skipping: Buzz component not found. Build with: cargo build --release --target wasm32-unknown-unknown -p buzz-tool (in tools-src/buzz)");
        return;
    };
    let buzz_bytes = std::fs::read(&buzz_path).unwrap();
    let fs = filesystem_with_wasm_component("buzz", "wasm/buzz_tool.component.wasm", &buzz_bytes).await;
    let registry = Arc::new(registry_with_package(BUZZ_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();

    // Mock nostr: subscribe returns empty events list
    let mock_nostr = Arc::new(MockWasmHostNostr::new());
    *mock_nostr.subscribe_result.lock().unwrap() = Some(Ok(
        serde_json::json!({"events": [], "truncated": false}).to_string(),
    ));

    let adapter = Arc::new(WasmRuntimeAdapter::with_host_and_config(
        WitToolHost::deny_all().with_nostr(Arc::clone(&mock_nostr)),
        WitToolRuntimeConfig::for_testing_with_memory(2 * 1024 * 1024),
    ));
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let subscribe_params = serde_json::json!({
        "action": "subscribe_channel",
        "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
        "relay_url": "wss://relay.example.com",
        "timeout_ms": 2000,
        "limit": 10
    });

    let result = dispatcher
        .dispatch_json(dispatch_request(
            "buzz.subscribe",
            subscribe_params.clone(),
        ))
        .await
        .expect("dispatch should succeed");

    assert_eq!(result.runtime, RuntimeKind::Wasm);

    // Verify the mock nostr was called with correct relay URL and filter
    let subscribe_calls = mock_nostr.subscribe_calls.lock().unwrap();
    assert_eq!(subscribe_calls.len(), 1, "expected exactly 1 subscribe call, got {}", subscribe_calls.len());
    let (relay_url, filter_json, timeout_ms) = &subscribe_calls[0];
    assert_eq!(relay_url, "wss://relay.example.com");
    assert!(filter_json.contains("8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f"), "filter should contain channel_id");
    assert_eq!(*timeout_ms, 2000);

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn buzz_send_message_dispatches_sign_and_publish() {
    let Some(buzz_path) = buzz_component_path() else {
        eprintln!("Skipping: Buzz component not found. Build with: cargo build --release --target wasm32-unknown-unknown -p buzz-tool (in tools-src/buzz)");
        return;
    };
    let buzz_bytes = std::fs::read(&buzz_path).unwrap();
    let fs = filesystem_with_wasm_component("buzz", "wasm/buzz_tool.component.wasm", &buzz_bytes).await;
    let registry = Arc::new(registry_with_package(BUZZ_MANIFEST));
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let events = InMemoryEventSink::new();

    let signed_event = json!({
        "id": "event-123",
        "pubkey": "testpub",
        "created_at": 1690000000,
        "kind": 1,
        "tags": [],
        "content": "hello buzz",
        "sig": "testsig"
    });

    let mock_nostr = Arc::new(MockWasmHostNostr::new());
    // Sign returns the signed event; publish returns success
    *mock_nostr.sign_result.lock().unwrap() = Some(Ok(signed_event.to_string()));
    *mock_nostr.publish_result.lock().unwrap() = Some(Ok("event-123".to_string()));

    let adapter = Arc::new(WasmRuntimeAdapter::with_host_and_config(
        WitToolHost::deny_all().with_nostr(Arc::clone(&mock_nostr)),
        WitToolRuntimeConfig::for_testing_with_memory(2 * 1024 * 1024),
    ));
    let dispatcher = dispatcher_for(&registry, Arc::new(fs), Arc::clone(&governor), &adapter)
        .with_event_sink_arc(Arc::new(events.clone()));

    let send_params = serde_json::json!({
        "action": "send_message",
        "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
        "content": "hello buzz",
        "relay_url": "wss://relay.example.com"
    });

    let result = dispatcher
        .dispatch_json(dispatch_request(
            "buzz.send_message",
            send_params.clone(),
        ))
        .await
        .expect("dispatch should succeed");

    assert_eq!(result.runtime, RuntimeKind::Wasm);

    // Buzz signs twice per send: once to probe host availability, once for the real event.
    let sign_calls = mock_nostr.sign_calls.lock().unwrap();
    assert_eq!(sign_calls.len(), 2, "expected exactly 2 sign calls (probe + real), got {:?}", sign_calls);

    // Publish is called once with the signed event
    let publish_calls = mock_nostr.publish_calls.lock().unwrap();
    assert_eq!(publish_calls.len(), 1, "expected exactly 1 publish call, got {}", publish_calls.len());
    assert_eq!(publish_calls[0].0, "wss://relay.example.com");
    // Buzz publishes the signed event — the content should be present
    let published = &publish_calls[0].1;
    assert!(published.contains("hello buzz") || published.contains("\"content\""),
        "published event should contain content, got: {published}");

    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}
