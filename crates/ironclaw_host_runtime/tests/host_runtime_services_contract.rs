use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer, InMemoryCapabilityLeaseStore,
    TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest, CapabilitySpawnRequest,
};
use ironclaw_event_projections::{
    AuditProjectionError, AuditProjectionRequest, AuditProjectionService, AuditProjectionStage,
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionRequest, ProjectionScope,
    ReplayAuditProjectionService, ReplayEventProjectionService, RunProjectionStatus,
    TimelineEntryKind,
};
use ironclaw_events::{
    DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventCursor, EventError,
    EventReplay, EventStreamKey, InMemoryAuditSink, InMemoryDurableAuditLog,
    InMemoryDurableEventLog, InMemoryEventSink, ReadScope, RuntimeEventKind,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::ScopedFilesystem;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationHandler, BuiltinObligationServices, CancelReason, CancelRuntimeWorkRequest,
    CapabilitySurfaceVersion, DefaultHostRuntime, HostHttpEgressService, HostRuntime,
    HostRuntimeServices, ProcessObligationLifecycleStore, ProductionWiringComponent,
    ProductionWiringConfig, ProductionWiringIssueKind, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeFailureKind,
    RuntimeStatusRequest, RuntimeWorkId,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{
    BackgroundFailureStage, BackgroundProcessManager, InMemoryProcessResultStore,
    InMemoryProcessStore, ProcessError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessHost, ProcessManager, ProcessResultRecord, ProcessResultStore,
    ProcessServices, ProcessStart, ProcessStatus, ProcessStore,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStoreError, RebornProfile, build_reborn_event_stores,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, JsonFileResourceGovernorStore, PersistentResourceGovernor,
    ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits, ResourceTally,
};
use ironclaw_run_state::{
    ApprovalRecord, ApprovalRequestStore, InMemoryApprovalRequestStore, InMemoryRunStateStore,
    RunRecord, RunStart, RunStateApprovalStore, RunStateError, RunStateStore, RunStatus,
};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptExecutionRequest,
    ScriptExecutionResult, ScriptExecutor, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
#[cfg(feature = "libsql")]
use ironclaw_turns::FilesystemTurnStateStore;
#[cfg(feature = "libsql")]
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, InMemoryRunProfileResolver, ReplyTargetBindingRef,
    RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnScope, TurnStateStore,
};
use ironclaw_turns::{NoopTurnRunWakeNotifier, TurnRunWake, TurnRunWakeNotifier};
use ironclaw_wasm::{
    RecordingWasmHostHttp, WasmHostError, WasmHostHttp, WasmHttpRequest, WasmHttpResponse,
    WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest, WasmStagedRuntimeCredential,
    WasmStagedRuntimeCredentials, WitToolHost, WitToolRuntimeConfig,
};
use serde_json::json;
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

#[test]
fn production_wiring_validation_rejects_missing_components_and_local_only_defaults() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let report = match services.host_runtime_for_production(&ProductionWiringConfig::new([])) {
        Ok(_) => panic!("bare local/test service graph must not pass production validation"),
        Err(report) => report,
    };

    assert!(
        report.contains(
            ProductionWiringComponent::TrustPolicy,
            ProductionWiringIssueKind::Missing
        ),
        "missing explicit trust policy should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::RunState,
            ProductionWiringIssueKind::Missing
        ),
        "missing run-state store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::ApprovalRequests,
            ProductionWiringIssueKind::Missing
        ),
        "missing approval store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::CapabilityLeases,
            ProductionWiringIssueKind::Missing
        ),
        "missing capability lease store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::TurnState,
            ProductionWiringIssueKind::Missing
        ),
        "missing turn-state store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::RunProfileResolver,
            ProductionWiringIssueKind::Missing
        ),
        "missing run-profile resolver should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::TurnRunWakeNotifier,
            ProductionWiringIssueKind::Missing
        ),
        "missing turn wake notifier should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::Missing
        ),
        "missing event sink should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::Missing
        ),
        "missing audit sink should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::SecretStore,
            ProductionWiringIssueKind::Missing
        ),
        "missing secret store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::Filesystem,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "local filesystem should be reported as local-only: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::ResourceGovernor,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory resource governor should be reported as local-only: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::ProcessStore,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory process store should be reported as local-only: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::ProcessResultStore,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory process result store should be reported as local-only: {report:?}"
    );
}

#[test]
fn production_wiring_validation_accepts_persistent_resource_governor_component() {
    let dir = tempfile::tempdir().unwrap();
    let governor = Arc::new(PersistentResourceGovernor::new(
        JsonFileResourceGovernorStore::new(dir.path().join("resource-governor.json")),
    ));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        governor,
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local/test defaults still prevent production validation");

    assert!(
        !report.contains(
            ProductionWiringComponent::ResourceGovernor,
            ProductionWiringIssueKind::LocalOnlyImplementation,
        ),
        "persistent resource governor should satisfy resource guardrail: {report:?}"
    );
}

/// Filesystem-backed equivalent of the deleted libSQL/Postgres tests.
/// Backend choice is a `RootFilesystem` property; the `with_filesystem_resource_governor`
/// builder drives the same surface that the deleted SQL-specific builders
/// covered.
#[tokio::test]
async fn with_filesystem_resource_governor_persists_reservations_across_handles() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").unwrap(),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let scoped = Arc::new(ScopedFilesystem::new(Arc::clone(&backend), mounts));

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_resource_governor(Arc::clone(&scoped));

    let governor = services.resource_governor();
    let scope = sample_scope(InvocationId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    governor.release(reservation.id).unwrap();
}

#[tokio::test]
async fn with_filesystem_resource_governor_closes_process_reservations_on_cancel() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").unwrap(),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let scoped = Arc::new(ScopedFilesystem::new(Arc::clone(&backend), mounts));
    let process_services = ProcessServices::new(
        Arc::new(InMemoryProcessStore::new()),
        Arc::new(InMemoryProcessResultStore::new()),
    );
    let process_store = process_services.process_store();

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_resource_governor(Arc::clone(&scoped));
    let governor = services.resource_governor();
    let scope = sample_scope(InvocationId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let reservation_id = ResourceReservationId::new();
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
        .reserve_with_id(scope.clone(), estimate.clone(), reservation_id)
        .unwrap();
    let process_id = ProcessId::new();
    let mut start = process_start(process_id, scope.invocation_id, scope.clone());
    start.estimated_resources = estimate;
    start.resource_reservation_id = Some(reservation_id);
    process_store.start(start).await.unwrap();

    let runtime = services.host_runtime_for_local_testing();
    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert_eq!(
        governor.reserved_for(&account).unwrap(),
        ResourceTally::default()
    );
    assert!(matches!(
        governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[test]
fn production_wiring_validation_classifies_combined_store_as_run_state_and_approvals() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_local_only_run_state_approval_store(Arc::new(
        InMemoryRecordingCombinedRunStateApprovalStore::new(),
    ));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("local/test combined store must not pass production validation");

    assert!(
        report.contains(
            ProductionWiringComponent::RunState,
            ProductionWiringIssueKind::LocalOnlyImplementation,
        ),
        "combined store should be classified for run-state guardrails: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::ApprovalRequests,
            ProductionWiringIssueKind::LocalOnlyImplementation,
        ),
        "combined store should be classified for approval guardrails: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::RunState,
            ProductionWiringIssueKind::Missing,
        ),
        "combined store should satisfy run-state presence: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::ApprovalRequests,
            ProductionWiringIssueKind::Missing,
        ),
        "combined store should satisfy approval-store presence: {report:?}"
    );
}

#[test]
fn production_wiring_validation_rejects_unsupported_runtime_requirements() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::System]))
        .expect_err("system runtime requirements are not dispatcher backend requirements");

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeBackend,
            ProductionWiringIssueKind::UnsupportedRequirement
        ),
        "unsupported runtime backend requirement should be reported: {report:?}"
    );
}

