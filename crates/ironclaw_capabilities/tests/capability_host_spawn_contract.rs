use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_dispatcher::*;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::*;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_resources::{
    InMemoryResourceGovernor, ReservationStatus, ResourceAccount, ResourceGovernor, ResourceLimits,
    ResourceReceipt,
};
use ironclaw_wasm::WasmRuntime;
use serde_json::json;

#[tokio::test]
async fn capability_host_spawns_authorized_capability_process() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let processes = InMemoryProcessStore::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_process_manager(&processes);

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "run in background"}),
        })
        .await
        .unwrap();

    assert_eq!(result.process.invocation_id, invocation_id);
    assert_eq!(result.process.scope, scope);
    assert_eq!(
        result.process.extension_id,
        ExtensionId::new("echo").unwrap()
    );
    assert_eq!(
        result.process.capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(result.process.runtime, RuntimeKind::Wasm);
    assert_eq!(result.process.status, ProcessStatus::Running);
    assert_eq!(result.process.parent_process_id, None);
    assert_eq!(result.process.grants.grants.len(), 1);
    assert_eq!(
        processes
            .get(&scope, result.process.process_id)
            .await
            .unwrap()
            .unwrap(),
        result.process
    );
}

#[tokio::test]
async fn capability_host_passes_spawn_input_to_process_manager_without_storing_it_in_record() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let input = json!({"message": "runtime payload"});
    let process_manager = InputAssertingProcessManager {
        expected_input: input.clone(),
    };
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager);

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input,
        })
        .await
        .unwrap();

    let serialized_record = serde_json::to_value(&result.process).unwrap();
    assert!(serialized_record.get("input").is_none());
}

#[tokio::test]
async fn capability_host_denies_spawn_without_spawn_process_effect() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let processes = InMemoryProcessStore::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let scope = context.resource_scope.clone();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_process_manager(&processes);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "not enough authority"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::PolicyDenied,
            ..
        }
    ));
    assert_eq!(
        processes.records_for_scope(&scope).await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn capability_host_rejects_spawn_invalid_context_before_process_persistence() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let processes = InMemoryProcessStore::new();
    let mut context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let original_scope = context.resource_scope.clone();
    context.user_id = UserId::new("other-user").unwrap();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_process_manager(&processes);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "invalid"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::InternalInvariantViolation,
            ..
        }
    ));
    assert_eq!(
        processes.records_for_scope(&original_scope).await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn dispatch_process_executor_routes_process_request_to_capability_dispatcher() {
    let dispatcher = Arc::new(RecordingSpawnDispatcher::default());
    let executor = DispatchProcessExecutor::new(dispatcher.clone());
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);

    let result = executor
        .execute(ProcessExecutionRequest {
            process_id: ProcessId::new(),
            invocation_id,
            scope: scope.clone(),
            extension_id: ExtensionId::new("echo").unwrap(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background dispatch"}),
            cancellation: ProcessCancellationToken::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "background dispatch"}));
    let request = dispatcher.take_request();
    assert_eq!(
        request.capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(request.scope, scope);
    assert_eq!(request.input, json!({"message": "background dispatch"}));
}

#[tokio::test]
async fn capability_host_spawn_can_run_background_dispatch_process() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = Arc::new(RecordingSpawnDispatcher::default());
    let executor = Arc::new(DispatchProcessExecutor::new(dispatcher.clone()));
    let process_store = Arc::new(InMemoryProcessStore::new());
    let process_manager = BackgroundProcessManager::new(process_store.clone(), executor);
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let invocation_id = context.resource_scope.invocation_id;
    let scope = context.resource_scope.clone();
    let host = CapabilityHost::new(&registry, dispatcher.as_ref(), &authorizer)
        .with_process_manager(&process_manager);

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background dispatch"}),
        })
        .await
        .unwrap();

    assert_eq!(spawned.process.invocation_id, invocation_id);
    wait_for_process_status(
        process_store.as_ref(),
        &scope,
        spawned.process.process_id,
        ProcessStatus::Completed,
    )
    .await;
    let request = dispatcher.take_request();
    assert_eq!(
        request.capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(request.scope, scope);
    assert_eq!(request.input, json!({"message": "background dispatch"}));
}

