use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, FilesystemCapabilityLeaseStore, GrantAuthorizer,
    TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_events::{
    DurableAuditSink, DurableEventSink, EventStreamKey, ReadScope, RuntimeEventKind,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeFailureKind,
};
use ironclaw_processes::{
    FilesystemProcessResultStore, FilesystemProcessStore, ProcessExecutionRequest,
    ProcessExecutionResult, ProcessExecutor, ProcessManager, ProcessServices, ProcessStart,
    ProcessStatus, ProcessStore,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStores, RebornProfile, build_reborn_event_stores,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{
    ApprovalRequestStore, FilesystemApprovalRequestStore, FilesystemRunStateStore, RunStateStore,
    RunStatus,
};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn approval_resume_survives_filesystem_service_restart_and_consumes_lease_once() {
    let temp = tempfile::tempdir().unwrap();
    let engine_root = temp.path().join("engine");
    let event_root = temp.path().join("events");
    let first = durable_services(&engine_root, &event_root).await;
    let first_runtime = first.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants_for_scope(sample_scope(InvocationId::new()));
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "restart approval"});

    let gate = block_for_approval(
        &first_runtime,
        context.clone(),
        estimate.clone(),
        input.clone(),
    )
    .await;
    assert_blocked_run(
        first.run_state.as_ref(),
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;

    let second = durable_services(&engine_root, &event_root).await;
    assert_blocked_run(
        second.run_state.as_ref(),
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;
    assert!(
        second
            .approval_requests
            .get(&scope, gate.approval_request_id)
            .await
            .unwrap()
            .is_some(),
        "pending approval must be readable after service graph rebuild"
    );
    let lease =
        approve_dispatch_for_services(&second.services, &scope, gate.approval_request_id).await;

    let resumed = second
        .services
        .host_runtime_for_local_testing()
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match resumed {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, input);
        }
        other => panic!("expected completed resume outcome, got {other:?}"),
    }
    assert_eq!(
        second
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Consumed
    );

    let third = durable_services(&engine_root, &event_root).await;
    let completed_run = third
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .expect("run state must survive restart");
    assert_eq!(completed_run.status, RunStatus::Completed);
    assert_eq!(
        third
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Consumed,
        "consumed lease state must survive restart"
    );

    let replay = third
        .events
        .events
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    let kinds = replay
        .entries
        .iter()
        .map(|entry| entry.record.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );

    let second_resume = third
        .services
        .host_runtime_for_local_testing()
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            json!({"message": "restart approval"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(second_resume, RuntimeFailureKind::Authorization);
}

#[tokio::test]
async fn process_result_and_output_survive_filesystem_service_restart_with_scope_filtering() {
    let temp = tempfile::tempdir().unwrap();
    let engine_root = temp.path().join("engine");
    let first_services = filesystem_process_services(&engine_root);
    let manager = first_services.background_manager(Arc::new(SuccessProcessExecutor));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let process_id = ProcessId::new();

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    wait_for_status(
        first_services.process_store().as_ref(),
        &scope,
        process_id,
        ProcessStatus::Completed,
    )
    .await;

    let restarted_services = filesystem_process_services(&engine_root);
    let host = restarted_services.host();
    let status = host
        .status(&scope, process_id)
        .await
        .unwrap()
        .expect("process status must survive restart");
    assert_eq!(status.status, ProcessStatus::Completed);
    let result = host
        .result(&scope, process_id)
        .await
        .unwrap()
        .expect("process result must survive restart");
    assert_eq!(result.status, ProcessStatus::Completed);
    assert!(result.output_ref.is_some());
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true}))
    );

    let foreign_scope = ResourceScope {
        project_id: Some(ProjectId::new("other-project").unwrap()),
        ..scope.clone()
    };
    assert!(
        host.status(&foreign_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        host.result(&foreign_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        host.output(&foreign_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn jsonl_event_and_audit_replay_survive_reopen_without_raw_sentinels() {
    let temp = tempfile::tempdir().unwrap();
    let engine_root = temp.path().join("engine");
    let event_root = temp.path().join("events");
    let event_stores = jsonl_event_stores(&event_root).await;
    let services = base_services(
        &engine_root,
        event_stores.clone(),
        Arc::new(AuditAuthorizer),
    )
    .with_audit_sink(Arc::new(DurableAuditSink::new(Arc::clone(
        &event_stores.audit,
    ))));
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "DURABLE_RESTART_RAW_INPUT /tmp/durable-private-path",
        "secret": "DURABLE_RESTART_SECRET_sk_live_secret",
        "output": "DURABLE_RESTART_OUTPUT",
    });

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate::default(),
            payload.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        RuntimeCapabilityOutcome::Completed(completed) if completed.output == payload
    ));

    let reopened = jsonl_event_stores(&event_root).await;
    let event_replay = reopened
        .events
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(event_replay.entries.len(), 3);
    let audit_replay = reopened
        .audit
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(audit_replay.entries.len(), 2);

    let serialized = serde_json::to_string(&(event_replay, audit_replay)).unwrap();
    for forbidden in [
        "DURABLE_RESTART_RAW_INPUT",
        "/tmp/durable-private-path",
        "DURABLE_RESTART_SECRET",
        "DURABLE_RESTART_OUTPUT",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable replay leaked {forbidden}: {serialized}"
        );
    }
}