// The legacy `LibSqlRunStateApprovalStore` / `PostgresRunStateApprovalStore`
// per-backend run-state + approval stores were deleted along with their
// `with_libsql_run_state_approval_store` /
// `with_postgres_run_state_approval_store` builder methods (see
// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`).
// Durability across reopen is now a property of the underlying
// `RootFilesystem` (`LibSqlRootFilesystem`, `PostgresRootFilesystem`, …)
// composed through `with_filesystem_run_state`; the run-state store layer
// no longer owns its own per-SQL persistence. The deleted tests were:
//
//   - `libsql_run_state_store_selection_satisfies_production_run_state_guardrails`
//   - `libsql_run_state_store_selection_persists_runtime_approval_block`
//
// The equivalent guardrail surface for the filesystem-backed wiring is
// exercised by `tests/reborn_durable_restart_integration.rs` (services
// graph restart over `LocalFilesystem`) and the `ironclaw_run_state`
// contract suite.

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_root_filesystem_selection_accepts_libsql_root_filesystem() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("root-filesystem.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.unwrap();

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_libsql_root_filesystem(Arc::clone(&filesystem));

    let path = VirtualPath::new("/engine/tenants/t1/users/u1/root-selection.txt").unwrap();
    filesystem.write_file(&path, b"selected").await.unwrap();
    assert_eq!(filesystem.read_file(&path).await.unwrap(), b"selected");

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local services remain intentionally unready");
    assert!(
        !report.contains(
            ProductionWiringComponent::Filesystem,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "LibSqlRootFilesystem must satisfy production filesystem selection: {report:?}"
    );
}

/// Construct an [`Arc<ScopedFilesystem<LibSqlRootFilesystem>>`] that exposes
/// the `/turns` mount alias over a libSQL-backed [`RootFilesystem`]. Mirrors
/// the production composition shape: the `/turns` alias rewrites to a
/// tenant/user-scoped target inside `/engine`, and the filesystem backend
/// supplies durable storage. Used by tests that previously constructed
/// `LibSqlTurnStateStore` directly.
#[cfg(feature = "libsql")]
async fn libsql_scoped_turns_fs(
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
    Arc::new(ScopedFilesystem::new(filesystem, view))
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_turn_state_selection_accepts_filesystem_turn_state_store() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("turn-state.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let scoped = libsql_scoped_turns_fs(Arc::clone(&db)).await;

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_turn_state_store(scoped);

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local services remain intentionally unready");
    assert!(
        !report.contains(
            ProductionWiringComponent::TurnState,
            ProductionWiringIssueKind::Missing
        ),
        "FilesystemTurnStateStore must satisfy production turn-state presence: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::TurnState,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "FilesystemTurnStateStore over LibSqlRootFilesystem must not be classified local-only: {report:?}"
    );
}

#[derive(Debug, Default)]
struct RecordingTurnRunWakeNotifier {
    wakes: Mutex<Vec<TurnRunWake>>,
}

impl RecordingTurnRunWakeNotifier {
    #[cfg(feature = "libsql")]
    fn wakes(&self) -> Vec<TurnRunWake> {
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

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_turn_coordinator_uses_configured_store_and_notifier() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("turn-coordinator.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let notifier = Arc::new(RecordingTurnRunWakeNotifier::default());
    let scoped = libsql_scoped_turns_fs(Arc::clone(&db)).await;

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_turn_state_store(Arc::clone(&scoped))
    .with_run_profile_resolver(Arc::new(InMemoryRunProfileResolver::default()))
    .with_turn_run_wake_notifier(Arc::clone(&notifier));

    let coordinator = services
        .turn_coordinator_for_production()
        .expect("production-ready turn wiring should build coordinator");
    let request = submit_turn_request("thread-production-turn-coordinator", "idem-production-turn");
    let response = coordinator.submit_turn(request.clone()).await.unwrap();
    let SubmitTurnResponse::Accepted { run_id, .. } = response;

    let reopened = FilesystemTurnStateStore::new(scoped);
    let state = reopened
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.run_id, run_id);
    assert_eq!(notifier.wakes().len(), 1);
    assert_eq!(notifier.wakes()[0].run_id, run_id);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_turn_coordinator_requires_explicit_run_profile_resolver() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("turn-coordinator-missing-resolver.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let scoped = libsql_scoped_turns_fs(Arc::clone(&db)).await;

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_turn_state_store(scoped)
    .with_turn_run_wake_notifier(Arc::new(RecordingTurnRunWakeNotifier::default()));

    let report = match services.turn_coordinator_for_production() {
        Ok(_) => panic!("production turn coordinator must fail closed without a resolver"),
        Err(report) => report,
    };
    assert!(report.contains(
        ProductionWiringComponent::RunProfileResolver,
        ProductionWiringIssueKind::Missing
    ));
}

#[test]
fn production_wiring_validation_rejects_noop_turn_wake_notifier() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_run_wake_notifier(Arc::new(NoopTurnRunWakeNotifier));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local services remain intentionally unready");
    assert!(
        report.contains(
            ProductionWiringComponent::TurnRunWakeNotifier,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "NoopTurnRunWakeNotifier must not satisfy production turn wake wiring: {report:?}"
    );
}

#[test]
fn production_wiring_validation_accepts_configured_turn_wake_notifier() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_turn_run_wake_notifier(Arc::new(RecordingTurnRunWakeNotifier::default()));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local services remain intentionally unready");
    assert!(
        !report.contains(
            ProductionWiringComponent::TurnRunWakeNotifier,
            ProductionWiringIssueKind::Missing
        ),
        "configured turn wake notifier must satisfy production presence: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::TurnRunWakeNotifier,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "configured turn wake notifier must not be classified local-only: {report:?}"
    );
}

#[tokio::test]
async fn production_event_store_config_rejects_jsonl_without_single_node_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let result = services
        .with_reborn_event_store_config(
            RebornProfile::Production,
            RebornEventStoreConfig::Jsonl {
                root: temp.path().join("reborn-event-store"),
                accept_single_node_durable: false,
            },
        )
        .await;

    assert!(matches!(
        result,
        Err(RebornEventStoreError::ProductionJsonlRequiresAcceptance)
    ));
}

#[tokio::test]
async fn local_reborn_event_store_config_does_not_satisfy_production_wiring() {
    let temp = tempfile::tempdir().unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_reborn_event_store_config(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: temp.path().join("local-reborn-event-store"),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("LocalDev stores are not production-verified event/audit sinks");

    assert!(
        report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "LocalDev Reborn event store must not satisfy production event sink guardrail: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "LocalDev Reborn audit store must not satisfy production audit sink guardrail: {report:?}"
    );
}

#[tokio::test]
async fn production_event_store_config_installs_verified_event_and_audit_sinks() {
    let temp = tempfile::tempdir().unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_reborn_event_store_config(
        RebornProfile::Production,
        RebornEventStoreConfig::Jsonl {
            root: temp.path().join("accepted-reborn-event-store"),
            accept_single_node_durable: true,
        },
    )
    .await
    .unwrap();

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local test services are still not production-ready");

    assert!(
        !report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::Missing
        ),
        "event sink must be installed from Reborn event store config: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::Missing
        ),
        "audit sink must be installed from Reborn event store config: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "Reborn durable event store adapter must not be treated as erased unverified sink: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "Reborn durable audit store adapter must not be treated as erased unverified sink: {report:?}"
    );
}

#[test]
fn production_wiring_validation_uses_configured_runtime_requirements() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );
    let config = ProductionWiringConfig::new([RuntimeKind::Script, RuntimeKind::Wasm])
        .require_runtime_http_egress()
        .require_wasm_credentials();

    let report = services
        .validate_production_wiring(&config)
        .expect_err("required runtime backends and egress must be reported when absent");

    assert!(
        report.contains(
            ProductionWiringComponent::ScriptRuntime,
            ProductionWiringIssueKind::Missing
        ),
        "missing script runtime should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::WasmRuntime,
            ProductionWiringIssueKind::Missing
        ),
        "missing wasm runtime should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeHttpEgress,
            ProductionWiringIssueKind::Missing
        ),
        "missing runtime HTTP egress should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::WasmCredentialProvider,
            ProductionWiringIssueKind::Missing
        ),
        "missing WASM credential provider should be reported: {report:?}"
    );
}

#[test]
fn production_wiring_validation_sees_underlying_in_memory_durable_logs() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_event_log(Arc::new(InMemoryDurableEventLog::new()))
    .with_durable_audit_log(Arc::new(InMemoryDurableAuditLog::new()));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("in-memory durable logs must not be hidden behind durable sink wrappers");

    assert!(
        report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory durable event log should be reported through with_durable_event_log: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory durable audit log should be reported through with_durable_audit_log: {report:?}"
    );
}

#[test]
fn production_wiring_validation_rejects_direct_durable_sink_wrappers_as_unverified() {
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let audit_log: Arc<dyn DurableAuditLog> = Arc::new(InMemoryDurableAuditLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_event_sink(Arc::new(DurableEventSink::new(event_log)))
    .with_audit_sink(Arc::new(DurableAuditSink::new(audit_log)));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("direct durable sink wrappers must not hide erased underlying log types");

    assert!(
        report.contains(
            ProductionWiringComponent::EventSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "direct durable event sink wrapper should require typed with_durable_event_log path: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "direct durable audit sink wrapper should require typed with_durable_audit_log path: {report:?}"
    );
}

#[test]
fn production_wiring_validation_accepts_verified_host_http_egress_shape() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()));
    let services = services
        .try_with_host_http_egress(RecordingNetworkHttpEgress::new())
        .unwrap();

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]).require_runtime_http_egress());

    assert!(
        report.as_ref().err().is_none_or(|report| !report.contains(
            ProductionWiringComponent::RuntimeHttpEgress,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        )),
        "verified host HTTP egress should satisfy the runtime egress guardrail: {report:?}"
    );
}

#[test]
fn host_http_egress_helper_requires_graph_secret_store() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let report = match services.try_with_host_http_egress(RecordingNetworkHttpEgress::new()) {
        Ok(_) => panic!("host HTTP egress helper must use configured graph secret store"),
        Err(report) => report,
    };

    assert!(report.contains(
        ProductionWiringComponent::SecretStore,
        ProductionWiringIssueKind::Missing
    ));
}

#[test]
fn production_wiring_validation_rejects_unverified_runtime_http_egress() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::new(
        HostHttpEgressService::new_with_request_policy_for_tests(
            RecordingNetworkHttpEgress::new(),
            InMemorySecretStore::new(),
        ),
    ));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]).require_runtime_http_egress())
        .expect_err(
            "generic/test runtime HTTP egress must not satisfy production egress guardrail",
        );

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeHttpEgress,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "runtime HTTP egress should require production verification: {report:?}"
    );
}

#[test]
fn production_wiring_validation_rejects_empty_verified_wasm_credentials() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_verified_wasm_runtime_credentials(Arc::new(WasmStagedRuntimeCredentials::new(vec![])))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();

    let report = services
        .validate_production_wiring(
            &ProductionWiringConfig::new([RuntimeKind::Wasm]).require_wasm_credentials(),
        )
        .expect_err("empty verified credential provider must not satisfy credential requirement");

    assert!(
        report.contains(
            ProductionWiringComponent::WasmCredentialProvider,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "empty WASM credentials should be reported as unverified: {report:?}"
    );
}

#[test]
fn production_wiring_validation_rejects_wasm_credentials_added_after_adapter() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap()
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![])));

    let report = services
        .validate_production_wiring(
            &ProductionWiringConfig::new([RuntimeKind::Wasm]).require_wasm_credentials(),
        )
        .expect_err(
            "credentials added after WASM adapter construction are not captured by the adapter",
        );

    assert!(
        report.contains(
            ProductionWiringComponent::WasmCredentialProvider,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "WASM credentials must be configured before adapter construction: {report:?}"
    );
}

#[test]
fn production_wiring_validation_rejects_wasm_credentials_replaced_after_adapter() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![])))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap()
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![])));

    let report = services
        .validate_production_wiring(
            &ProductionWiringConfig::new([RuntimeKind::Wasm]).require_wasm_credentials(),
        )
        .expect_err(
            "replacing credentials after WASM adapter construction is not captured by the adapter",
        );

    assert!(
        report.contains(
            ProductionWiringComponent::WasmCredentialProvider,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "WASM credentials must not be replaced after adapter construction: {report:?}"
    );
}