#[tokio::test]
async fn capability_host_spawn_records_process_resource_reservation() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let process_store =
        ResourceManagedProcessStore::new(InMemoryProcessStore::new(), governor.clone());
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_store);

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                process_count: Some(1),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "run in background"}),
        })
        .await
        .unwrap();

    assert!(result.process.resource_reservation_id.is_some());
    let reserved = governor.reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone()));
    assert_eq!(reserved.process_count, 1);
    assert_eq!(reserved.concurrency_slots, 1);
}

#[tokio::test]
async fn capability_host_spawn_emits_scoped_process_events_without_raw_input() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = Arc::new(RecordingSpawnDispatcher::default());
    let executor = Arc::new(DispatchProcessExecutor::new(dispatcher.clone()));
    let events = Arc::new(InMemoryEventSink::new());
    let process_store = Arc::new(EventingProcessStore::new(
        InMemoryProcessStore::new(),
        events.clone(),
    ));
    let process_manager = BackgroundProcessManager::new(process_store.clone(), executor);
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let secret_input = json!({"message": "background dispatch", "token": "do-not-log"});
    let host = CapabilityHost::new(&registry, dispatcher.as_ref(), &authorizer)
        .with_process_manager(&process_manager);

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: secret_input,
        })
        .await
        .unwrap();

    wait_for_process_status(
        process_store.as_ref(),
        &scope,
        spawned.process.process_id,
        ProcessStatus::Completed,
    )
    .await;
    wait_for_event_count(events.as_ref(), 2).await;
    let emitted = events.events();
    assert_eq!(emitted[0].kind, RuntimeEventKind::ProcessStarted);
    assert_eq!(emitted[1].kind, RuntimeEventKind::ProcessCompleted);
    for event in emitted {
        assert_eq!(event.scope, scope);
        assert_eq!(event.process_id, Some(spawned.process.process_id));
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains("do-not-log"));
        assert!(!serialized.contains("background dispatch"));
    }
}

#[tokio::test]
async fn capability_host_spawn_can_run_background_runtime_dispatcher_process() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let registry = Arc::new(registry);
    let fs = Arc::new(fs);
    let governor = Arc::new(InMemoryResourceGovernor::new());
    governor.set_limit(
        ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
        ResourceLimits {
            max_process_count: Some(1),
            ..ResourceLimits::default()
        },
    );
    let wasm_runtime = Arc::new(WasmRuntime::for_testing().unwrap());
    let dispatcher = Arc::new(
        RuntimeDispatcher::from_arcs(registry.clone(), fs, governor.clone())
            .with_wasm_runtime_arc(wasm_runtime),
    );
    let executor = Arc::new(DispatchProcessExecutor::new(dispatcher.clone()));
    let process_store = Arc::new(ResourceManagedProcessStore::new(
        InMemoryProcessStore::new(),
        governor.clone(),
    ));
    let process_manager = BackgroundProcessManager::new(process_store.clone(), executor);
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let host = CapabilityHost::new(registry.as_ref(), dispatcher.as_ref(), &authorizer)
        .with_process_manager(&process_manager);

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "real runtime dispatch"}),
        })
        .await
        .unwrap();

    wait_for_process_status(
        process_store.as_ref(),
        &scope,
        spawned.process.process_id,
        ProcessStatus::Completed,
    )
    .await;
    assert_eq!(
        governor
            .reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone()))
            .process_count,
        0
    );
}

#[tokio::test]
async fn capability_host_with_process_services_spawns_background_result_visible_to_host() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = Arc::new(RecordingSpawnDispatcher::default());
    let executor = Arc::new(DispatchProcessExecutor::new(dispatcher.clone()));
    let services = ProcessServices::in_memory();
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let host = CapabilityHost::new(&registry, dispatcher.as_ref(), &authorizer)
        .with_process_services(&services, executor);

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "services background dispatch"}),
        })
        .await
        .unwrap();

    let process_host = services.host();
    let result = process_host
        .await_result(&scope, spawned.process.process_id)
        .await
        .unwrap();
    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(
        process_host
            .output(&scope, spawned.process.process_id)
            .await
            .unwrap(),
        Some(json!({"message": "services background dispatch"}))
    );
}

