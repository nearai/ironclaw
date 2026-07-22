#![allow(dead_code)]
// arch-exempt: large_file, mechanical lease-store test repoint to FilesystemCapabilityLeaseStore<InMemoryBackend> helper (arch-simplification §4.3), no new test logic, plan #6168

use super::legacy_capability_fixture_to_v2;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    FilesystemCapabilityLeaseStore, GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer,
    in_memory_backed_capability_lease_store,
};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest, CapabilitySpawnRequest, CredentialPresence, HostPolicyFacts,
    PolicyAction,
};
use ironclaw_events::{
    DurableAuditLog, EventCursor, EventError, EventReplay, EventStreamKey, InMemoryAuditSink,
    InMemoryEventSink, ReadScope,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::{
    DiskFilesystem, Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::dispatch_test_support::TestDispatcher;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationHandler, BuiltinObligationServices, CapabilitySurfaceVersion,
    CommandExecutionOutput, CommandExecutionRequest, DefaultHostRuntime, HostRuntime,
    HostRuntimeServices, ProcessObligationLifecycleStore, ProductionWiringComponent,
    ProductionWiringConfig, ProductionWiringIssueKind, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeProcessError, RuntimeProcessPort,
    SandboxCommandTransport, builtin_first_party_package,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{
    BackgroundFailureStage, BackgroundProcessManager, FilesystemProcessResultStore,
    FilesystemProcessStore, ProcessError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessResultStore, ProcessStart, ProcessStatus, ProcessStore,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits,
};
use ironclaw_run_state::{
    ApprovalRecord, ApprovalRequestStore, RunRecord, RunStart, RunStateApprovalStore,
    RunStateError, RunStateStore, RunStatus,
};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptExecutionRequest,
    ScriptExecutionResult, ScriptExecutor, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_secrets::{FilesystemSecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef,
    SubmitTurnRequest, TurnActor, TurnScope,
};
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier};
use ironclaw_wasm::{
    WasmHostError, WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest, WitToolHost,
    WitToolRuntimeConfig,
};
use serde_json::json;
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

/// Permissive [`HostPolicyFacts`] double for kernel-tier `CapabilityHost` tests:
/// every credential is present and no persistent grants exist, so the in-fold
/// credential pre-flight (§5.3.2/§9) never fires. Production credential
/// pre-flight behavior is covered through the `DefaultHostRuntime` caller in the
/// host_runtime integration suites, not here.
pub(crate) struct PermissiveHostPolicyFacts;

#[async_trait]
impl HostPolicyFacts for PermissiveHostPolicyFacts {
    async fn credential_presence(
        &self,
        _capability_id: &CapabilityId,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        _capability_id: &CapabilityId,
        _context: &ExecutionContext,
        _action: PolicyAction,
    ) -> Vec<CapabilityGrant> {
        Vec::new()
    }
}

/// Construct an [`Arc<ScopedFilesystem<LibSqlRootFilesystem>>`] that exposes
/// the `/turns` mount alias over a libSQL-backed [`RootFilesystem`]. Mirrors
/// the production composition shape: the `/turns` alias rewrites to a
/// tenant/user-scoped target inside `/engine`, and the filesystem backend
/// supplies durable storage. Used by tests that previously constructed
/// `LibSqlTurnStateStore` directly.
pub(crate) async fn libsql_scoped_turns_fs(
    db: Arc<libsql::Database>,
) -> Arc<ScopedFilesystem<LibSqlRootFilesystem>> {
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.unwrap();
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").unwrap(),
        VirtualPath::new("/engine/tenants/tenant1/users/user1/turns").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(filesystem, view))
}

#[derive(Debug, Default)]
pub(crate) struct RecordingTurnRunWakeNotifier {
    pub(crate) wakes: Mutex<Vec<TurnRunWake>>,
}

impl RecordingTurnRunWakeNotifier {
    pub(crate) fn wakes(&self) -> Vec<TurnRunWake> {
        self.wakes.lock().unwrap().clone()
    }
}

impl TurnRunWakeNotifier for RecordingTurnRunWakeNotifier {
    fn notify_queued_run(
        &self,
        wake: TurnRunWake,
    ) -> Result<(), ironclaw_turns::TurnRunWakeNotifyError> {
        self.wakes.lock().unwrap().push(wake);
        Ok(())
    }
}

pub(crate) async fn assert_services_use_combined_store_for_atomic_approval_block<
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
>(
    services: HostRuntimeServices<F, G, S, R>,
    message: &str,
) {
    let combined_store = Arc::new(InMemoryRecordingCombinedRunStateApprovalStore::new());
    let services = services
        .with_trust_policy(Arc::new(local_manifest_trust_policy(
            "script",
            vec![EffectKind::DispatchCapability],
        )))
        .with_run_state_approval_store(Arc::clone(&combined_store))
        .with_script_runtime(Arc::new(ScriptRuntime::new(
            ScriptRuntimeConfig::for_testing(),
            EchoScriptBackend,
        )));

    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": message}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            assert_eq!(combined_store.combined_calls(), 1);
            assert_eq!(combined_store.separate_save_calls(), 0);
            let run_record = RunStateStore::get(
                combined_store.as_ref(),
                &context.resource_scope,
                context.invocation_id,
            )
            .await
            .unwrap()
            .expect("run record persisted");
            assert_eq!(run_record.status, RunStatus::BlockedApproval);
            assert_eq!(
                run_record.approval_request_id,
                Some(gate.approval_request_id)
            );
            assert!(
                ApprovalRequestStore::get(
                    combined_store.as_ref(),
                    &context.resource_scope,
                    gate.approval_request_id,
                )
                .await
                .unwrap()
                .is_some()
            );
        }
        other => panic!("expected approval gate, got {other:?}"),
    }
}

