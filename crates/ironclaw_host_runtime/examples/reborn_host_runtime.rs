use std::{error::Error, sync::Arc};

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_capabilities::CapabilitySpawnRequest;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy, Principal,
    ProjectId, ResourceEstimate, ResourceScope, RuntimeKind, TenantId, TrustClass, UserId,
    VirtualPath,
};
use ironclaw_host_runtime::HostRuntimeServices;
use ironclaw_processes::{ProcessServices, ProcessStatus};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use serde_json::json;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST)?);
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        InProcessEchoBackend,
    ));

    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_script_runtime(script_runtime);

    let dispatcher = services.runtime_dispatcher_arc();
    let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo-script.say")?,
            Principal::Extension(ExtensionId::new("caller")?),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )?],
    })?;
    let scope = context.resource_scope.clone();

    let spawned = capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say")?,
            estimate: ResourceEstimate::default(),
            input: json!({"message": "hello from HostRuntimeServices"}),
        })
        .await?;

    let process_host = services.process_host();
    let result = process_host
        .await_result(&scope, spawned.process.process_id)
        .await?;
    let output = process_host
        .output(&scope, spawned.process.process_id)
        .await?
        .unwrap_or(json!(null));

    println!("process_id={}", spawned.process.process_id);
    println!("status={:?}", result.status);
    println!("output={output}");

    if result.status != ProcessStatus::Completed {
        return Err(format!("expected completed process, got {:?}", result.status).into());
    }

    Ok(())
}

struct InProcessEchoBackend;

impl ScriptBackend for InProcessEchoBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let input: serde_json::Value =
            serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(input))
    }
}

fn registry_with_manifest(manifest: &str) -> Result<ExtensionRegistry, Box<dyn Error>> {
    let manifest = ExtensionManifest::parse(manifest)?;
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str()))?;
    let package = ExtensionPackage::from_manifest(manifest, root)?;
    let mut registry = ExtensionRegistry::new();
    registry.insert(package)?;
    Ok(registry)
}

fn grant_for(
    capability: CapabilityId,
    grantee: Principal,
    allowed_effects: Vec<EffectKind>,
) -> Result<CapabilityGrant, Box<dyn Error>> {
    Ok(CapabilityGrant {
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
    })
}

fn execution_context(grants: CapabilitySet) -> Result<ExecutionContext, Box<dyn Error>> {
    let invocation_id = ironclaw_host_api::InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1")?,
        user_id: UserId::new("user1")?,
        project_id: Some(ProjectId::new("project1")?),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };

    Ok(ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller")?,
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    })
}

const SCRIPT_MANIFEST: &str = r#"
id = "echo-script"
name = "Echo Script"
version = "0.1.0"
description = "Echo script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
# The manifest parser currently accepts the V1 script backend string `docker`.
# This example does not invoke Docker; HostRuntimeServices injects the
# in-process InProcessEchoBackend below as the actual runtime backend.
backend = "docker"
image = "example/echo"
command = "echo"

[[capabilities]]
id = "echo-script.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