#[tokio::test]
async fn capability_host_with_process_services_shares_cancellation_with_process_host() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let observed_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let executor = Arc::new(CancellationObservingExecutor {
        observed_cancel: observed_cancel.clone(),
    });
    let services = ProcessServices::in_memory();
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_services(&services, executor);

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "cancel through services"}),
        })
        .await
        .unwrap();

    let process_host = services.host();
    process_host
        .kill(&scope, spawned.process.process_id)
        .await
        .unwrap();
    wait_for_cancel_observed(observed_cancel.as_ref()).await;
    let exit = process_host
        .await_process(&scope, spawned.process.process_id)
        .await
        .unwrap();
    assert_eq!(exit.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn capability_host_spawn_requires_process_manager() {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let dispatcher = NoopDispatcher;
    let authorizer = GrantAuthorizer::new();
    let context = execution_context(CapabilitySet::default());
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "missing process manager"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ProcessManagerMissing { .. }
    ));
}

#[derive(Default)]
struct RecordingSpawnDispatcher {
    request: std::sync::Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingSpawnDispatcher {
    fn take_request(&self) -> CapabilityDispatchRequest {
        self.request.lock().unwrap().take().unwrap()
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingSpawnDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self.request.lock().unwrap() = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            output: request.input,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

struct InputAssertingProcessManager {
    expected_input: serde_json::Value,
}

#[async_trait]
impl ProcessManager for InputAssertingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        assert_eq!(start.input, self.expected_input);
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        })
    }
}

struct CancellationObservingExecutor {
    observed_cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl ProcessExecutor for CancellationObservingExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        request.cancellation.cancelled().await;
        self.observed_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ProcessExecutionResult {
            output: json!({"cancelled": true}),
        })
    }
}

struct NoopDispatcher;

#[async_trait]
impl CapabilityDispatcher for NoopDispatcher {
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        panic!("spawn_json must not call dispatch_json")
    }
}

fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();
    std::fs::write(storage.join("echo/wasm/echo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    (fs, package_from_manifest(WASM_MANIFEST))
}

fn json_echo_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (global $heap (mut i32) (i32.const 1024))
            (global $out_ptr (mut i32) (i32.const 0))
            (global $out_len (mut i32) (i32.const 0))
            (func (export "alloc") (param $len i32) (result i32)
              (local $ptr i32)
              global.get $heap
              local.set $ptr
              global.get $heap
              local.get $len
              i32.add
              global.set $heap
              local.get $ptr)
            (func (export "say") (param $ptr i32) (param $len i32) (result i32)
              local.get $ptr
              global.set $out_ptr
              local.get $len
              global.set $out_len
              i32.const 0)
            (func (export "output_ptr") (result i32)
              global.get $out_ptr)
            (func (export "output_len") (result i32)
              global.get $out_len))"#,
    )
    .unwrap()
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn grant_for(
    capability: CapabilityId,
    grantee: Principal,
    allowed_effects: Vec<EffectKind>,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee,
        issued_by: Principal::System,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

async fn wait_for_event_count(events: &InMemoryEventSink, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let count = events.events().len();
        if count >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "event sink did not reach {expected} events; last count was {count}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_cancel_observed(flag: &std::sync::atomic::AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "executor did not observe process cancellation"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_process_status<S>(
    store: &S,
    scope: &ResourceScope,
    process_id: ProcessId,
    expected: ProcessStatus,
) where
    S: ProcessStore + ?Sized,
{
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let record = store.get(scope, process_id).await.unwrap().unwrap();
        if record.status == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "process {process_id} did not reach {expected:?}; last status was {:?}",
            record.status
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