pub(crate) fn assert_failed_outcome(
    outcome: RuntimeCapabilityOutcome,
    expected_kind: RuntimeFailureKind,
) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected_kind),
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

pub(crate) fn assert_completed_outcome(
    outcome: RuntimeCapabilityOutcome,
    expected_capability: &CapabilityId,
) {
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(&completed.capability_id, expected_capability);
            assert_eq!(completed.output, json!(1));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
}

pub(crate) type InMemoryHostRuntimeServices = HostRuntimeServices<
    DiskFilesystem,
    InMemoryResourceGovernor,
    FilesystemProcessStore<InMemoryBackend>,
    FilesystemProcessResultStore<InMemoryBackend>,
>;

pub(crate) struct InMemoryRecordingCombinedRunStateApprovalStore {
    pub(crate) runs:
        ironclaw_run_state::FilesystemRunStateStore<ironclaw_filesystem::InMemoryBackend>,
    pub(crate) approvals:
        ironclaw_run_state::FilesystemApprovalRequestStore<ironclaw_filesystem::InMemoryBackend>,
    pub(crate) combined_calls: AtomicUsize,
    pub(crate) separate_save_calls: AtomicUsize,
}

impl InMemoryRecordingCombinedRunStateApprovalStore {
    pub(crate) fn new() -> Self {
        Self {
            runs: ironclaw_run_state::in_memory_backed_run_state_store(),
            approvals: ironclaw_run_state::in_memory_backed_approval_request_store(),
            combined_calls: AtomicUsize::new(0),
            separate_save_calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn combined_calls(&self) -> usize {
        self.combined_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn separate_save_calls(&self) -> usize {
        self.separate_save_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RunStateStore for InMemoryRecordingCombinedRunStateApprovalStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.runs.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.runs
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.runs.block_auth(scope, invocation_id, error_kind).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.runs.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.runs.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.runs.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.runs.records_for_scope(scope).await
    }
}

#[async_trait]
impl ApprovalRequestStore for InMemoryRecordingCombinedRunStateApprovalStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.separate_save_calls.fetch_add(1, Ordering::SeqCst);
        self.approvals.save_pending(scope, request).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        self.approvals.get(scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.approvals.approve(scope, request_id).await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.approvals.deny(scope, request_id).await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.approvals.discard_pending(scope, request_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        self.approvals.records_for_scope(scope).await
    }
}

#[async_trait]
impl RunStateApprovalStore for InMemoryRecordingCombinedRunStateApprovalStore {
    async fn save_pending_and_block_approval(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.combined_calls.fetch_add(1, Ordering::SeqCst);
        self.approvals
            .save_pending(scope.clone(), approval.clone())
            .await?;
        self.runs
            .block_approval(&scope, invocation_id, approval)
            .await
    }
}

pub(crate) struct ApprovalResumeFixture {
    pub(crate) services: InMemoryHostRuntimeServices,
    pub(crate) run_state:
        Arc<ironclaw_run_state::FilesystemRunStateStore<ironclaw_filesystem::InMemoryBackend>>,
    pub(crate) approval_requests: Arc<
        ironclaw_run_state::FilesystemApprovalRequestStore<ironclaw_filesystem::InMemoryBackend>,
    >,
    pub(crate) capability_leases: Arc<FilesystemCapabilityLeaseStore<InMemoryBackend>>,
    pub(crate) events: InMemoryEventSink,
}

pub(crate) fn approval_resume_fixture() -> ApprovalResumeFixture {
    approval_resume_fixture_with_manifest(SCRIPT_MANIFEST, vec![EffectKind::DispatchCapability])
}

pub(crate) fn approval_resume_fixture_with_manifest(
    manifest: &str,
    trust_effects: Vec<EffectKind>,
) -> ApprovalResumeFixture {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let events = InMemoryEventSink::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        trust_effects,
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_event_sink(Arc::new(events.clone()));

    ApprovalResumeFixture {
        services,
        run_state,
        approval_requests,
        capability_leases,
        events,
    }
}

pub(crate) fn resume_runtime_with_empty_registry(
    fixture: &ApprovalResumeFixture,
) -> DefaultHostRuntime {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&fixture.run_state))
    .with_approval_requests(Arc::clone(&fixture.approval_requests))
    .with_capability_leases(Arc::clone(&fixture.capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .host_runtime_for_local_testing()
}

pub(crate) fn resume_runtime_with_policy(
    fixture: &ApprovalResumeFixture,
    policy: EffectiveRuntimePolicy,
) -> DefaultHostRuntime {
    HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_NETWORK_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )))
    .with_run_state(Arc::clone(&fixture.run_state))
    .with_approval_requests(Arc::clone(&fixture.approval_requests))
    .with_capability_leases(Arc::clone(&fixture.capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_event_sink(Arc::new(fixture.events.clone()))
    .with_runtime_policy(policy)
    .host_runtime_for_local_testing()
}

pub(crate) async fn assert_blocked_approval_run(
    fixture: &ApprovalResumeFixture,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    approval_request_id: ApprovalRequestId,
) {
    let run = fixture
        .run_state
        .get(scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    assert_eq!(run.approval_request_id, Some(approval_request_id));
    assert_eq!(run.error_kind, None);
}

pub(crate) async fn block_for_approval(
    runtime: &impl HostRuntime,
    context: ExecutionContext,
    estimate: ResourceEstimate,
    input: serde_json::Value,
) -> ironclaw_host_runtime::RuntimeApprovalGate {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected approval gate, got {other:?}"),
    }
}

pub(crate) async fn approve_dispatch_for_services(
    services: &InMemoryHostRuntimeServices,
    scope: &ResourceScope,
    approval_request_id: ApprovalRequestId,
    expires_at: Option<Timestamp>,
) -> ironclaw_authorization::CapabilityLease {
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_dispatch(
            scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .unwrap()
}

pub(crate) async fn approve_spawn_for_services(
    services: &InMemoryHostRuntimeServices,
    scope: &ResourceScope,
    approval_request_id: ApprovalRequestId,
    expires_at: Option<Timestamp>,
) -> ironclaw_authorization::CapabilityLease {
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_spawn(
            scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: process_sandbox_authority_effects(),
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .unwrap()
}

pub(crate) struct SentinelApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SentinelApprovalAuthorizer {
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
                    reason: "APPROVAL_REASON_SENTINEL_3022 /tmp/private-approval-reason"
                        .to_string(),
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

pub(crate) struct ApprovalThenGrantAuthorizer;

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

    async fn authorize_spawn_with_trust(
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
                    action: Box::new(Action::SpawnCapability {
                        capability: descriptor.id.clone(),
                        estimated_resources: estimate.clone(),
                    }),
                    invocation_fingerprint: None,
                    reason: "spawn approval required".to_string(),
                    reusable_scope: None,
                },
            }
        } else {
            GrantAuthorizer::new()
                .authorize_spawn_with_trust(context, descriptor, estimate, trust_decision)
                .await
        }
    }
}

pub(crate) struct ApprovalThenSecretObligationAuthorizer {
    pub(crate) handle: SecretHandle,
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalThenSecretObligationAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
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
            Decision::Allow {
                obligations: Obligations::new(vec![Obligation::InjectSecretOnce {
                    handle: self.handle.clone(),
                }])
                .unwrap(),
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingScriptExecutor {
    pub(crate) mounts: std::sync::Mutex<Vec<Option<MountView>>>,
}

impl RecordingScriptExecutor {
    pub(crate) fn recorded_mounts(&self) -> Vec<Option<MountView>> {
        self.mounts.lock().unwrap().clone()
    }
}

impl ScriptExecutor for RecordingScriptExecutor {
    fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: ScriptExecutionRequest<'_>,
    ) -> Result<ScriptExecutionResult, ironclaw_scripts::ScriptError> {
        self.mounts.lock().unwrap().push(request.mounts.clone());
        let reservation = match request.resource_reservation.clone() {
            Some(reservation) => reservation,
            None => governor.reserve(request.scope.clone(), request.estimate.clone())?,
        };
        let usage = ResourceUsage::default();
        let receipt = governor.reconcile(reservation.id, usage.clone())?;
        Ok(ScriptExecutionResult {
            result: CapabilityHostResult {
                output: request.invocation.input,
                reservation_id: reservation.id,
                usage,
                output_bytes: 0,
            },
            receipt,
        })
    }
}

pub(crate) struct ExitFailureScriptBackend;

impl ScriptBackend for ExitFailureScriptBackend {
    fn execute(&self, _request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        Ok(ScriptBackendOutput {
            exit_code: 2,
            stdout: Vec::new(),
            stderr: b"simulated script failure".to_vec(),
            wall_clock_ms: 1,
        })
    }
}

pub(crate) struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

pub(crate) struct FailingDurableAuditLog;

#[async_trait]
impl DurableAuditLog for FailingDurableAuditLog {
    async fn append(
        &self,
        _record: AuditEnvelope,
    ) -> Result<ironclaw_events::EventLogEntry<AuditEnvelope>, EventError> {
        Err(EventError::DurableLog {
            reason: "simulated audit backend failure at /tmp/audit-backend-secret".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        _after: Option<EventCursor>,
        _limit: usize,
    ) -> Result<EventReplay<AuditEnvelope>, EventError> {
        Err(EventError::DurableLog {
            reason: "simulated audit replay failure".to_string(),
        })
    }
}

pub(crate) struct AllowAllDispatchAuthorizer;

pub(crate) struct ObligatingAuthorizer {
    pub(crate) obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    pub(crate) fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[derive(Debug)]
pub(crate) struct ProductionCandidateProcessPort;

#[async_trait]
impl RuntimeProcessPort for ProductionCandidateProcessPort {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ProductionCandidateSandboxTransport;

#[async_trait]
impl SandboxCommandTransport for ProductionCandidateSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: false,
            duration: Duration::ZERO,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordingNetworkHttpEgress {
    pub(crate) requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetworkHttpEgress {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        self.requests.lock().unwrap().push(request);
        Ok(NetworkHttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: 0,
                resolved_ip: None,
            },
        })
    }
}

#[derive(Debug)]
pub(crate) struct SecretStoreLeaseCredentials {
    pub(crate) handle: SecretHandle,
}

impl WasmRuntimeCredentialProvider for SecretStoreLeaseCredentials {
    fn credential_injections(
        &self,
        _request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        Ok(vec![RuntimeCredentialInjection {
            handle: self.handle.clone(),
            source: RuntimeCredentialSource::SecretStoreLease,
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }])
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordingRuntimeHttpEgress {
    pub(crate) requests: Arc<std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>>,
    pub(crate) delay: Duration,
    pub(crate) response_status: u16,
}

impl Default for RecordingRuntimeHttpEgress {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingRuntimeHttpEgress {
    pub(crate) fn new() -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            delay: Duration::ZERO,
            response_status: 200,
        }
    }

    pub(crate) fn with_delay(delay: Duration) -> Self {
        Self {
            delay,
            response_status: 204,
            ..Self::new()
        }
    }

    pub(crate) fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.requests.lock().unwrap().push(request.clone());
        Ok(RuntimeHttpEgressResponse {
            status: self.response_status,
            headers: Vec::new(),
            body: Vec::new(),
            saved_body: None,
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}

pub(crate) async fn stage_process_handoffs(
    services: &BuiltinObligationServices,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    secret_handle: &SecretHandle,
    policy: NetworkPolicy,
    material: &str,
) {
    services
        .secret_store()
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from(material),
            None,
        )
        .await
        .unwrap();
    let context =
        execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope.clone());
    services
        .obligation_handler()
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &[
                Obligation::ApplyNetworkPolicy { policy },
                Obligation::InjectSecretOnce {
                    handle: secret_handle.clone(),
                },
            ],
        })
        .await
        .unwrap();
}

pub(crate) struct SpawnObligationFixture {
    pub(crate) registry: Arc<ExtensionRegistry>,
    pub(crate) dispatcher: Arc<TestDispatcher>,
    pub(crate) authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    pub(crate) handler: Arc<BuiltinObligationHandler>,
    pub(crate) process_manager: Arc<BackgroundProcessManager>,
    pub(crate) process_store: Arc<ProcessObligationLifecycleStore>,
    pub(crate) governor: Arc<InMemoryResourceGovernor>,
    pub(crate) context: ExecutionContext,
    pub(crate) scope: ResourceScope,
    pub(crate) estimate: ResourceEstimate,
}

impl SpawnObligationFixture {
    pub(crate) async fn spawn(&self) -> ironclaw_processes::ProcessRecord {
        // Kernel now computes trust + runtime-policy in-fold (§5.3.2/§9); supply
        // a trust policy that mirrors the former `trust_decision_with_dispatch_authority`
        // ceiling and a runtime policy that permits the script process backend.
        let trust_policy = local_manifest_trust_policy(
            "script",
            vec![EffectKind::DispatchCapability, EffectKind::Network],
        );
        let runtime_policy = local_dev_runtime_policy();
        let host = CapabilityHost::new(
            self.registry.as_ref(),
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
            &trust_policy,
            &runtime_policy,
            &PermissiveHostPolicyFacts,
        )
        .with_obligation_handler(self.handler.as_ref())
        .with_process_manager(self.process_manager.as_ref());

        host.spawn_json(CapabilitySpawnRequest {
            context: self.context.clone(),
            capability_id: script_capability_id(),
            estimate: self.estimate.clone(),
            input: json!({"message": "background"}),
        })
        .await
        .unwrap()
        .process
    }
}

pub(crate) async fn spawn_obligation_fixture(
    reservation_id: ResourceReservationId,
    secret_handle: SecretHandle,
    executor: BackgroundExecutor,
) -> SpawnObligationFixture {
    spawn_obligation_fixture_with_result_store(
        reservation_id,
        secret_handle,
        executor,
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
    )
    .await
}

pub(crate) async fn spawn_obligation_fixture_with_result_store<R>(
    reservation_id: ResourceReservationId,
    secret_handle: SecretHandle,
    executor: BackgroundExecutor,
    result_store: Arc<R>,
) -> SpawnObligationFixture
where
    R: ProcessResultStore + 'static,
{
    spawn_obligation_fixture_with_process_store_and_result_store(
        reservation_id,
        secret_handle,
        executor,
        Arc::new(ironclaw_processes::in_memory_backed_process_store()),
        result_store,
    )
    .await
}

pub(crate) async fn spawn_obligation_fixture_with_process_store_and_result_store<P, R>(
    reservation_id: ResourceReservationId,
    secret_handle: SecretHandle,
    executor: BackgroundExecutor,
    inner_process_store: Arc<P>,
    result_store: Arc<R>,
) -> SpawnObligationFixture
where
    P: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        panic!("spawn tests must not invoke the foreground dispatcher")
    }));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let secret_store = Arc::new(FilesystemSecretStore::ephemeral());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        secret_store.clone(),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let context =
        execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone());
    let estimate = ResourceEstimate::default()
        .set_process_count(1)
        .set_concurrency_slots(1);
    secret_store
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from("runtime-secret"),
            None,
        )
        .await
        .unwrap();
    let handler = Arc::new(obligation_services.obligation_handler());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ReserveResources { reservation_id },
            Obligation::ApplyNetworkPolicy {
                policy: wasm_http_policy(),
            },
            Obligation::InjectSecretOnce {
                handle: secret_handle,
            },
        ]));
    let process_store =
        Arc::new(obligation_services.process_obligation_lifecycle_store(inner_process_store));
    let cleanup_process_store = Arc::clone(&process_store);
    let process_manager = Arc::new(
        BackgroundProcessManager::new(Arc::clone(&process_store), Arc::new(executor))
            .with_result_store(result_store)
            .with_error_handler(move |failure| {
                let reconcile = match failure.stage {
                    BackgroundFailureStage::StoreComplete => true,
                    BackgroundFailureStage::StoreFail => false,
                    BackgroundFailureStage::ResultStoreComplete => true,
                    BackgroundFailureStage::ResultStoreFail => false,
                    _ => return,
                };
                let cleanup_process_store = Arc::clone(&cleanup_process_store);
                tokio::spawn(async move {
                    if let Err(error) = cleanup_process_store
                        .cleanup_process_obligations(&failure.scope, failure.process_id, reconcile)
                        .await
                    {
                        tracing::debug!(?error, "best-effort process obligation cleanup failed");
                    }
                });
            }),
    );

    SpawnObligationFixture {
        registry,
        dispatcher,
        authorizer,
        handler,
        process_manager,
        process_store,
        governor,
        context,
        scope,
        estimate,
    }
}

