use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::{
    CapabilityObligationError, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest, CapabilitySpawnRequest,
};
use ironclaw_dispatcher::DispatchError;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::HostRuntimeServices;
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
    ProcessServices, ProcessStatus,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use serde_json::json;

#[tokio::test]
async fn host_runtime_services_spawn_through_runtime_dispatcher_outputs_to_process_host() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_script_runtime(script_runtime);
    let dispatcher = services.runtime_dispatcher_arc();
    let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo-script.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();

    let spawned = capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "composed runtime dispatch"}),
        })
        .await
        .unwrap();

    let process_host = services.process_host();
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
        Some(json!({"message": "composed runtime dispatch"}))
    );
}

#[tokio::test]
async fn host_runtime_services_capability_and_process_hosts_share_cancellation() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services);
    let dispatcher = NoopDispatcher;
    let observed_cancel = Arc::new(AtomicBool::new(false));
    let capability_host = services.capability_host(
        &dispatcher,
        Arc::new(CancellationObservingExecutor {
            observed_cancel: observed_cancel.clone(),
        }),
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo-script.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();

    let spawned = capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "cancel through composed hosts"}),
        })
        .await
        .unwrap();

    let process_host = services.process_host();
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
async fn host_runtime_services_composes_approval_resolver_with_shared_audit_sink() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let approval_requests = Arc::new(ironclaw_run_state::InMemoryApprovalRequestStore::new());
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let audit = Arc::new(InMemoryAuditSink::new());
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_approval_requests(approval_requests.clone())
            .with_capability_leases(leases.clone())
            .with_audit_sink(audit.clone());
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let approval = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("echo-script.say").unwrap(),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "composition approval".to_string(),
        reusable_scope: None,
    };
    let request_id = approval.id;
    approval_requests
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    let resolver = services.approval_resolver().expect(
        "approval resolver should be available when approval and lease stores are configured",
    );
    let lease = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    assert_eq!(leases.get(&scope, lease.grant.id).await, Some(lease));
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::ApprovalResolved);
    assert_eq!(records[0].approval_request_id, Some(request_id));
    assert_eq!(records[0].decision.kind, "approved");
}

#[tokio::test]
async fn host_runtime_services_configures_capability_obligation_handler() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(ObligationAuthorizer);
    let process_services = ProcessServices::in_memory();
    let observed = Arc::new(AtomicBool::new(false));
    let handler = Arc::new(FlaggingObligationHandler {
        observed: Arc::clone(&observed),
    });
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_obligation_handler(handler);
    let dispatcher = NoopDispatcher;
    let capability_host = services.capability_host(&dispatcher, Arc::new(ImmediateExecutor));
    let context = execution_context(CapabilitySet::default());

    capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "obligated spawn"}),
        })
        .await
        .unwrap();

    assert!(observed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn host_runtime_services_can_configure_spawn_run_state_stores() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let run_state = Arc::new(ironclaw_run_state::InMemoryRunStateStore::new());
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_run_state(run_state.clone());
    let dispatcher = NoopDispatcher;
    let capability_host = services.capability_host(
        &dispatcher,
        Arc::new(CancellationObservingExecutor {
            observed_cancel: Arc::new(AtomicBool::new(false)),
        }),
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo-script.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "record state"}),
        })
        .await
        .unwrap();

    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, ironclaw_run_state::RunStatus::Completed);
}

struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value: serde_json::Value =
            serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

struct ImmediateExecutor;

#[async_trait]
impl ProcessExecutor for ImmediateExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
    }
}

struct CancellationObservingExecutor {
    observed_cancel: Arc<AtomicBool>,
}

#[async_trait]
impl ProcessExecutor for CancellationObservingExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        request.cancellation.cancelled().await;
        self.observed_cancel.store(true, Ordering::SeqCst);
        Ok(ProcessExecutionResult {
            output: json!({"cancelled": true}),
        })
    }
}

struct FlaggingObligationHandler {
    observed: Arc<AtomicBool>,
}

#[async_trait]
impl CapabilityObligationHandler for FlaggingObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        assert_eq!(request.phase, CapabilityObligationPhase::Spawn);
        assert_eq!(request.obligations, &[Obligation::AuditBefore]);
        self.observed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct ObligationAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for ObligationAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::AuditBefore],
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::AuditBefore],
        }
    }
}

struct NoopDispatcher;

#[async_trait]
impl CapabilityDispatcher for NoopDispatcher {
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        panic!("test executor should not use dispatcher")
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
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

async fn wait_for_cancel_observed(flag: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "executor did not observe process cancellation"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

const SCRIPT_MANIFEST: &str = r#"
id = "echo-script"
name = "Echo Script"
version = "0.1.0"
description = "Echo script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
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
