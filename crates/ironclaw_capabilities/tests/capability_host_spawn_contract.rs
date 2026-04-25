use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
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