#[derive(Debug)]
pub(crate) struct FailingCleanupResourceGovernor;

impl ResourceGovernor for FailingCleanupResourceGovernor {
    fn set_limit(
        &self,
        _account: ResourceAccount,
        _limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        Ok(ironclaw_resources::ReservationOutcome {
            reservation: ResourceReservation {
                id: ResourceReservationId::new(),
                scope,
                estimate,
            },
            warnings: Vec::new(),
        })
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        Ok(ironclaw_resources::ReservationOutcome {
            reservation: ResourceReservation {
                id: reservation_id,
                scope,
                estimate,
            },
            warnings: Vec::new(),
        })
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        _actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::ReservationMismatch { id: reservation_id })
    }

    fn validate_reservation(&self, reservation: &ResourceReservation) -> Result<(), ResourceError> {
        Err(ResourceError::ReservationMismatch { id: reservation.id })
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::ReservationMismatch { id: reservation_id })
    }

    fn account_snapshot(
        &self,
        _account: &ResourceAccount,
    ) -> Result<Option<ironclaw_resources::AccountSnapshot>, ResourceError> {
        Ok(None)
    }
}

/// Real `FilesystemProcessResultStore` over a [`FaultInjecting`] backend armed
/// to fail every result write, replacing the whole-trait
/// `FailingProcessResultStore` fake. Both `complete` (first `put` =
/// `write_output`) and `fail` (first `put` = `write_result`) hit the injected
/// `FilesystemError::Backend` on their first backend write, which the store
/// maps to `ProcessError::Filesystem`. Returns the store plus the fault handle
/// so a test can observe the write attempt (`backend.count(WriteFile)`).
pub(crate) fn result_store_failing_writes() -> (
    Arc<FilesystemProcessResultStore<FaultInjecting<InMemoryBackend>>>,
    Arc<FaultInjecting<InMemoryBackend>>,
) {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()).with_fault(
        Fault::on(FilesystemOperation::WriteFile).backend("injected process-result write failure"),
    ));
    let store = Arc::new(FilesystemProcessResultStore::new(scoped_processes_fs(
        backend.clone(),
    )));
    (store, backend)
}