type DurableProcessServices = ProcessServices<
    FilesystemProcessStore<LocalFilesystem>,
    FilesystemProcessResultStore<LocalFilesystem>,
>;

type DurableHostRuntimeServices = HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    FilesystemProcessStore<LocalFilesystem>,
    FilesystemProcessResultStore<LocalFilesystem>,
>;

struct DurableServices {
    services: DurableHostRuntimeServices,
    run_state: Arc<FilesystemRunStateStore<LocalFilesystem>>,
    approval_requests: Arc<FilesystemApprovalRequestStore<LocalFilesystem>>,
    capability_leases: Arc<FilesystemCapabilityLeaseStore<LocalFilesystem>>,
    events: RebornEventStores,
}

async fn durable_services(engine_root: &Path, event_root: &Path) -> DurableServices {
    let event_stores = jsonl_event_stores(event_root).await;
    // All three filesystem-backed stores now take `Arc<ScopedFilesystem<F>>`
    // (run_state migrated in commit 475588153; capability lease in 34e3c68cb).
    let scoped_fs = scoped_engine_filesystem(engine_root);
    let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(&scoped_fs)));
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(&scoped_fs)));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(&scoped_fs)));
    let services = base_services(
        engine_root,
        event_stores.clone(),
        Arc::new(ApprovalThenGrantAuthorizer),
    )
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases));

    DurableServices {
        services,
        run_state,
        approval_requests,
        capability_leases,
        events: event_stores,
    }
}

fn base_services(
    engine_root: &Path,
    event_stores: RebornEventStores,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
) -> DurableHostRuntimeServices {
    HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(mounted_engine_filesystem(engine_root)),
        Arc::new(InMemoryResourceGovernor::new()),
        authorizer,
        filesystem_process_services(engine_root),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_event_sink(Arc::new(DurableEventSink::new(Arc::clone(
        &event_stores.events,
    ))))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
}

async fn jsonl_event_stores(event_root: &Path) -> RebornEventStores {
    build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: event_root.to_path_buf(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap()
}

fn filesystem_process_services(engine_root: &Path) -> DurableProcessServices {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(mounted_engine_filesystem(engine_root)),
        durable_mount_view(),
    ));
    ProcessServices::filesystem(scoped)
}

/// Mount view granting the migrated consumer crates full per-user-owner
/// permissions on their canonical aliases. Test-only: production
/// composition (`ironclaw_reborn_composition::default_singleton_mount_view`)
/// has the equivalent shape but is `pub(crate)`.
fn durable_mount_view() -> MountView {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/processes").unwrap(),
            VirtualPath::new("/processes").unwrap(),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/authorization").unwrap(),
            VirtualPath::new("/authorization").unwrap(),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/run-state").unwrap(),
            VirtualPath::new("/run-state").unwrap(),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/approvals").unwrap(),
            VirtualPath::new("/approvals").unwrap(),
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .unwrap()
}

/// Build a fresh [`ScopedFilesystem`] over a [`LocalFilesystem`] rooted at
/// `engine_root`. The restart contract spawns multiple service graphs against
/// the same on-disk root, so each call here constructs a distinct
/// `ScopedFilesystem` over a freshly-mounted `LocalFilesystem`; identity of
/// the wrapping struct is irrelevant — durability lives on disk, and the
/// per-path lock map is process-global by design.
fn scoped_engine_filesystem(engine_root: &Path) -> Arc<ScopedFilesystem<LocalFilesystem>> {
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(mounted_engine_filesystem(engine_root)),
        durable_mount_view(),
    ))
}

