mod support;

use support::{RecordingExecutor, legacy_capability_fixture_to_v2};

use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::json;

#[tokio::test]
async fn dispatcher_routes_wasm_capability_through_registered_adapter() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(WASM_MANIFEST))
        .unwrap();
    let executor = RecordingExecutor::new()
        .static_output(RuntimeKind::Wasm, json!({"message": "hello adapter"}));
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor.clone());
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_output_bytes(10_000),
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

    let requests = executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, ExtensionId::new("echo").unwrap());
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].lane, RuntimeLane::Wasm);
    assert_eq!(requests[0].input, json!({"message": "hello dispatcher"}));
}

#[tokio::test]
async fn dispatcher_routes_script_capability_through_registered_adapter() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let executor = RecordingExecutor::new().static_output(
        RuntimeKind::Script,
        json!({
            "message": "hello script adapter"
        }),
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_process_count(10)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_process_count(1)
                .set_output_bytes(10_000),
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
    let executor = RecordingExecutor::new()
        .failing(RuntimeKind::Script, RuntimeDispatchErrorKind::ExitFailure);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_process_count(10)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_process_count(1)
                .set_output_bytes(10_000),
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
    let executor = RecordingExecutor::new().static_output(
        RuntimeKind::Mcp,
        json!({
            "matches": ["ironclaw"]
        }),
    );
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_process_count(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_process_count(1)
                .set_output_bytes(10_000),
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
    let executor = RecordingExecutor::new().static_output(RuntimeKind::Wasm, json!({}));

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor.clone());
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default().set_concurrency_slots(1),
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "nope"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(executor.requests().is_empty());
}

#[tokio::test]
async fn dispatcher_releases_prepared_reservation_when_validation_fails_before_adapter() {
    let fs = mounted_empty_extension_root();
    let registry = ExtensionRegistry::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, RecordingExecutor::new());
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
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

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, RecordingExecutor::new());
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_process_count(1),
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

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, RecordingExecutor::new());
    let err = dispatcher
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

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, RecordingExecutor::new());
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default().set_concurrency_slots(1),
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

#[tokio::test]
async fn dispatcher_fails_closed_for_system_runtime_without_routing_to_a_lane() {
    // `RuntimeKind::System` is host-internal — `RuntimeLane::from_runtime_kind`
    // returns `None`, so a System capability must never be dispatched to a lane.
    // Even with every untrusted lane wired, the executor is not consulted and
    // dispatch fails closed with the redacted `MissingRuntimeBackend`.
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_bundled_manifest(SYSTEM_MANIFEST))
        .unwrap();
    let executor = RecordingExecutor::new()
        .echo(RuntimeKind::Wasm)
        .echo(RuntimeKind::Mcp)
        .echo(RuntimeKind::Script)
        .echo(RuntimeKind::FirstParty);
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor, executor.clone());
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            run_id: None,
            capability_id: CapabilityId::new("system-ext.op").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default().set_concurrency_slots(1),
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "host-internal"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::System
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    // The closed executor was never consulted for a host-internal runtime.
    assert!(executor.requests().is_empty());
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

// `System`/`FirstParty` runtimes are only assertible by a host-bundled source.
fn package_from_bundled_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    let manifest = ExtensionManifest::parse(
        &manifest,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
    )
    .unwrap();
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

const SYSTEM_MANIFEST: &str = r#"
id = "system-ext"
name = "System Ext"
version = "0.1.0"
description = "Host-internal system runtime demo extension"
trust = "untrusted"

[runtime]
kind = "system"
service = "master-key"

[[capabilities]]
id = "system-ext.op"
description = "Host-internal op"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