/// Real `FilesystemProcessStore` over a [`FaultInjecting`] backend armed to
/// fail the terminal status transition's write, replacing the whole-trait
/// `FailingTerminalProcessStore` fake. `start`'s record write is the 1st
/// backend write and succeeds; the terminal transition (`complete` or `fail`,
/// whichever the manager issues for the executor outcome) is the 2nd write and
/// is faulted, surfacing as `ProcessError::Filesystem`. `get` /
/// `records_for_scope` still read the live `Running` record.
pub(crate) fn terminal_failing_process_store() -> (
    Arc<FilesystemProcessStore<FaultInjecting<InMemoryBackend>>>,
    Arc<FaultInjecting<InMemoryBackend>>,
) {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .nth(2)
                .backend("injected terminal transition write failure"),
        ),
    );
    let store = Arc::new(FilesystemProcessStore::new(scoped_processes_fs(
        backend.clone(),
    )));
    (store, backend)
}

/// A `ScopedFilesystem` over `backend` exposing the `/processes` mount — the
/// fault-injecting analogue of
/// `ironclaw_processes::in_memory_backed_processes_filesystem()`.
fn scoped_processes_fs(
    backend: Arc<FaultInjecting<InMemoryBackend>>,
) -> Arc<ScopedFilesystem<FaultInjecting<InMemoryBackend>>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").unwrap(),
        VirtualPath::new("/engine/processes").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