#[tokio::test]
async fn host_runtime_services_builds_dispatcher_runtime_and_health_from_registered_adapters() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(GrantAuthorizer::new());
    let process_services = ProcessServices::in_memory();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let events = InMemoryEventSink::new();
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));

    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_capability_leases(capability_leases)
    .with_script_runtime(script_runtime)
    .with_event_sink(Arc::new(events.clone()));

    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_with_dispatch_grant(script_capability_id());
    let request = RuntimeCapabilityRequest::new(
        context,
        script_capability_id(),
        ResourceEstimate::default(),
        json!({"message": "from services"}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, json!({"message": "from services"}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let health = runtime.health().await.unwrap();
    assert!(
        health.ready,
        "registered script adapter should make health ready"
    );
    assert!(health.missing_runtime_backends.is_empty());
    let kinds = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
}

#[tokio::test]
async fn host_runtime_services_wires_combined_store_for_atomic_approval_block() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    assert_services_use_combined_store_for_atomic_approval_block(
        services,
        "approval from services",
    )
    .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn host_runtime_services_preserves_combined_store_after_root_filesystem_selection() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir
        .path()
        .join("root-filesystem-preserves-combined-store.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_libsql_root_filesystem(filesystem);

    assert_services_use_combined_store_for_atomic_approval_block(
        services,
        "approval after root filesystem selection",
    )
    .await;
}

async fn assert_services_use_combined_store_for_atomic_approval_block<
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
            trust_decision_with_dispatch_authority(),
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

#[tokio::test]
async fn host_runtime_services_writes_runtime_events_to_durable_event_log_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(GrantAuthorizer::new());
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_durable_event_log(Arc::clone(&event_log))
    .with_script_runtime(script_runtime);
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "RAW_EVENT_INPUT_SENTINEL_3147 /tmp/private-event-path",
        "secret": "SECRET_EVENT_SENTINEL_3147_sk_live_secret",
        "output": "RUNTIME_EVENT_OUTPUT_SENTINEL_3147",
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

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.output, payload);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }

    let replay = event_log
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
    assert_eq!(
        replay.entries[2].record.output_bytes,
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let serialized = serde_json::to_string(&replay).unwrap();
    for forbidden in [
        "RAW_EVENT_INPUT_SENTINEL_3147",
        "/tmp/private-event-path",
        "SECRET_EVENT_SENTINEL_3147",
        "RUNTIME_EVENT_OUTPUT_SENTINEL_3147",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable runtime event replay leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("script.echo"));
    assert!(serialized.contains("dispatch_requested"));
    assert!(serialized.contains("dispatch_succeeded"));
}

#[tokio::test]
async fn host_runtime_services_consumes_reborn_jsonl_event_store_without_v1_composition() {
    let temp = tempfile::tempdir().unwrap();
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: temp.path().join("reborn-event-store"),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let event_log = Arc::clone(&stores.events);
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(script_runtime)
    .with_event_sink(Arc::new(DurableEventSink::new(Arc::clone(&event_log))));

    let scope = sample_scope(InvocationId::new());
    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "from jsonl store"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        RuntimeCapabilityOutcome::Completed(completed)
            if completed.output == json!({"message": "from jsonl store"})
    ));

    let replay = event_log
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
}

#[tokio::test]
async fn host_runtime_services_durable_event_replay_cursor_and_gap_behavior() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_durable_event_log(Arc::clone(&event_log))
    .with_script_runtime(script_runtime);
    let scope = sample_scope(InvocationId::new());
    let stream = EventStreamKey::from_scope(&scope);

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "cursor replay"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, json!({"message": "cursor replay"}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let first_page = event_log
        .read_after_cursor(&stream, &ReadScope::any(), None, 1)
        .await
        .unwrap();
    assert_eq!(first_page.entries.len(), 1);
    assert_eq!(
        first_page.entries[0].record.kind,
        RuntimeEventKind::DispatchRequested
    );
    let second_page = event_log
        .read_after_cursor(&stream, &ReadScope::any(), Some(first_page.next_cursor), 10)
        .await
        .unwrap();
    assert_eq!(second_page.entries.len(), 2);
    assert_eq!(
        second_page
            .entries
            .iter()
            .map(|entry| entry.record.kind)
            .collect::<Vec<_>>(),
        vec![
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    let empty_page = event_log
        .read_after_cursor(
            &stream,
            &ReadScope::any(),
            Some(second_page.next_cursor),
            10,
        )
        .await
        .unwrap();
    assert!(empty_page.entries.is_empty());
    assert_eq!(empty_page.next_cursor, second_page.next_cursor);

    event_log
        .truncate_before_or_at(&stream, first_page.next_cursor)
        .unwrap();
    let gap = event_log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::origin()), 10)
        .await
        .expect_err("origin cursor should be stale after retention truncation");
    assert!(matches!(gap, EventError::ReplayGap { .. }));
}

