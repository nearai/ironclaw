use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_run_state::*;
use ironclaw_wasm::*;
use serde_json::json;

#[tokio::test]
async fn capability_host_blocks_for_approval_without_dispatch_or_reservation() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = ApprovalAuthorizer;
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "needs approval"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::BlockedApproval);
    let approval_request_id = record.approval_request_id.unwrap();
    let approval = approval_requests
        .get(&scope, approval_request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.scope, scope);
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(approval.request.id, approval_request_id);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn capability_host_records_completed_run_after_authorized_dispatch() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let authorizer = GrantAuthorizer::new();
    let run_state = InMemoryRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "ok"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"message": "ok"}));
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn capability_host_records_failed_run_after_dispatch_error() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = GrantAuthorizer::new();
    let run_state = InMemoryRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "missing runtime"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CapabilityInvocationError::Dispatch(_)));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("Dispatch"));
}

struct ApprovalAuthorizer;

impl CapabilityDispatchAuthorizer for ApprovalAuthorizer {
    fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                reason: "test approval".to_string(),
                reusable_scope: None,
            },
        }
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

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
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