/// Real `FilesystemSecretStore` over a [`FaultInjecting`] backend armed to fail
/// every secret read, replacing the whole-trait, call-counting
/// `CountingErrorSecretStore` fake. `metadata()` runs its genuine
/// `read_secret` -> `get` path and the injected `FilesystemError::Backend` maps
/// (`fs_to_secret_store_error`) to `SecretStoreError::StoreUnavailable` — the
/// same erroring shape the fake hand-returned, now proven through the real
/// store. Returns the store plus the fault handle, so a test can observe the
/// read probes (`backend.count(ReadFile)`) instead of a bespoke counter.
pub(crate) fn secret_store_failing_reads() -> (
    Arc<FilesystemSecretStore<FaultInjecting<InMemoryBackend>>>,
    Arc<FaultInjecting<InMemoryBackend>>,
) {
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::ReadFile)
                .path("secrets")
                .backend("injected secret read failure"),
        ),
    );
    let store = Arc::new(FilesystemSecretStore::ephemeral_over(backend.clone()));
    (store, backend)
}

pub(crate) struct BackgroundExecutor {
    pub(crate) outcome: BackgroundExecutorOutcome,
}

impl BackgroundExecutor {
    pub(crate) fn success() -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Success(json!({"ok": true})),
        }
    }

    pub(crate) fn success_with_output(output: serde_json::Value) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Success(output),
        }
    }

    pub(crate) fn failure(kind: impl Into<String>) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Failure(kind.into()),
        }
    }

    pub(crate) fn delayed_success(delay: Duration) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::DelayedSuccess(delay),
        }
    }
}

pub(crate) enum BackgroundExecutorOutcome {
    Success(serde_json::Value),
    Failure(String),
    DelayedSuccess(Duration),
}