#[tokio::test]
async fn host_runtime_services_runtime_events_project_through_replay_projection_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let services = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_durable_event_log(Arc::clone(&event_log))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "RAW_PROJECTION_INPUT_SENTINEL_3022 /tmp/private-projection-path",
        "secret": "SECRET_PROJECTION_SENTINEL_3022_sk_live_secret",
        "output": "RUNTIME_PROJECTION_OUTPUT_SENTINEL_3022",
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

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(completed) if completed.output == payload)
    );

    let projection = ReplayEventProjectionService::new(Arc::clone(&event_log));
    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.runs[0].capability_id, script_capability_id());
    assert_eq!(
        snapshot.timeline.entries[2].output_bytes,
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let serialized = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        "RAW_PROJECTION_INPUT_SENTINEL_3022",
        "/tmp/private-projection-path",
        "SECRET_PROJECTION_SENTINEL_3022",
        "RUNTIME_PROJECTION_OUTPUT_SENTINEL_3022",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "runtime projection leaked {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_projection_rejects_foreign_cursor_and_surfaces_rebase_after_gap() {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_durable_event_log(Arc::clone(&event_log))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let scope_a = sample_scope(InvocationId::new());
    let scope_b = ResourceScope {
        thread_id: Some(ThreadId::new("thread-b").unwrap()),
        invocation_id: InvocationId::new(),
        ..scope_a.clone()
    };

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(
                script_capability_id(),
                scope_a.clone(),
            ),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "scope a"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(matches!(outcome, RuntimeCapabilityOutcome::Completed(_)));

    let projection = ReplayEventProjectionService::new(Arc::clone(&event_log));
    let scope_a_projection = ProjectionScope::from_resource_scope(&scope_a);
    let scope_b_projection = ProjectionScope::from_resource_scope(&scope_b);
    let snapshot_a = projection
        .snapshot(ProjectionRequest {
            scope: scope_a_projection.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    let snapshot_b = projection
        .snapshot(ProjectionRequest {
            scope: scope_b_projection.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(snapshot_b.timeline.entries.is_empty());

    let foreign_cursor = projection
        .updates(ProjectionRequest {
            scope: scope_b_projection,
            after: Some(snapshot_a.next_cursor.clone()),
            limit: 10,
        })
        .await
        .expect_err("foreign projection cursor must force rebase");
    assert!(matches!(
        foreign_cursor,
        ProjectionError::RebaseRequired { .. }
    ));

    event_log
        .truncate_before_or_at(
            &EventStreamKey::from_scope(&scope_a),
            snapshot_a.timeline.entries[0].cursor,
        )
        .unwrap();
    let stale_cursor = projection
        .updates(ProjectionRequest {
            scope: scope_a_projection.clone(),
            after: Some(ProjectionCursor::origin_for_scope(scope_a_projection)),
            limit: 10,
        })
        .await
        .expect_err("retained-history gap must force projection rebase");
    assert!(matches!(
        stale_cursor,
        ProjectionError::RebaseRequired { .. }
    ));
}

#[tokio::test]
async fn host_runtime_services_jsonl_event_store_projects_same_runtime_sequence_without_sentinels()
{
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let event_log = Arc::clone(&stores.events);
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_event_sink(Arc::new(DurableEventSink::new(Arc::clone(&event_log))))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "JSONL_RAW_INPUT_SENTINEL_3022 /tmp/jsonl-private-path",
        "secret": "JSONL_SECRET_SENTINEL_3022_sk_live_secret",
        "output": "JSONL_OUTPUT_SENTINEL_3022",
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
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(completed) if completed.output == payload)
    );

    let projection = ReplayEventProjectionService::from_runtime_log(Arc::clone(&event_log));
    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "JSONL_RAW_INPUT_SENTINEL_3022",
        "/tmp/jsonl-private-path",
        "JSONL_SECRET_SENTINEL_3022",
        "JSONL_OUTPUT_SENTINEL_3022",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "JSONL-backed projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "JSONL durable event bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_approval_resolution_projects_durable_audit_metadata_only() {
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(SentinelApprovalAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_durable_audit_log(Arc::clone(&audit_log))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let runtime = services.host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let context = execution_context_without_grants_for_scope(scope.clone());
    let input = json!({
        "message": "APPROVAL_RAW_INPUT_SENTINEL_3022 /tmp/private-approval-path",
        "secret": "APPROVAL_SECRET_SENTINEL_3022_sk_live_secret",
        "output": "APPROVAL_OUTPUT_SENTINEL_3022",
    });

    let gate = block_for_approval(
        &runtime,
        context.clone(),
        ResourceEstimate::default(),
        input.clone(),
    )
    .await;
    approve_dispatch_for_services(&services, &scope, gate.approval_request_id, None).await;
    let resumed = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            ResourceEstimate::default(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(resumed, RuntimeCapabilityOutcome::Completed(completed) if completed.output == input)
    );

    let projection = ReplayAuditProjectionService::new(Arc::clone(&audit_log));
    let snapshot = projection
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    let entry = &snapshot.entries[0];
    assert_eq!(entry.stage, AuditProjectionStage::ApprovalResolved);
    assert_eq!(entry.invocation_id, scope.invocation_id);
    assert_eq!(entry.thread_id, scope.thread_id);
    assert_eq!(entry.approval_request_id, Some(gate.approval_request_id));
    assert_eq!(entry.action_kind, "dispatch");
    assert_eq!(
        entry.action_target.as_deref(),
        Some(script_capability_id().as_str())
    );
    assert_eq!(entry.decision_kind, "approved");

    let serialized = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        "APPROVAL_REASON_SENTINEL_3022",
        "APPROVAL_RAW_INPUT_SENTINEL_3022",
        "/tmp/private-approval-path",
        "APPROVAL_SECRET_SENTINEL_3022",
        "APPROVAL_OUTPUT_SENTINEL_3022",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "approval audit projection leaked {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_jsonl_approval_audit_projection_rejects_foreign_cursor_without_leaks()
 {
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let audit_log = Arc::clone(&stores.audit);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(SentinelApprovalAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_audit_sink(Arc::new(ironclaw_events::DurableAuditSink::new(
        Arc::clone(&audit_log),
    )))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let runtime = services.host_runtime_for_local_testing();
    let scope_a = sample_scope(InvocationId::new());
    let scope_b = ResourceScope {
        thread_id: Some(ThreadId::new("approval-thread-b").unwrap()),
        invocation_id: InvocationId::new(),
        ..scope_a.clone()
    };
    let context = execution_context_without_grants_for_scope(scope_a.clone());
    let input = json!({"message": "JSONL_APPROVAL_INPUT_SENTINEL_3022"});

    let gate = block_for_approval(
        &runtime,
        context.clone(),
        ResourceEstimate::default(),
        input.clone(),
    )
    .await;
    approve_dispatch_for_services(&services, &scope_a, gate.approval_request_id, None).await;

    let projection = ReplayAuditProjectionService::from_audit_log(Arc::clone(&audit_log));
    let scope_a_projection = ProjectionScope::from_resource_scope(&scope_a);
    let scope_b_projection = ProjectionScope::from_resource_scope(&scope_b);
    let snapshot_a = projection
        .snapshot(AuditProjectionRequest {
            scope: scope_a_projection,
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(snapshot_a.entries.len(), 1);
    let snapshot_b = projection
        .snapshot(AuditProjectionRequest {
            scope: scope_b_projection.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(snapshot_b.entries.is_empty());

    let foreign_cursor = projection
        .updates(AuditProjectionRequest {
            scope: scope_b_projection,
            after: Some(snapshot_a.next_cursor.clone()),
            limit: 10,
        })
        .await
        .expect_err("foreign audit projection cursor must force rebase");
    assert!(matches!(
        foreign_cursor,
        AuditProjectionError::RebaseRequired { .. }
    ));

    let projection_json = serde_json::to_string(&snapshot_a).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "APPROVAL_REASON_SENTINEL_3022",
        "JSONL_APPROVAL_INPUT_SENTINEL_3022",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "JSONL approval audit projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "JSONL durable audit bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

#[tokio::test]
async fn process_lifecycle_projects_through_durable_replay_without_output_leaks() {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let inner_process_store = Arc::new(InMemoryProcessStore::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let process_store =
        Arc::new(obligation_services.process_obligation_lifecycle_store(inner_process_store));
    let durable_event_log: Arc<dyn DurableEventLog> = event_log.clone();
    process_store.set_event_sink(Arc::new(DurableEventSink::new(durable_event_log)));
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let manager = BackgroundProcessManager::new(
        Arc::clone(&process_store),
        Arc::new(BackgroundExecutor::success_with_output(json!({
            "result": "PROCESS_OUTPUT_SENTINEL_3022 /tmp/process-output-private"
        }))),
    )
    .with_result_store(Arc::clone(&result_store));
    let process_id = ProcessId::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);

    let process = manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    wait_for_status(
        process_store.as_ref(),
        &scope,
        process.process_id,
        ProcessStatus::Completed,
    )
    .await;

    let host =
        ProcessHost::new(process_store.as_ref()).with_result_store(Arc::clone(&result_store));
    let output = host
        .output(&scope, process.process_id)
        .await
        .unwrap()
        .expect("process output should be available through ProcessHost");
    assert_eq!(
        output,
        json!({"result": "PROCESS_OUTPUT_SENTINEL_3022 /tmp/process-output-private"})
    );

    let projection = ReplayEventProjectionService::new(Arc::clone(&event_log));
    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&scope, process.process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ProcessStarted,
            TimelineEntryKind::ProcessCompleted,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.runs[0].process_id, Some(process.process_id));

    let foreign_scope = ResourceScope {
        project_id: Some(ProjectId::new("foreign-project").unwrap()),
        ..scope.clone()
    };
    let foreign_snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&foreign_scope, process.process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(foreign_snapshot.timeline.entries.is_empty());

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let replay_json = serde_json::to_string(
        &event_log
            .read_after_cursor(
                &EventStreamKey::from_scope(&scope),
                &ReadScope::any(),
                None,
                10,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    for forbidden in [
        "PROCESS_OUTPUT_SENTINEL_3022",
        "/tmp/process-output-private",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "process projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !replay_json.contains(forbidden),
            "process durable replay leaked {forbidden}: {replay_json}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_cancel_projects_kill_event_from_configured_event_sink() {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let process_services = ProcessServices::new(
        Arc::new(InMemoryProcessStore::new()),
        Arc::new(InMemoryProcessResultStore::new()),
    );
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_event_log(Arc::clone(&event_log))
    .host_runtime_for_local_testing();
    let process_id = ProcessId::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.input = json!({
        "message": "KILL_PROCESS_INPUT_SENTINEL_3022 /tmp/process-kill-private"
    });
    process_store.start(start).await.unwrap();

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .unwrap();
    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert_eq!(
        result_store
            .get(&scope, process_id)
            .await
            .unwrap()
            .expect("cancel should persist killed process result")
            .status,
        ProcessStatus::Killed
    );

    let projection = ReplayEventProjectionService::new(Arc::clone(&event_log));
    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&scope, process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert_eq!(
        snapshot.timeline.entries[0].kind,
        TimelineEntryKind::ProcessKilled
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Killed);

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let replay_json = serde_json::to_string(
        &event_log
            .read_after_cursor(
                &EventStreamKey::from_scope(&scope),
                &ReadScope::any(),
                None,
                10,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    for forbidden in [
        "KILL_PROCESS_INPUT_SENTINEL_3022",
        "/tmp/process-kill-private",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "kill projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !replay_json.contains(forbidden),
            "kill durable replay leaked {forbidden}: {replay_json}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_resumes_approved_capability_and_consumes_lease_once() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approval resume"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;

    let resumed = runtime
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
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Consumed
    );
    let kinds = fixture
        .events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );

    let second = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(second, RuntimeFailureKind::Authorization);
    assert_eq!(
        fixture.events.events().len(),
        3,
        "second resume must fail before a second dispatch"
    );
}

#[tokio::test]
async fn host_runtime_services_resume_changed_input_fails_before_lease_claim_or_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let original_input = json!({"message": "original"});

    let gate =
        block_for_approval(&runtime, context.clone(), estimate.clone(), original_input).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            json!({"message": "changed"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(fixture.events.events().is_empty());
    // The approval request stores the original invocation fingerprint; changed input
    // computes a different resume fingerprint, so no matching lease is claimable.
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "fingerprint mismatch must fail before lease claim/consume"
    );
}

#[tokio::test]
async fn host_runtime_services_resume_wrong_user_scope_is_hidden_before_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "wrong user"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let wrong_scope = ResourceScope {
        user_id: UserId::new("other-user").unwrap(),
        ..scope.clone()
    };
    let wrong_context =
        execution_context_with_dispatch_grant_for_scope(script_capability_id(), wrong_scope);

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            wrong_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert!(fixture.events.events().is_empty());
    let original_run = fixture
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original_run.status, RunStatus::BlockedApproval);
    assert_eq!(
        original_run.approval_request_id,
        Some(gate.approval_request_id)
    );
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn host_runtime_services_resume_expired_lease_fails_before_dispatch() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "expired"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease = approve_dispatch_for_services(
        &fixture.services,
        &scope,
        gate.approval_request_id,
        Some(Utc::now() - ChronoDuration::seconds(1)),
    )
    .await;

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(fixture.events.events().is_empty());
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn host_runtime_services_resume_trust_preflight_failure_fails_only_matching_blocked_run() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "stale trust metadata"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    let wrong_scope = ResourceScope {
        user_id: UserId::new("other-user").unwrap(),
        ..scope.clone()
    };
    let wrong_context = execution_context_without_grants_for_scope(wrong_scope);
    let wrong_scope_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            wrong_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(wrong_scope_outcome, RuntimeFailureKind::MissingRuntime);
    assert_blocked_approval_run(
        &fixture,
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;

    let mut invalid_context = context.clone();
    invalid_context.user_id = UserId::new("tampered-user").unwrap();
    let invalid_context_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            invalid_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(invalid_context_outcome, RuntimeFailureKind::MissingRuntime);
    assert_blocked_approval_run(
        &fixture,
        &scope,
        context.invocation_id,
        gate.approval_request_id,
    )
    .await;

    let matching_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(matching_outcome, RuntimeFailureKind::MissingRuntime);

    let failed_run = fixture
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed_run.status, RunStatus::Failed);
    assert_eq!(failed_run.approval_request_id, None);
    assert_eq!(failed_run.error_kind.as_deref(), Some("unknown_capability"));
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "trust preflight failure must not claim or consume the approval lease"
    );
    assert!(fixture.events.events().is_empty());
}

#[tokio::test]
async fn host_runtime_services_resume_without_backing_stores_fails_closed() {
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .host_runtime_for_local_testing();

    let outcome = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            execution_context_without_grants(),
            ApprovalRequestId::new(),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "missing stores"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn host_runtime_services_registered_runtime_health_tracks_script_mcp_and_wasm_adapters() {
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifests(&[
            SCRIPT_MANIFEST,
            MCP_MANIFEST,
            WASM_MANIFEST,
        ])),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_script_runtime(script_runtime)
    .with_mcp_runtime(Arc::new(PanicMcpExecutor))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap()
    .host_runtime_for_local_testing();

    let health = runtime.health().await.unwrap();

    assert!(health.ready);
    assert!(health.missing_runtime_backends.is_empty());
}

#[tokio::test]
async fn host_runtime_services_health_fails_closed_for_unregistered_required_runtime() {
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime_for_local_testing();

    let health = runtime.health().await.unwrap();

    assert!(!health.ready);
    assert_eq!(health.missing_runtime_backends, vec![RuntimeKind::Script]);
}

#[tokio::test]
async fn host_runtime_services_installs_builtin_obligation_handler_with_audit_sink() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let audit = Arc::new(InMemoryAuditSink::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![Obligation::AuditBefore]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_audit_sink(Arc::clone(&audit))
    .with_script_runtime(script_runtime);

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(script_capability_id()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "audited through services"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(
                completed.output,
                json!({"message": "audited through services"})
            );
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::Before);
    assert_eq!(records[0].action.target.as_deref(), Some("script.echo"));
}

#[tokio::test]
async fn host_runtime_services_maps_script_exit_failure_through_private_adapter() {
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        ExitFailureScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(Vec::new())),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_script_runtime(script_runtime);

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(script_capability_id()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "fail through services"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Process);
}

#[tokio::test]
async fn host_runtime_services_maps_mcp_client_failure_through_private_adapter() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(MCP_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(Vec::new())),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_mcp_runtime(Arc::new(ClientErrorMcpExecutor));

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(mcp_capability_id()),
            mcp_capability_id(),
            ResourceEstimate::default(),
            json!({"query": "fail through services"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn host_runtime_services_applies_scoped_mount_obligation_to_script_runtime() {
    let scoped_mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_only(),
    );
    let mut context = execution_context_with_dispatch_grant(script_capability_id());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::UseScopedMounts {
                mounts: scoped_mounts.clone(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(Arc::clone(&script_runtime));

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "mount narrowed"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, json!({"message": "mount narrowed"}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    assert_eq!(script_runtime.recorded_mounts(), vec![Some(scoped_mounts)]);
}

#[tokio::test]
async fn host_runtime_services_rejects_broader_scoped_mount_before_dispatch() {
    let mut context = execution_context_with_dispatch_grant(script_capability_id());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_only(),
    );
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::UseScopedMounts {
                mounts: mount_view(
                    "/workspace",
                    "/projects/demo",
                    MountPermissions::read_write(),
                ),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(Arc::clone(&script_runtime));

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "broader mount"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(
        script_runtime.recorded_mounts().is_empty(),
        "broader mount obligation must fail before runtime dispatch"
    );
}

#[tokio::test]
async fn host_runtime_services_writes_obligation_audit_records_to_durable_log_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::AuditBefore,
            Obligation::AuditAfter,
        ]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_audit_log(Arc::clone(&audit_log))
    .with_script_runtime(script_runtime);
    let scope = sample_scope(InvocationId::new());
    let payload = json!({
        "message": "RAW_INPUT_SENTINEL_3147 /tmp/private-host-path",
        "secret": "SECRET_SENTINEL_3147_sk_live_secret",
        "output": "RUNTIME_OUTPUT_SENTINEL_3147",
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

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.output, payload);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let replay = audit_log
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].record.stage, AuditStage::Before);
    assert_eq!(replay.entries[1].record.stage, AuditStage::After);
    assert_eq!(
        replay.entries[1]
            .record
            .result
            .as_ref()
            .and_then(|result| result.output_bytes),
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let serialized = serde_json::to_string(&replay).unwrap();
    for forbidden in [
        "RAW_INPUT_SENTINEL_3147",
        "/tmp/private-host-path",
        "SECRET_SENTINEL_3147",
        "RUNTIME_OUTPUT_SENTINEL_3147",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable obligation audit replay leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("script.echo"));
    assert!(serialized.contains("audit_before"));
    assert!(serialized.contains("audit_after"));
}

#[tokio::test]
async fn host_runtime_services_projects_resource_network_secret_obligation_audit_metadata_only() {
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let audit_log = Arc::clone(&stores.audit);
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("obligation-api-token").unwrap();
    let reservation_id = ResourceReservationId::new();
    let policy = NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "NETWORK_POLICY_SENTINEL_3022.example.test".to_string(),
            port: Some(443),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    };
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::AuditBefore,
            Obligation::ApplyNetworkPolicy { policy },
            Obligation::InjectSecretOnce {
                handle: secret_handle.clone(),
            },
            Obligation::ReserveResources { reservation_id },
            Obligation::AuditAfter,
        ]));
    let services: InMemoryHostRuntimeServices = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )))
    .with_secret_store(Arc::clone(&secret_store))
    .with_audit_sink(Arc::new(DurableAuditSink::new(Arc::clone(&audit_log))))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )));
    let scope = sample_scope(InvocationId::new());
    secret_store
        .put(
            scope.clone(),
            secret_handle,
            SecretMaterial::from("SECRET_MATERIAL_SENTINEL_3022_sk_live_secret"),
        )
        .await
        .unwrap();
    let payload = json!({
        "message": "OBLIGATION_INPUT_SENTINEL_3022 /tmp/private-obligation-path",
        "output": "OBLIGATION_OUTPUT_SENTINEL_3022",
    });

    let runtime = services.host_runtime_for_local_testing();
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                network_egress_bytes: Some(10),
                output_bytes: Some(100),
                ..ResourceEstimate::default()
            },
            payload.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(completed) if completed.output == payload)
    );

    let projection = ReplayAuditProjectionService::from_audit_log(Arc::clone(&audit_log));
    let snapshot = projection
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].stage, AuditProjectionStage::Before);
    assert_eq!(snapshot.entries[1].stage, AuditProjectionStage::After);
    let mut status_labels = snapshot.entries[0]
        .result_status
        .as_deref()
        .unwrap()
        .split(',')
        .collect::<Vec<_>>();
    status_labels.sort_unstable();
    assert_eq!(
        status_labels,
        vec![
            "apply_network_policy",
            "audit_after",
            "audit_before",
            "inject_secret_once",
            "reserve_resources",
        ]
    );
    assert_eq!(
        snapshot.entries[1].output_bytes,
        Some(serde_json::to_vec(&payload).unwrap().len() as u64)
    );

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "NETWORK_POLICY_SENTINEL_3022",
        "SECRET_MATERIAL_SENTINEL_3022",
        "OBLIGATION_INPUT_SENTINEL_3022",
        "/tmp/private-obligation-path",
        "OBLIGATION_OUTPUT_SENTINEL_3022",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "obligation audit projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "durable obligation audit bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

