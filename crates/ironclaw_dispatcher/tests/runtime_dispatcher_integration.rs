mod support;

use support::{RecordingExecutor, legacy_capability_fixture_to_v2};

use std::sync::Arc;

use ironclaw_dispatcher::*;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::{
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        ProcessBackendKind, RuntimeProfile, SecretMode,
    },
    *,
};
use ironclaw_resources::*;
use serde_json::json;

#[tokio::test]
async fn runtime_dispatcher_routes_already_authorized_request_through_public_trait_object() {
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let executor =
        RecordingExecutor::new().static_output(RuntimeKind::Wasm, json!({"reply": "from adapter"}));
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project-a").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();

    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::from_arcs(
        Arc::clone(&registry),
        Arc::clone(&filesystem),
        Arc::clone(&governor),
        executor.clone(),
    )
    .with_event_sink_arc(Arc::new(events.clone()));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;
    let authenticated_actor_user_id =
        UserId::new("slack-alice").expect("valid authenticated actor user id");

    let result = dispatch_port
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
            authenticated_actor_user_id: Some(authenticated_actor_user_id.clone()),
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_output_bytes(10_000),
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

    let requests = executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, ExtensionId::new("echo").unwrap());
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].lane, RuntimeLane::Wasm);
    assert_eq!(requests[0].network_mode, NetworkMode::Deny);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(
        requests[0].authenticated_actor_user_id,
        Some(authenticated_actor_user_id)
    );
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
async fn runtime_dispatcher_forwards_configured_runtime_policy_to_adapter() {
    let registry = Arc::new(registry_with_package(WASM_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let executor =
        RecordingExecutor::new().static_output(RuntimeKind::Wasm, json!({"reply": "from adapter"}));
    let dispatcher = RuntimeDispatcher::from_arcs(registry, filesystem, governor, executor.clone())
        .with_runtime_policy(local_dev_policy());

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: sample_scope(),
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello through configured policy"}),
        })
        .await
        .unwrap();

    let requests = executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].network_mode, NetworkMode::DirectLogged);
}

#[tokio::test]
async fn runtime_dispatcher_fails_closed_for_missing_backend_before_reservation_or_adapter_call() {
    let registry = Arc::new(registry_with_package(SCRIPT_MANIFEST));
    let filesystem = Arc::new(mounted_empty_extension_root());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::from_arcs(
        registry,
        filesystem,
        Arc::clone(&governor),
        RecordingExecutor::new(),
    )
    .with_event_sink_arc(Arc::new(events.clone()));
    let dispatch_port: &dyn CapabilityDispatcher = &dispatcher;

    let err = dispatch_port
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_process_count(1),
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
    let manifest = parse_manifest(WASM_MANIFEST);
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

fn registry_with_package(manifest: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry.insert(package_from_manifest(manifest)).unwrap();
    registry
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = parse_manifest(manifest);
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap()
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

fn local_dev_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
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