#[async_trait]
impl ProcessExecutor for BackgroundExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ironclaw_processes::ProcessExecutionError> {
        match &self.outcome {
            BackgroundExecutorOutcome::Success(output) => Ok(ProcessExecutionResult {
                output: output.clone(),
            }),
            BackgroundExecutorOutcome::Failure(kind) => {
                Err(ironclaw_processes::ProcessExecutionError::new(kind.clone()))
            }
            BackgroundExecutorOutcome::DelayedSuccess(delay) => {
                tokio::time::sleep(*delay).await;
                Ok(ProcessExecutionResult {
                    output: json!({"ok": true}),
                })
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingSandboxProcessExecutor {
    pub(crate) requests: std::sync::Mutex<Vec<ProcessExecutionRequest>>,
}

impl RecordingSandboxProcessExecutor {
    pub(crate) fn requests(&self) -> Vec<ProcessExecutionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProcessExecutor for RecordingSandboxProcessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ironclaw_processes::ProcessExecutionError> {
        self.requests.lock().unwrap().push(request);
        Ok(ProcessExecutionResult {
            output: json!({"executor": "process_sandbox"}),
        })
    }
}

pub(crate) struct FailingSpawnManager;

#[async_trait]
impl ironclaw_processes::ProcessManager for FailingSpawnManager {
    async fn spawn(
        &self,
        _start: ProcessStart,
    ) -> Result<ironclaw_processes::ProcessRecord, ProcessError> {
        Err(ProcessError::InvalidStoredRecord {
            reason: "start failed".to_string(),
        })
    }
}

pub(crate) async fn wait_for_status(
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

pub(crate) async fn wait_for_sandbox_process_result(
    executor: &RecordingSandboxProcessExecutor,
    scope: &ResourceScope,
    process_id: ProcessId,
    result_store: &dyn ProcessResultStore,
) {
    for _ in 0..100 {
        let requests = executor.requests();
        if let Some(request) = requests.first()
            && request.process_id == process_id
            && request.capability_id == process_sandbox_capability_id()
            && request.runtime == RuntimeKind::System
            && let Some(result) = result_store.get(scope, process_id).await.unwrap()
        {
            assert_eq!(result.status, ProcessStatus::Completed);
            // Filesystem result store externalizes output behind `output_ref`;
            // read the bytes through the store, not the inline record field (§4.3).
            assert_eq!(
                result_store.output(scope, process_id).await.unwrap(),
                Some(json!({"executor": "process_sandbox"}))
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process sandbox executor did not complete process {process_id}");
}

pub(crate) async fn wait_for_result_store_write(backend: &FaultInjecting<InMemoryBackend>) {
    // The result store's terminal write (complete/fail) is faulted; the gated
    // op is still recorded, so a single recorded WriteFile marks the attempt.
    for _ in 0..100 {
        if backend.count(FilesystemOperation::WriteFile) >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("result store did not attempt a write");
}

pub(crate) async fn wait_for_terminal_transition_write(backend: &FaultInjecting<InMemoryBackend>) {
    // `start`'s record write is the 1st backend write; the faulted terminal
    // status transition is the 2nd.
    for _ in 0..100 {
        if backend.count(FilesystemOperation::WriteFile) >= 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process store did not attempt the terminal transition write");
}

pub(crate) async fn wait_for_no_reserved_processes(governor: &InMemoryResourceGovernor) {
    for _ in 0..100 {
        if governor.reserved_for(&sample_account()).process_count == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process reservation was not cleaned up");
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for AllowAllDispatchAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

pub(crate) struct ClientErrorMcpExecutor;

#[async_trait]
impl McpExecutor for ClientErrorMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::Client {
            reason: "simulated MCP client failure".to_string(),
        })
    }
}

pub(crate) struct PanicMcpExecutor;

#[async_trait]
impl McpExecutor for PanicMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        panic!("health-only test must not execute MCP runtime")
    }
}

pub(crate) fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    registry_with_manifests(&[manifest])
}

pub(crate) fn registry_with_host_bundled_manifest(manifest: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    let manifest = parse_manifest_from_source(manifest, ManifestSource::HostBundled);
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    registry.insert(package).unwrap();
    registry
}

pub(crate) fn registry_with_builtin_first_party_package() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

pub(crate) fn registry_with_manifests(manifests: &[&str]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    for manifest in manifests {
        let manifest = parse_manifest(manifest);
        let root =
            VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
        let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
        registry.insert(package).unwrap();
    }
    registry
}

pub(crate) fn parse_manifest(manifest: &str) -> ExtensionManifest {
    parse_manifest_from_source(manifest, ManifestSource::InstalledLocal)
}

pub(crate) fn parse_manifest_from_source(
    manifest: &str,
    source: ManifestSource,
) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        source,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap()
}

pub(crate) fn execution_context_without_grants() -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

pub(crate) fn execution_context_without_grants_for_scope(scope: ResourceScope) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(RunId::new()),
        origin: None,
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        authenticated_actor_user_id: None,
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

pub(crate) fn execution_context_with_dispatch_grant(capability: CapabilityId) -> ExecutionContext {
    let grants = capability_grants(capability);
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

pub(crate) fn execution_context_with_dispatch_grant_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
) -> ExecutionContext {
    execution_context_with_effect_grants_for_scope(
        capability,
        scope,
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )
}

pub(crate) fn execution_context_with_effect_grants_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
    allowed_effects: Vec<EffectKind>,
) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(RunId::new()),
        origin: None,
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::UserTrusted,
        grants: capability_grants_with_effects(capability, allowed_effects),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

pub(crate) fn capability_grants(capability: CapabilityId) -> CapabilitySet {
    capability_grants_with_effects(
        capability,
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )
}

pub(crate) fn capability_grants_with_effects(
    capability: CapabilityId,
    allowed_effects: Vec<EffectKind>,
) -> CapabilitySet {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
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

pub(crate) fn mount_view(alias: &str, target: &str, permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).unwrap(),
        VirtualPath::new(target).unwrap(),
        permissions,
    )])
    .unwrap()
}

pub(crate) fn local_manifest_trust_policy(
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

pub(crate) fn trust_decision_with_dispatch_authority() -> TrustDecision {
    trust_decision_with_authority(vec![EffectKind::DispatchCapability, EffectKind::Network])
}

pub(crate) fn trust_decision_with_authority(allowed_effects: Vec<EffectKind>) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

pub(crate) fn network_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

pub(crate) fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
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

pub(crate) fn hosted_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedDev,
        resolved_profile: RuntimeProfile::HostedDev,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Allowlist,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    }
}