#[tokio::test]
async fn host_runtime_services_enforces_output_limit_and_reconciles_resource_usage() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope(InvocationId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                max_output_bytes: Some(10_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let reservation_id = ResourceReservationId::new();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ReserveResources { reservation_id },
            Obligation::EnforceOutputLimit { bytes: 8 },
        ]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_run_state(Arc::clone(&run_state))
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_script_runtime(script_runtime);
    let input = json!({"message": "this output is deliberately too large"});

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(1024),
                ..ResourceEstimate::default()
            },
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::OutputTooLarge);
    assert_eq!(governor.reserved_for(&account), Default::default());
    assert!(
        governor.usage_for(&account).output_bytes > 8,
        "runtime usage should remain reconciled even when post-dispatch output limit blocks publication"
    );
    let run = run_state
        .get(&scope, scope.invocation_id)
        .await
        .unwrap()
        .expect("run state should record the failed invocation");
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ObligationFailed"));
}

#[tokio::test]
async fn host_runtime_services_releases_reservation_when_dispatch_preflight_fails_after_obligations()
 {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope(InvocationId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let reservation_id = ResourceReservationId::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ReserveResources { reservation_id },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_run_state(Arc::clone(&run_state))
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )));

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone()),
            script_capability_id(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            json!({"message": "missing runtime after reservation"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::MissingRuntime);
    assert_eq!(governor.reserved_for(&account), Default::default());
    assert!(matches!(
        governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
    let run = run_state
        .get(&scope, scope.invocation_id)
        .await
        .unwrap()
        .expect("run state should record the failed invocation");
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("Dispatch"));
}

#[tokio::test]
async fn host_runtime_services_fails_closed_when_durable_obligation_audit_append_fails() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![Obligation::AuditBefore]));
    let script_runtime = Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    ));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_durable_audit_log(Arc::new(FailingDurableAuditLog))
    .with_script_runtime(script_runtime);

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context_with_dispatch_grant(script_capability_id()),
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "must not dispatch after audit append failure"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::Backend);
            let message = failure.message.unwrap_or_default();
            assert!(message.contains("obligation handling failed: Audit"));
            assert!(
                !message.contains("/tmp/audit-backend-secret"),
                "audit backend details must remain sanitized: {message}"
            );
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn host_runtime_services_routes_wasm_http_through_per_invocation_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
        ]));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let scope = sample_scope(InvocationId::new());

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope.clone(),
            json!({"call": "http-success"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output, json!(1));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].network_policy, policy);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(requests[0].url, "https://example.test/api");
    assert_eq!(requests[0].body, b"hello".to_vec());
}

#[tokio::test]
async fn host_runtime_services_routes_cached_wasm_http_through_per_invocation_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
        ]));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
    .unwrap();
    let runtime = services.host_runtime_for_local_testing();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let first_scope = sample_scope(InvocationId::new());
    let second_scope = sample_scope(InvocationId::new());

    let first = runtime
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            first_scope.clone(),
            json!({"call": "http-success-first"}),
        ))
        .await
        .unwrap();
    let second = runtime
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            second_scope.clone(),
            json!({"call": "http-success-second"}),
        ))
        .await
        .unwrap();

    assert_completed_outcome(first, &capability_id);
    assert_completed_outcome(second, &capability_id);
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].scope, first_scope);
    assert_eq!(requests[1].scope, second_scope);
    assert_eq!(requests[0].network_policy, policy);
    assert_eq!(requests[1].network_policy, policy);
}

#[tokio::test]
async fn host_runtime_services_wasm_http_uses_production_staged_network_and_secret_handoffs() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("api-token").unwrap();
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectSecretOnce {
                handle: secret_handle.clone(),
            },
        ]));
    let network = RecordingNetworkHttpEgress::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_exact_url(
            secret_handle.clone(),
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            true,
            "https://example.test/api".to_string(),
        ),
    ])));
    let services = services
        .try_with_host_http_egress(network.clone())
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
        .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let scope = sample_scope(InvocationId::new());
    secret_store
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from("sk-vertical-secret"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope.clone(),
            json!({"call": "http-success-with-secret"}),
        ))
        .await
        .unwrap();

    assert_completed_outcome(outcome, &capability_id);
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].policy, policy);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-vertical-secret".to_string(),
        ))
    );
    // The consumed-staged-secret one-shot invariant is covered by
    // `reborn_e2e_gate_host_http_consumes_staged_policy_and_secret_once`.
}

#[tokio::test]
async fn host_runtime_services_wasm_http_secret_store_lease_uses_graph_secret_store() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("api-token").unwrap();
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
        ]));
    let network = RecordingNetworkHttpEgress::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_wasm_runtime_credential_provider(Arc::new(SecretStoreLeaseCredentials {
        handle: secret_handle.clone(),
    }));
    let services = services
        .try_with_host_http_egress(network.clone())
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
        .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();
    let scope = sample_scope(InvocationId::new());
    secret_store
        .put(
            scope.clone(),
            secret_handle,
            SecretMaterial::from("sk-graph-store-secret"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"call": "http-success-with-secret-store-lease"}),
        ))
        .await
        .unwrap();

    assert_completed_outcome(outcome, &capability_id);
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].policy, policy);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-graph-store-secret".to_string(),
        ))
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_http_missing_staged_secret_stays_before_transport() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let secret_handle = SecretHandle::new("api-token").unwrap();
    let policy = wasm_http_policy();
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy { policy },
        ]));
    let network = RecordingNetworkHttpEgress::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_exact_url(
            secret_handle,
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            true,
            "https://example.test/api".to_string(),
        ),
    ])));
    let services = services
        .try_with_host_http_egress(network.clone())
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::for_testing(), WitToolHost::deny_all())
        .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request(
            capability_id.clone(),
            json!({"call": "http-missing-staged-secret"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.usage.network_egress_bytes, 0);
        }
        other => panic!("expected guest to complete after host HTTP denial, got {other:?}"),
    }
    assert!(
        network.requests().is_empty(),
        "missing staged secret must be denied before outbound transport"
    );
}