fn mounted_engine_filesystem(engine_root: &Path) -> LocalFilesystem {
    std::fs::create_dir_all(engine_root).unwrap();
    let mut filesystem = LocalFilesystem::new();
    // Backend mount for `/engine` plus the consumer-store virtual roots
    // exposed via `durable_mount_view`. Each top-level root resolves to a
    // sibling subdirectory under `engine_root` so durable-restart fixtures
    // can reopen the same on-disk tree across service graphs.
    for root in [
        "/engine",
        "/processes",
        "/authorization",
        "/run-state",
        "/approvals",
    ] {
        let host_dir = engine_root.join(root.trim_start_matches('/'));
        std::fs::create_dir_all(&host_dir).unwrap();
        filesystem
            .mount_local(
                VirtualPath::new(root).unwrap(),
                HostPath::from_path_buf(host_dir),
            )
            .unwrap();
    }
    filesystem
}

async fn block_for_approval(
    runtime: &impl HostRuntime,
    context: ExecutionContext,
    estimate: ResourceEstimate,
    input: Value,
) -> ironclaw_host_runtime::RuntimeApprovalGate {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected approval gate, got {other:?}"),
    }
}

async fn approve_dispatch_for_services(
    services: &DurableHostRuntimeServices,
    scope: &ResourceScope,
    approval_request_id: ApprovalRequestId,
) -> ironclaw_authorization::CapabilityLease {
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_dispatch(
            scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap()
}

async fn assert_blocked_run(
    run_state: &dyn RunStateStore,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    approval_request_id: ApprovalRequestId,
) {
    let run = run_state
        .get(scope, invocation_id)
        .await
        .unwrap()
        .expect("run state should exist");
    assert_eq!(run.status, RunStatus::BlockedApproval);
    assert_eq!(run.approval_request_id, Some(approval_request_id));
}

async fn wait_for_status(
    store: &dyn ProcessStore,
    scope: &ResourceScope,
    process_id: ProcessId,
    status: ProcessStatus,
) {
    for _ in 0..100 {
        if let Some(record) = store.get(scope, process_id).await.unwrap()
            && record.status == status
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process {process_id} did not reach {status:?}");
}

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected: RuntimeFailureKind) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected),
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

struct ApprovalThenGrantAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalThenGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        if context.grants.grants.is_empty() {
            Decision::RequireApproval {
                request: ApprovalRequest {
                    id: ApprovalRequestId::new(),
                    correlation_id: context.correlation_id,
                    requested_by: Principal::Extension(context.extension_id.clone()),
                    action: Box::new(Action::Dispatch {
                        capability: descriptor.id.clone(),
                        estimated_resources: estimate.clone(),
                    }),
                    invocation_fingerprint: None,
                    reason: "approval required".to_string(),
                    reusable_scope: None,
                },
            }
        } else {
            GrantAuthorizer::new()
                .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
                .await
        }
    }
}

struct AuditAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for AuditAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::AuditBefore, Obligation::AuditAfter])
                .unwrap(),
        }
    }
}

struct SuccessProcessExecutor;

#[async_trait]
impl ProcessExecutor for SuccessProcessExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ironclaw_processes::ProcessExecutionError> {
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
    }
}

struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
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

fn execution_context_without_grants_for_scope(scope: ResourceScope) -> ExecutionContext {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Script,
        trust: TrustClass::UserTrusted,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn execution_context_with_dispatch_grant_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
) -> ExecutionContext {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Script,
        trust: TrustClass::UserTrusted,
        grants: capability_grants(capability),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn capability_grants(capability: CapabilityId) -> CapabilitySet {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    grants
}

fn process_start(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        extension_id: ExtensionId::new("script").unwrap(),
        capability_id: script_capability_id(),
        runtime: RuntimeKind::Script,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: json!({"message": "running"}),
    }
}

fn local_manifest_trust_policy(
    extension_id: &str,
    allowed_effects: Vec<EffectKind>,
) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new(extension_id).unwrap(),
            format!("/system/extensions/{extension_id}/manifest.toml"),
            None,
            HostTrustAssignment::user_trusted(),
            allowed_effects,
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id,
    }
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script integration extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