pub(crate) fn assert_local_only_runtime_policy_rejected(
    runtime_policy: EffectiveRuntimePolicy,
    expected_implementation: &'static str,
) {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_policy(runtime_policy);

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("local-only runtime-policy field must not pass production validation");

    assert!(
        report.issues().iter().any(|issue| {
            issue.component() == ProductionWiringComponent::RuntimePolicy
                && issue.kind() == ProductionWiringIssueKind::LocalOnlyImplementation
                && issue.implementation() == Some(expected_implementation)
        }),
        "runtime policy should report {expected_implementation}: {report:?}"
    );
}

pub(crate) fn read_directory_text(root: &std::path::Path) -> String {
    let mut output = String::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = std::fs::read_dir(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                output.push_str(&std::fs::read_to_string(&path).unwrap_or_else(|err| {
                    panic!("failed to read {} as utf-8 text: {err}", path.display())
                }));
            }
        }
    }
    output
}

pub(crate) fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
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

pub(crate) fn process_start(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        authenticated_actor_user_id: None,
        extension_id: script_extension_id(),
        capability_id: script_capability_id(),
        runtime: RuntimeKind::Script,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        authorized_continuation: None,
        input: json!({"message": "running"}),
    }
}

pub(crate) fn process_sandbox_start(process_id: ProcessId, scope: ResourceScope) -> ProcessStart {
    let invocation_id = scope.invocation_id;
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        authenticated_actor_user_id: None,
        extension_id: ExtensionId::new("system.process_sandbox").unwrap(),
        capability_id: process_sandbox_capability_id(),
        runtime: RuntimeKind::System,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        authorized_continuation: None,
        input: process_sandbox_input(),
    }
}

pub(crate) fn process_sandbox_runtime_request_for_scope(
    scope: ResourceScope,
) -> RuntimeCapabilityRequest {
    RuntimeCapabilityRequest::new(
        execution_context_with_effect_grants_for_scope(
            process_sandbox_capability_id(),
            scope,
            process_sandbox_authority_effects(),
        ),
        process_sandbox_capability_id(),
        process_sandbox_estimate(),
        process_sandbox_input(),
    )
}

pub(crate) fn process_sandbox_estimate() -> ResourceEstimate {
    ResourceEstimate::default()
        .set_process_count(1)
        .set_concurrency_slots(1)
}

pub(crate) fn process_sandbox_input() -> serde_json::Value {
    json!({"run": {"command": "echo", "args": ["ok"]}})
}

pub(crate) fn invalid_process_sandbox_input() -> serde_json::Value {
    json!({"run": {"command": ""}})
}

pub(crate) fn process_sandbox_authority_effects() -> Vec<EffectKind> {
    vec![EffectKind::ExecuteCode, EffectKind::SpawnProcess]
}

pub(crate) fn process_sandbox_trust_decision() -> TrustDecision {
    trust_decision_with_authority(process_sandbox_authority_effects())
}

pub(crate) fn script_extension_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}

pub(crate) fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

pub(crate) fn mcp_capability_id() -> CapabilityId {
    CapabilityId::new("mcp.search").unwrap()
}

pub(crate) fn process_sandbox_capability_id() -> CapabilityId {
    CapabilityId::new("system.process_sandbox.run").unwrap()
}

pub(crate) struct WasmRuntimeFixture {
    pub(crate) runtime: DefaultHostRuntime,
    pub(crate) governor: Arc<InMemoryResourceGovernor>,
    pub(crate) http: Arc<RecordingRuntimeHttpEgress>,
    pub(crate) capability_id: CapabilityId,
}

pub(crate) struct WasmWallClockRuntimeFixture {
    pub(crate) runtime: DefaultHostRuntime,
    pub(crate) governor: Arc<InMemoryResourceGovernor>,
    pub(crate) http: Arc<RecordingRuntimeHttpEgress>,
    pub(crate) capability_id: CapabilityId,
}

pub(crate) async fn wasm_runtime_for_component(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmRuntimeFixture {
    let parsed_manifest = parse_manifest(manifest);
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy { policy },
        ]));
    let http = Arc::new(RecordingRuntimeHttpEgress::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&http))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();

    WasmRuntimeFixture {
        runtime: services.host_runtime_for_local_testing(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

pub(crate) async fn wasm_runtime_for_component_with_slow_zero_body_http(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmWallClockRuntimeFixture {
    let parsed_manifest = parse_manifest(manifest);
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy { policy },
        ]));
    let http = Arc::new(RecordingRuntimeHttpEgress::with_delay(
        Duration::from_millis(25),
    ));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&http))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();

    WasmWallClockRuntimeFixture {
        runtime: services.host_runtime_for_local_testing(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

pub(crate) async fn filesystem_with_wasm_component(
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

pub(crate) fn mounted_empty_extension_root() -> DiskFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = DiskFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

pub(crate) fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits::default()
                .set_max_concurrency_slots(10)
                .set_max_network_egress_bytes(10_000)
                .set_max_output_bytes(100_000),
        )
        .unwrap();
    governor
}

pub(crate) fn wasm_runtime_request(
    capability_id: CapabilityId,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let scope = sample_scope(InvocationId::new());
    wasm_runtime_request_for_scope(capability_id, scope, input)
}

pub(crate) fn wasm_runtime_request_for_scope(
    capability_id: CapabilityId,
    scope: ResourceScope,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let context = execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope);
    RuntimeCapabilityRequest::new(context, capability_id, wasm_http_estimate(), input)
}