#[tokio::test]
async fn host_runtime_services_denies_wasm_http_when_shared_egress_has_no_policy_handoff() {
    let parsed_manifest = ExtensionManifest::parse(WASM_HTTP_SUCCESS_MANIFEST).unwrap();
    let component = tool_component(HTTP_TOOL_WAT);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(
            parsed_manifest.id.as_str(),
            "wasm/http-success.wasm",
            &component,
        )
        .await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let direct_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        filesystem,
        governor,
        Arc::new(AllowAllDispatchAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::clone(&egress))
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&direct_http)),
    )
    .unwrap();
    let capability_id = CapabilityId::new("wasm-http.success").unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request(
            capability_id,
            json!({"call": "http-without-policy"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.usage.network_egress_bytes, 0);
        }
        RuntimeCapabilityOutcome::Failed(_) => {}
        other => panic!("expected completed or failed outcome, got {other:?}"),
    }
    assert!(egress.requests().is_empty());
    assert!(
        direct_http.requests().unwrap().is_empty(),
        "HostRuntimeServices must not let a preconfigured WASM host bypass policy handoff when shared egress is active"
    );
}

#[test]
fn host_runtime_services_wasm_input_encode_releases_prepared_reservation() {
    let services = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services.rs"),
    )
    .unwrap();
    let reservation_index = services
        .find("let reservation = match request.resource_reservation")
        .expect("WASM execution must bind the dispatch reservation");
    let input_index = services
        .find("let input_json = match serde_json::to_string(&request.input)")
        .expect("WASM input encoding must use explicit cleanup branch");

    assert!(
        reservation_index < input_index,
        "WASM adapters must take ownership of a prepared reservation before input encoding so encode failures can release it"
    );
    assert!(
        services.contains(
            "Err(_) => {\n            release_wasm_reservation(request.governor, reservation.id);"
        ),
        "InputEncode failures must release the prepared WASM reservation"
    );
}