pub(crate) fn wasm_http_estimate() -> ResourceEstimate {
    ResourceEstimate::default()
        .set_concurrency_slots(1)
        .set_network_egress_bytes(10)
        .set_output_bytes(10_000)
}

pub(crate) fn sample_account() -> ResourceAccount {
    ResourceAccount::tenant(TenantId::new("tenant-a").unwrap())
}

pub(crate) fn wasm_http_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

pub(crate) fn tool_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).unwrap();
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("tool.wit", include_str!("../../../../wit/tool.wit"))
        .unwrap();
    let world = resolve
        .select_world(&[package], Some("sandboxed-tool"))
        .unwrap();

    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();

    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true);
    encoder.encode().unwrap()
}

pub(crate) fn http_then_operation_failed_wat() -> String {
    HTTP_TOOL_WAT.replace(
        "i32.const 48\n    i32.const 1\n    i32.store\n    i32.const 52\n    i32.const 3072\n    i32.store\n    i32.const 56\n    i32.const 1\n    i32.store\n    i32.const 60\n    i32.const 0\n    i32.store\n    i32.const 48",
        "i32.const 48\n    i32.const 0\n    i32.store\n    i32.const 52\n    i32.const 0\n    i32.store\n    i32.const 56\n    i32.const 0\n    i32.store\n    i32.const 60\n    i32.const 1\n    i32.store\n    i32.const 64\n    i32.const 3072\n    i32.store\n    i32.const 68\n    i32.const 11\n    i32.store\n    i32.const 48",
    )
}

pub(crate) fn http_then_invalid_output_wat() -> String {
    HTTP_TOOL_WAT
        .replace(
            r#"(data (i32.const 3072) "1")"#,
            r#"(data (i32.const 3072) "not-json")"#,
        )
        .replace(
            "i32.const 56\n    i32.const 1\n    i32.store",
            "i32.const 56\n    i32.const 8\n    i32.store",
        )
}

pub(crate) fn http_without_body_then_operation_failed_wat() -> String {
    http_then_operation_failed_wat().replace(
        "i32.const 1\n    i32.const 256\n    i32.const 5",
        "i32.const 0\n    i32.const 0\n    i32.const 0",
    )
}

pub(crate) fn submit_turn_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        requested_model: None,
        scope: TurnScope::new(
            TenantId::new("tenant1").unwrap(),
            Some(AgentId::new("agent1").unwrap()),
            Some(ProjectId::new("project1").unwrap()),
            ThreadId::new(thread).unwrap(),
        ),
        actor: TurnActor::new(UserId::new("user1").unwrap()),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

// ─── Fix B: credential pre-flight ordering tests ─────────────────────────────
//
// These tests verify that `invoke_capability` surfaces `AuthRequired` BEFORE
// the approval gate fires when a required credential is absent. The canonical
// set of credential requirements is derived from the capability manifest via
// `capability_credential_requirements` — a single source of truth consumed by
// both the pre-flight check (ordering) and the dispatch-time obligation check
// (enforcement backstop).
//
// arch-exempt: large_file, credential preflight contract coverage,
// plan docs/plans/2026-06-12-approval-invocation-identity.md

/// Manifest for a script capability that declares a required runtime credential.
/// The `required = true` field (default) tells both the pre-flight check and
/// the obligation handler that the secret must be present.
pub(crate) const SCRIPT_WITH_CREDENTIAL_MANIFEST: &str = r#"
id = "script"
name = "Script With Credential"
version = "0.1.0"
description = "Script extension that requires a runtime credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
parameters_schema = { type = "object" }

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "script_api_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "x-api-key" }
required = true
"#;

pub(crate) const SCRIPT_MANIFEST: &str = r#"
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

pub(crate) const PROCESS_SANDBOX_MANIFEST: &str = r#"
id = "system.process_sandbox"
name = "Process Sandbox"
version = "0.1.0"
description = "System process sandbox runtime"
trust = "system_requested"

[runtime]
kind = "system"
service = "process_sandbox"

[[capabilities]]
id = "system.process_sandbox.run"
description = "Run a process inside the system sandbox backend"
effects = ["execute_code", "spawn_process"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

pub(crate) const SCRIPT_NETWORK_MANIFEST: &str = r#"
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
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const MCP_MANIFEST: &str = r#"
id = "mcp"
name = "MCP Search"
version = "0.1.0"
description = "MCP integration extension"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "mcp.search"
description = "Search through MCP"
effects = ["dispatch_capability", "network"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

pub(crate) const WASM_MANIFEST: &str = r#"
id = "wasm"
name = "WASM Count"
version = "0.1.0"
description = "WASM integration extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "tool.wasm"

[[capabilities]]
id = "wasm.count"
description = "Count through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const WASM_HTTP_SUCCESS_MANIFEST: &str = r#"
id = "wasm-http"
name = "WASM HTTP Success"
version = "0.1.0"
description = "WASM HTTP success extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/http-success.wasm"

[[capabilities]]
id = "wasm-http.success"
description = "Call host HTTP then return success"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const WASM_OPERATION_FAILED_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Operation Failed"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/operation-failed.wasm"

[[capabilities]]
id = "wasm-accounting.operation_failed"
description = "Call host HTTP then return an operation failure"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const WASM_INVALID_OUTPUT_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Invalid Output"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/invalid-output.wasm"

[[capabilities]]
id = "wasm-accounting.invalid_output"
description = "Call host HTTP then return invalid output"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const WASM_WALL_CLOCK_FAILURE_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Wall Clock Failure"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/wall-clock-failure.wasm"

[[capabilities]]
id = "wasm-accounting.wall_clock_failure"
description = "Spend wall-clock time through host HTTP then return an operation failure"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

pub(crate) const HTTP_TOOL_WAT: &str = r#"
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

fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