#[tokio::test]
async fn host_runtime_services_cancel_and_status_share_process_result_and_cancellation_graph() {
    let process_services = ProcessServices::in_memory();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime_for_local_testing();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id);
    let token = cancellation_registry.register(&scope, process_id);
    process_store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            scope.clone(),
            CorrelationId::new(),
        ))
        .await
        .unwrap();
    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Process(process_id)
    );

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert!(token.is_cancelled());
    let result = result_store.get(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn host_runtime_services_cancel_writes_killed_result_when_reservation_is_stale() {
    let process_services = ProcessServices::in_memory();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime_for_local_testing();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let stale_reservation_id = ResourceReservationId::new();
    let scope = sample_scope(invocation_id);
    let token = cancellation_registry.register(&scope, process_id);
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.resource_reservation_id = Some(stale_reservation_id);
    process_store.start(start).await.unwrap();

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert!(token.is_cancelled());
    let record = process_store
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Killed);
    let result = result_store.get(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn host_runtime_services_cancel_records_kill_side_effects_when_cleanup_fails() {
    let process_services = ProcessServices::new(
        Arc::new(InMemoryProcessStore::new()),
        Arc::new(InMemoryProcessResultStore::new()),
    );
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(LocalFilesystem::new()),
        Arc::new(FailingCleanupResourceGovernor),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime_for_local_testing();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id);
    let token = cancellation_registry.register(&scope, process_id);
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.resource_reservation_id = Some(ResourceReservationId::new());
    process_store.start(start).await.unwrap();

    let _error = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            scope.clone(),
            CorrelationId::new(),
            CancelReason::UserRequested,
        ))
        .await
        .expect_err("cleanup failure should remain visible to callers");

    assert!(
        token.is_cancelled(),
        "cleanup errors after terminalization must not skip cooperative cancellation"
    );
    let record = process_store
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Killed);
    let result = result_store
        .get(&scope, process_id)
        .await
        .unwrap()
        .expect("cleanup errors after terminalization must still write a killed result");
    assert_eq!(result.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn spawned_obligation_lifecycle_reconciles_resources_and_discards_handoffs_on_success() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let fixture = spawn_obligation_fixture(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::success(),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_status(
        fixture.process_store.as_ref(),
        &fixture.scope,
        process.process_id,
        ProcessStatus::Completed,
    )
    .await;

    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_releases_resources_and_discards_handoffs_on_runtime_failure()
{
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let fixture = spawn_obligation_fixture(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::failure("runtime_dispatch"),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_status(
        fixture.process_store.as_ref(),
        &fixture.scope,
        process.process_id,
        ProcessStatus::Failed,
    )
    .await;

    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_releases_resources_and_discards_handoffs_on_kill() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let fixture = spawn_obligation_fixture(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::delayed_success(Duration::from_millis(50)),
    )
    .await;

    let process = fixture.spawn().await;
    let host = ProcessHost::new(fixture.process_store.as_ref());
    host.kill(&fixture.scope, process.process_id).await.unwrap();

    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn process_obligation_lifecycle_cleans_record_started_before_wrapper_exists() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let inner_store = Arc::new(InMemoryProcessStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let estimate = ResourceEstimate {
        process_count: Some(1),
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    governor
        .reserve_with_id(scope.clone(), estimate.clone(), reservation_id)
        .unwrap();
    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "runtime-secret",
    )
    .await;
    let process_id = ProcessId::new();
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.estimated_resources = estimate;
    start.resource_reservation_id = Some(reservation_id);
    inner_store.start(start).await.unwrap();

    let lifecycle_store = obligation_services.process_obligation_lifecycle_store(inner_store);
    lifecycle_store.kill(&scope, process_id).await.unwrap();

    assert!(matches!(
        governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn process_obligation_lifecycle_cleans_legacy_handoffs_without_resource_reservation() {
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let inner_store = Arc::new(InMemoryProcessStore::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "runtime-secret",
    )
    .await;
    let process_id = ProcessId::new();
    inner_store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let lifecycle_store = obligation_services.process_obligation_lifecycle_store(inner_store);
    lifecycle_store.kill(&scope, process_id).await.unwrap();
}

#[tokio::test]
async fn process_obligation_lifecycle_rejects_second_active_handoff_for_same_scope_capability() {
    let inner_store = Arc::new(InMemoryProcessStore::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let first_process_id = ProcessId::new();
    let second_process_id = ProcessId::new();
    let lifecycle_store = obligation_services.process_obligation_lifecycle_store(inner_store);
    let secret_handle = SecretHandle::new("api_token").unwrap();

    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "runtime-secret",
    )
    .await;
    lifecycle_store
        .start(process_start(
            first_process_id,
            invocation_id,
            scope.clone(),
        ))
        .await
        .unwrap();

    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "runtime-secret",
    )
    .await;
    let error = lifecycle_store
        .start(process_start(
            second_process_id,
            invocation_id,
            scope.clone(),
        ))
        .await
        .expect_err("a scoped capability may only have one active process handoff");

    assert!(matches!(error, ProcessError::InvalidStoredRecord { .. }));
    assert!(
        lifecycle_store
            .get(&scope, second_process_id)
            .await
            .unwrap()
            .is_none(),
        "the rejected second process must not be persisted as running"
    );

    lifecycle_store
        .complete(&scope, first_process_id)
        .await
        .unwrap();
    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "runtime-secret",
    )
    .await;
    lifecycle_store
        .start(process_start(
            second_process_id,
            invocation_id,
            scope.clone(),
        ))
        .await
        .expect("a new handoff can start after the prior handoff reaches terminal cleanup");
}

#[tokio::test]
async fn process_obligation_lifecycle_does_not_clean_handoffs_twice_after_background_cleanup() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let inner_store = Arc::new(InMemoryProcessStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let process_id = ProcessId::new();
    let estimate = ResourceEstimate {
        process_count: Some(1),
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    governor
        .reserve_with_id(scope.clone(), estimate.clone(), reservation_id)
        .unwrap();
    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "first-runtime-secret",
    )
    .await;
    let lifecycle_store = obligation_services.process_obligation_lifecycle_store(inner_store);
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.estimated_resources = estimate;
    start.resource_reservation_id = Some(reservation_id);
    lifecycle_store.start(start).await.unwrap();

    lifecycle_store
        .cleanup_process_obligations(&scope, process_id, false)
        .await
        .unwrap();
    stage_process_handoffs(
        &obligation_services,
        &scope,
        &script_capability_id(),
        &secret_handle,
        wasm_http_policy(),
        "second-runtime-secret",
    )
    .await;

    lifecycle_store.kill(&scope, process_id).await.unwrap();
}

#[tokio::test]
async fn process_obligation_lifecycle_surfaces_resource_cleanup_errors_after_terminal_transition() {
    let reservation_id = ResourceReservationId::new();
    let inner_store = Arc::new(InMemoryProcessStore::new());
    let governor = Arc::new(FailingCleanupResourceGovernor);
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let process_id = ProcessId::new();
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.resource_reservation_id = Some(reservation_id);
    let lifecycle_store = obligation_services.process_obligation_lifecycle_store(inner_store);
    lifecycle_store.start(start).await.unwrap();

    let error = lifecycle_store
        .kill(&scope, process_id)
        .await
        .expect_err("terminal cleanup failures should be visible to callers");

    assert!(matches!(
        error,
        ProcessError::Resource(ResourceError::ReservationMismatch { id }) if id == reservation_id
    ));
    let record = lifecycle_store
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn spawned_obligation_lifecycle_cleans_handoffs_when_result_store_complete_fails() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let result_store = Arc::new(FailingProcessResultStore::default());
    let fixture = spawn_obligation_fixture_with_result_store(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::success(),
        Arc::clone(&result_store),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_result_store_attempt(&result_store, "complete").await;
    wait_for_no_reserved_processes(&fixture.governor).await;

    let record = fixture
        .process_store
        .get(&fixture.scope, process.process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_cleans_handoffs_when_result_store_fail_fails() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let result_store = Arc::new(FailingProcessResultStore::default());
    let fixture = spawn_obligation_fixture_with_result_store(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::failure("runtime_dispatch"),
        Arc::clone(&result_store),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_result_store_attempt(&result_store, "fail").await;
    wait_for_no_reserved_processes(&fixture.governor).await;

    let record = fixture
        .process_store
        .get(&fixture.scope, process.process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_reconciles_when_store_complete_fails_after_result_write() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let inner_process_store = Arc::new(FailingTerminalProcessStore::fail_complete());
    let fixture = spawn_obligation_fixture_with_process_store_and_result_store(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::success(),
        Arc::clone(&inner_process_store),
        Arc::new(InMemoryProcessResultStore::new()),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_process_store_attempt(&inner_process_store, "complete").await;
    wait_for_no_reserved_processes(&fixture.governor).await;

    let record = fixture
        .process_store
        .get(&fixture.scope, process.process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_releases_when_store_fail_fails_after_result_write() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let inner_process_store = Arc::new(FailingTerminalProcessStore::fail_fail());
    let fixture = spawn_obligation_fixture_with_process_store_and_result_store(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::failure("runtime_dispatch"),
        Arc::clone(&inner_process_store),
        Arc::new(InMemoryProcessResultStore::new()),
    )
    .await;

    let process = fixture.spawn().await;
    wait_for_process_store_attempt(&inner_process_store, "fail").await;
    wait_for_no_reserved_processes(&fixture.governor).await;

    let record = fixture
        .process_store
        .get(&fixture.scope, process.process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn spawned_obligation_lifecycle_abort_cleans_up_when_process_start_fails() {
    let reservation_id = ResourceReservationId::new();
    let secret_handle = SecretHandle::new("api_token").unwrap();
    let fixture = spawn_obligation_fixture(
        reservation_id,
        secret_handle.clone(),
        BackgroundExecutor::success(),
    )
    .await;
    let failing_manager = FailingSpawnManager;
    let host = CapabilityHost::new(
        fixture.registry.as_ref(),
        fixture.dispatcher.as_ref(),
        fixture.authorizer.as_ref(),
    )
    .with_obligation_handler(fixture.handler.as_ref())
    .with_process_manager(&failing_manager);

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context: fixture.context.clone(),
            capability_id: script_capability_id(),
            estimate: fixture.estimate.clone(),
            input: json!({"message": "spawn fails"}),
            trust_decision: trust_decision_with_dispatch_authority(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ironclaw_capabilities::CapabilityInvocationError::Process { .. }
    ));
    assert!(matches!(
        fixture.governor.release(reservation_id).unwrap_err(),
        ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        }
    ));
}

#[tokio::test]
async fn host_runtime_services_wasm_guest_error_reconciles_usage_after_host_effect() {
    let wat = http_then_guest_error_wat();
    let runtime = wasm_runtime_for_component(
        WASM_GUEST_ERROR_MANIFEST,
        "wasm-accounting.guest_error",
        "wasm/guest-error.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "guest-error"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the guest returns an error response"
    );
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_invalid_output_reconciles_usage_after_host_effect() {
    let wat = http_then_invalid_output_wat();
    let runtime = wasm_runtime_for_component(
        WASM_INVALID_OUTPUT_MANIFEST,
        "wasm-accounting.invalid_output",
        "wasm/invalid-output.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "invalid-output"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::InvalidInput);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the guest returns malformed output"
    );
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_guest_error_reconciles_wall_clock_after_host_effect() {
    let wat = http_without_body_then_guest_error_wat();
    let runtime = wasm_runtime_for_component_with_slow_zero_body_http(
        WASM_WALL_CLOCK_FAILURE_MANIFEST,
        "wasm-accounting.wall_clock_failure",
        "wasm/wall-clock-failure.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "wall-clock-failure"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert_eq!(runtime.http.requests().unwrap().len(), 1);
    let usage = runtime.governor.usage_for(&sample_account());
    assert!(
        usage.wall_clock_ms > 0,
        "wall-clock usage must be reconciled even when a failed guest has no byte/token/process usage"
    );
    assert_eq!(usage.network_egress_bytes, 0);
    assert_eq!(
        runtime
            .governor
            .reserved_for(&sample_account())
            .network_egress_bytes,
        0
    );
}

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected_kind: RuntimeFailureKind) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected_kind),
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

fn assert_completed_outcome(outcome: RuntimeCapabilityOutcome, expected_capability: &CapabilityId) {
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(&completed.capability_id, expected_capability);
            assert_eq!(completed.output, json!(1));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
}

type InMemoryHostRuntimeServices = HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    InMemoryProcessStore,
    InMemoryProcessResultStore,
>;

struct InMemoryRecordingCombinedRunStateApprovalStore {
    runs: InMemoryRunStateStore,
    approvals: InMemoryApprovalRequestStore,
    combined_calls: AtomicUsize,
    separate_save_calls: AtomicUsize,
}

impl InMemoryRecordingCombinedRunStateApprovalStore {
    fn new() -> Self {
        Self {
            runs: InMemoryRunStateStore::new(),
            approvals: InMemoryApprovalRequestStore::new(),
            combined_calls: AtomicUsize::new(0),
            separate_save_calls: AtomicUsize::new(0),
        }
    }

    fn combined_calls(&self) -> usize {
        self.combined_calls.load(Ordering::SeqCst)
    }

    fn separate_save_calls(&self) -> usize {
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

struct ApprovalResumeFixture {
    services: InMemoryHostRuntimeServices,
    run_state: Arc<InMemoryRunStateStore>,
    approval_requests: Arc<InMemoryApprovalRequestStore>,
    capability_leases: Arc<InMemoryCapabilityLeaseStore>,
    events: InMemoryEventSink,
}

fn approval_resume_fixture() -> ApprovalResumeFixture {
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let events = InMemoryEventSink::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
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

fn resume_runtime_with_empty_registry(fixture: &ApprovalResumeFixture) -> DefaultHostRuntime {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
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

async fn assert_blocked_approval_run(
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

async fn block_for_approval(
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
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap()
}

struct SentinelApprovalAuthorizer;

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

#[derive(Default)]
struct RecordingScriptExecutor {
    mounts: std::sync::Mutex<Vec<Option<MountView>>>,
}

impl RecordingScriptExecutor {
    fn recorded_mounts(&self) -> Vec<Option<MountView>> {
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
            result: ironclaw_scripts::ScriptCapabilityResult {
                output: request.invocation.input,
                reservation_id: reservation.id,
                usage,
                output_bytes: 0,
            },
            receipt,
        })
    }
}

struct ExitFailureScriptBackend;

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

struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

struct FailingDurableAuditLog;

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

struct AllowAllDispatchAuthorizer;

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[derive(Debug, Clone, Default)]
struct RecordingNetworkHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetworkHttpEgress {
    fn new() -> Self {
        Self::default()
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    fn execute(
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
struct SecretStoreLeaseCredentials {
    handle: SecretHandle,
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

#[derive(Debug, Clone, Default)]
struct RecordingRuntimeHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeHttpEgress {
    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}

async fn stage_process_handoffs(
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

struct SpawnObligationFixture {
    registry: Arc<ExtensionRegistry>,
    dispatcher: Arc<NoopDispatcher>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    handler: Arc<BuiltinObligationHandler>,
    process_manager: Arc<BackgroundProcessManager>,
    process_store: Arc<ProcessObligationLifecycleStore>,
    governor: Arc<InMemoryResourceGovernor>,
    context: ExecutionContext,
    scope: ResourceScope,
    estimate: ResourceEstimate,
}

impl SpawnObligationFixture {
    async fn spawn(&self) -> ironclaw_processes::ProcessRecord {
        let host = CapabilityHost::new(
            self.registry.as_ref(),
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
        )
        .with_obligation_handler(self.handler.as_ref())
        .with_process_manager(self.process_manager.as_ref());

        host.spawn_json(CapabilitySpawnRequest {
            context: self.context.clone(),
            capability_id: script_capability_id(),
            estimate: self.estimate.clone(),
            input: json!({"message": "background"}),
            trust_decision: trust_decision_with_dispatch_authority(),
        })
        .await
        .unwrap()
        .process
    }
}

async fn spawn_obligation_fixture(
    reservation_id: ResourceReservationId,
    secret_handle: SecretHandle,
    executor: BackgroundExecutor,
) -> SpawnObligationFixture {
    spawn_obligation_fixture_with_result_store(
        reservation_id,
        secret_handle,
        executor,
        Arc::new(InMemoryProcessResultStore::new()),
    )
    .await
}

async fn spawn_obligation_fixture_with_result_store<R>(
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
        Arc::new(InMemoryProcessStore::new()),
        result_store,
    )
    .await
}

async fn spawn_obligation_fixture_with_process_store_and_result_store<P, R>(
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
    let dispatcher = Arc::new(NoopDispatcher);
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        secret_store.clone(),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let context =
        execution_context_with_dispatch_grant_for_scope(script_capability_id(), scope.clone());
    let estimate = ResourceEstimate {
        process_count: Some(1),
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    secret_store
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from("runtime-secret"),
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
                    let _ = cleanup_process_store
                        .cleanup_process_obligations(&failure.scope, failure.process_id, reconcile)
                        .await;
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

#[derive(Default)]
struct FailingProcessResultStore {
    attempts: std::sync::Mutex<Vec<&'static str>>,
}

#[derive(Debug)]
struct FailingCleanupResourceGovernor;

impl ResourceGovernor for FailingCleanupResourceGovernor {
    fn set_limit(
        &self,
        _account: ResourceAccount,
        _limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        Ok(ResourceReservation {
            id: ResourceReservationId::new(),
            scope,
            estimate,
        })
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        Ok(ResourceReservation {
            id: reservation_id,
            scope,
            estimate,
        })
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        _actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::ReservationMismatch { id: reservation_id })
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        Err(ResourceError::ReservationMismatch { id: reservation_id })
    }
}

impl FailingProcessResultStore {
    fn attempts(&self) -> Vec<&'static str> {
        self.attempts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProcessResultStore for FailingProcessResultStore {
    async fn complete(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
        _output: serde_json::Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.attempts.lock().unwrap().push("complete");
        Err(ProcessError::InvalidStoredRecord {
            reason: "result complete failed".to_string(),
        })
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
        _error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.attempts.lock().unwrap().push("fail");
        Err(ProcessError::InvalidStoredRecord {
            reason: "result fail failed".to_string(),
        })
    }

    async fn kill(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.attempts.lock().unwrap().push("kill");
        Err(ProcessError::InvalidStoredRecord {
            reason: "result kill failed".to_string(),
        })
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        Ok(None)
    }
}

struct FailingTerminalProcessStore {
    inner: InMemoryProcessStore,
    fail_complete: bool,
    fail_fail: bool,
    attempts: std::sync::Mutex<Vec<&'static str>>,
}

impl FailingTerminalProcessStore {
    fn fail_complete() -> Self {
        Self {
            inner: InMemoryProcessStore::new(),
            fail_complete: true,
            fail_fail: false,
            attempts: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn fail_fail() -> Self {
        Self {
            inner: InMemoryProcessStore::new(),
            fail_complete: false,
            fail_fail: true,
            attempts: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn attempts(&self) -> Vec<&'static str> {
        self.attempts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProcessStore for FailingTerminalProcessStore {
    async fn start(
        &self,
        start: ProcessStart,
    ) -> Result<ironclaw_processes::ProcessRecord, ProcessError> {
        self.inner.start(start).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ironclaw_processes::ProcessRecord, ProcessError> {
        self.attempts.lock().unwrap().push("complete");
        if self.fail_complete {
            return Err(ProcessError::InvalidStoredRecord {
                reason: "status complete failed".to_string(),
            });
        }
        self.inner.complete(scope, process_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ironclaw_processes::ProcessRecord, ProcessError> {
        self.attempts.lock().unwrap().push("fail");
        if self.fail_fail {
            return Err(ProcessError::InvalidStoredRecord {
                reason: "status fail failed".to_string(),
            });
        }
        self.inner.fail(scope, process_id, error_kind).await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ironclaw_processes::ProcessRecord, ProcessError> {
        self.inner.kill(scope, process_id).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ironclaw_processes::ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_processes::ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

struct BackgroundExecutor {
    outcome: BackgroundExecutorOutcome,
}

impl BackgroundExecutor {
    fn success() -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Success(json!({"ok": true})),
        }
    }

    fn success_with_output(output: serde_json::Value) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Success(output),
        }
    }

    fn failure(kind: impl Into<String>) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::Failure(kind.into()),
        }
    }

    fn delayed_success(delay: Duration) -> Self {
        Self {
            outcome: BackgroundExecutorOutcome::DelayedSuccess(delay),
        }
    }
}

enum BackgroundExecutorOutcome {
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

struct FailingSpawnManager;

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

struct NoopDispatcher;

#[async_trait]
impl CapabilityDispatcher for NoopDispatcher {
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        panic!("spawn tests must not invoke the foreground dispatcher")
    }
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

async fn wait_for_result_store_attempt(store: &FailingProcessResultStore, attempt: &'static str) {
    for _ in 0..100 {
        if store.attempts().contains(&attempt) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("result store did not record {attempt} attempt");
}

async fn wait_for_process_store_attempt(
    store: &FailingTerminalProcessStore,
    attempt: &'static str,
) {
    for _ in 0..100 {
        if store.attempts().contains(&attempt) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process store did not record {attempt} attempt");
}

async fn wait_for_no_reserved_processes(governor: &InMemoryResourceGovernor) {
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

struct ClientErrorMcpExecutor;

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

struct PanicMcpExecutor;

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

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    registry_with_manifests(&[manifest])
}

fn registry_with_manifests(manifests: &[&str]) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    for manifest in manifests {
        let manifest = ExtensionManifest::parse(manifest).unwrap();
        let root =
            VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
        let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
        registry.insert(package).unwrap();
    }
    registry
}

fn execution_context_without_grants() -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
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

fn execution_context_with_dispatch_grant(capability: CapabilityId) -> ExecutionContext {
    let grants = capability_grants(capability);
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
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
        runtime: RuntimeKind::Wasm,
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
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
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

fn mount_view(alias: &str, target: &str, permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).unwrap(),
        VirtualPath::new(target).unwrap(),
        permissions,
    )])
    .unwrap()
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
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn read_directory_text(root: &std::path::Path) -> String {
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
        extension_id: script_extension_id(),
        capability_id: script_capability_id(),
        runtime: RuntimeKind::Script,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: json!({"message": "running"}),
    }
}

fn script_extension_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn mcp_capability_id() -> CapabilityId {
    CapabilityId::new("mcp.search").unwrap()
}

struct WasmRuntimeFixture {
    runtime: DefaultHostRuntime,
    governor: Arc<InMemoryResourceGovernor>,
    http: Arc<RecordingWasmHostHttp>,
    capability_id: CapabilityId,
}

struct WasmWallClockRuntimeFixture {
    runtime: DefaultHostRuntime,
    governor: Arc<InMemoryResourceGovernor>,
    http: Arc<SlowZeroBodyWasmHostHttp>,
    capability_id: CapabilityId,
}

#[derive(Debug)]
struct SlowZeroBodyWasmHostHttp {
    requests: std::sync::Mutex<Vec<WasmHttpRequest>>,
    delay: Duration,
}

impl SlowZeroBodyWasmHostHttp {
    fn new(delay: Duration) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            delay,
        }
    }

    fn requests(&self) -> Result<Vec<WasmHttpRequest>, WasmHostError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| WasmHostError::Failed("slow HTTP request log is poisoned".into()))
    }
}

impl WasmHostHttp for SlowZeroBodyWasmHostHttp {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        self.requests
            .lock()
            .map_err(|_| WasmHostError::Failed("slow HTTP request log is poisoned".into()))?
            .push(request);
        thread::sleep(self.delay);
        Ok(WasmHttpResponse {
            status: 204,
            headers_json: "{}".to_string(),
            body: Vec::new(),
        })
    }
}

async fn wasm_runtime_for_component(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmRuntimeFixture {
    let parsed_manifest = ExtensionManifest::parse(manifest).unwrap();
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(AllowAllDispatchAuthorizer);
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&http)),
    )
    .unwrap();

    WasmRuntimeFixture {
        runtime: services.host_runtime_for_local_testing(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

async fn wasm_runtime_for_component_with_slow_zero_body_http(
    manifest: &str,
    capability: &str,
    module_path: &str,
    wat: &str,
) -> WasmWallClockRuntimeFixture {
    let parsed_manifest = ExtensionManifest::parse(manifest).unwrap();
    let component = tool_component(wat);
    let filesystem = Arc::new(
        filesystem_with_wasm_component(parsed_manifest.id.as_str(), module_path, &component).await,
    );
    let governor = Arc::new(governor_with_default_limit(sample_account()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(AllowAllDispatchAuthorizer);
    let http = Arc::new(SlowZeroBodyWasmHostHttp::new(Duration::from_millis(25)));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(manifest)),
        filesystem,
        Arc::clone(&governor),
        authorizer,
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .try_with_wasm_runtime(
        WitToolRuntimeConfig::for_testing(),
        WitToolHost::deny_all().with_http(Arc::clone(&http)),
    )
    .unwrap();

    WasmWallClockRuntimeFixture {
        runtime: services.host_runtime_for_local_testing(),
        governor,
        http,
        capability_id: CapabilityId::new(capability).unwrap(),
    }
}

async fn filesystem_with_wasm_component(
    extension_id: &str,
    module_path: &str,
    wasm_bytes: &[u8],
) -> LocalFilesystem {
    let fs = mounted_empty_extension_root();
    let path =
        VirtualPath::new(format!("/system/extensions/{extension_id}/{module_path}")).unwrap();
    fs.write_file(&path, wasm_bytes).await.unwrap();
    fs
}

fn mounted_empty_extension_root() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(10),
                max_network_egress_bytes: Some(10_000),
                max_output_bytes: Some(100_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
}

fn wasm_runtime_request(
    capability_id: CapabilityId,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let scope = sample_scope(InvocationId::new());
    wasm_runtime_request_for_scope(capability_id, scope, input)
}

fn wasm_runtime_request_for_scope(
    capability_id: CapabilityId,
    scope: ResourceScope,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let context = execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope);
    RuntimeCapabilityRequest::new(
        context,
        capability_id,
        wasm_http_estimate(),
        input,
        trust_decision_with_dispatch_authority(),
    )
}

fn wasm_http_estimate() -> ResourceEstimate {
    ResourceEstimate {
        concurrency_slots: Some(1),
        network_egress_bytes: Some(10),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
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
        max_egress_bytes: Some(10_000),
    }
}

fn tool_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).unwrap();
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("tool.wit", include_str!("../../../wit/tool.wit"))
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

fn http_then_guest_error_wat() -> String {
    HTTP_TOOL_WAT.replace(
        "i32.const 48\n    i32.const 1\n    i32.store\n    i32.const 52\n    i32.const 3072\n    i32.store\n    i32.const 56\n    i32.const 1\n    i32.store\n    i32.const 60\n    i32.const 0\n    i32.store\n    i32.const 48",
        "i32.const 48\n    i32.const 0\n    i32.store\n    i32.const 52\n    i32.const 0\n    i32.store\n    i32.const 56\n    i32.const 0\n    i32.store\n    i32.const 60\n    i32.const 1\n    i32.store\n    i32.const 64\n    i32.const 3072\n    i32.store\n    i32.const 68\n    i32.const 11\n    i32.store\n    i32.const 48",
    )
}

fn http_then_invalid_output_wat() -> String {
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

fn http_without_body_then_guest_error_wat() -> String {
    http_then_guest_error_wat().replace(
        "i32.const 1\n    i32.const 256\n    i32.const 5",
        "i32.const 0\n    i32.const 0\n    i32.const 0",
    )
}

#[cfg(feature = "libsql")]
fn submit_turn_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
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
    }
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

const MCP_MANIFEST: &str = r#"
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

const WASM_MANIFEST: &str = r#"
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

const WASM_HTTP_SUCCESS_MANIFEST: &str = r#"
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

const WASM_GUEST_ERROR_MANIFEST: &str = r#"
id = "wasm-accounting"
name = "WASM Accounting Guest Error"
version = "0.1.0"
description = "WASM accounting extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/guest-error.wasm"

[[capabilities]]
id = "wasm-accounting.guest_error"
description = "Call host HTTP then return guest error"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const WASM_INVALID_OUTPUT_MANIFEST: &str = r#"
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

const WASM_WALL_CLOCK_FAILURE_MANIFEST: &str = r#"
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
description = "Spend wall-clock time through host HTTP then return a guest error"
effects = ["dispatch_capability", "network"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const HTTP_TOOL_WAT: &str = r#"
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
