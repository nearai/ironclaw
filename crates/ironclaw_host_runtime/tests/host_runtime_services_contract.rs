// arch-exempt: large_file, mechanical lease-store test repoint to FilesystemCapabilityLeaseStore<InMemoryBackend> helper (arch-simplification §4.3), no new test logic, plan #6168
mod support;

use support::host_runtime_harness::*;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer,
    TrustAwareCapabilityDispatchAuthorizer, in_memory_backed_capability_lease_store,
};
use ironclaw_capabilities::{CapabilityHost, CapabilitySpawnRequest};
use ironclaw_event_projections::{
    AuditProjectionError, AuditProjectionRequest, AuditProjectionService, EventProjectionService,
    ProjectionCursor, ProjectionError, ProjectionRequest, ProjectionScope,
    ReplayAuditProjectionService, ReplayEventProjectionService, RunProjectionStatus,
    TimelineEntryKind,
};
use ironclaw_events::{
    DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventCursor, EventError,
    EventStreamKey, InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog,
    InMemoryEventSink, ReadScope, RuntimeEventKind,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::DiskFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationServices, CancelReason, CancelRuntimeWorkRequest, CapabilitySurfaceVersion,
    HostRuntime, HostRuntimeServices, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssueKind, RuntimeCapabilityAuthResumeRequest, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeFailureKind,
    RuntimeStatusRequest, RuntimeWorkId, TenantSandboxProcessPort, builtin_first_party_handlers,
};
use ironclaw_processes::{
    BackgroundProcessManager, FilesystemProcessResultStore, FilesystemProcessStore, ProcessError,
    ProcessHost, ProcessManager, ProcessResultStore, ProcessStatus, ProcessStore,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStoreError, RebornProfile, build_reborn_event_stores,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, JsonFileResourceGovernorStore, PersistentResourceGovernor,
    ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits, ResourceTally,
};
use ironclaw_run_state::{ApprovalRequestStore, RunStart, RunStateStore, RunStatus};
use ironclaw_scripts::{ScriptRuntime, ScriptRuntimeConfig};
use ironclaw_secrets::{
    InMemoryCredentialBroker, InMemorySecretStore, SecretMaterial, SecretStore,
};
use ironclaw_triggers::InMemoryTriggerRepository;
#[cfg(feature = "libsql")]
use ironclaw_turns::FilesystemTurnStateStore;
use ironclaw_turns::NoopTurnRunWakeNotifier;
#[cfg(feature = "libsql")]
use ironclaw_turns::{
    InMemoryRunProfileResolver, SubmitTurnResponse, TurnCoordinator, TurnStateStore,
};
use ironclaw_wasm::{
    RecordingWasmHostHttp, WasmHttpResponse, WasmStagedRuntimeCredential,
    WasmStagedRuntimeCredentials, WitToolHost, WitToolRuntimeConfig,
};
use serde_json::json;

fn with_authenticated_actor(
    mut context: ExecutionContext,
    actor_user_id: Option<&str>,
) -> ExecutionContext {
    context.authenticated_actor_user_id =
        actor_user_id.map(|value| UserId::new(value).expect("valid authenticated actor user id"));
    context
}

fn assert_actor_policy_denied(outcome: RuntimeCapabilityOutcome) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
            assert!(
                failure
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("PolicyDenied")),
                "actor mismatch must surface the policy-denied authorization reason: {failure:?}"
            );
        }
        other => panic!("expected actor policy denial, got {other:?}"),
    }
}

async fn assert_alice_run_status(
    run_state: &ironclaw_run_state::FilesystemRunStateStore<ironclaw_filesystem::InMemoryBackend>,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    expected_status: RunStatus,
) {
    let record = run_state
        .get(scope, invocation_id)
        .await
        .unwrap()
        .expect("Alice-owned run must remain present");
    assert_eq!(record.status, expected_status);
    assert_eq!(
        record
            .authenticated_actor_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("slack-alice")
    );
}

#[tokio::test]
async fn production_wiring_validation_rejects_missing_components_and_local_only_defaults() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
            ProductionWiringComponent::RuntimePolicy,
            ProductionWiringIssueKind::Missing
        ),
        "missing resolved runtime policy should be reported: {report:?}"
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

#[tokio::test]
async fn production_wiring_validation_rejects_local_only_runtime_policy() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_policy(local_dev_runtime_policy());

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("local-dev runtime policy must not pass production validation");

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimePolicy,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "local runtime policy should be reported as local-only: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_rejects_each_local_only_runtime_policy_field() {
    let mut host_workspace = hosted_dev_runtime_policy();
    host_workspace.filesystem_backend = FilesystemBackendKind::HostWorkspace;
    assert_local_only_runtime_policy_rejected(host_workspace, "host_workspace_filesystem");

    let mut host_workspace_and_home = hosted_dev_runtime_policy();
    host_workspace_and_home.filesystem_backend = FilesystemBackendKind::HostWorkspaceAndHome;
    assert_local_only_runtime_policy_rejected(host_workspace_and_home, "host_workspace_filesystem");

    let mut local_process = hosted_dev_runtime_policy();
    local_process.process_backend = ProcessBackendKind::LocalHost;
    assert_local_only_runtime_policy_rejected(local_process, "local_host_process");

    let mut direct_network = hosted_dev_runtime_policy();
    direct_network.network_mode = NetworkMode::Direct;
    assert_local_only_runtime_policy_rejected(direct_network, "direct_network");

    let mut scrubbed_secrets = hosted_dev_runtime_policy();
    scrubbed_secrets.secret_mode = SecretMode::ScrubbedEnv;
    assert_local_only_runtime_policy_rejected(scrubbed_secrets, "local_secret_environment");

    let mut inherited_secrets = hosted_dev_runtime_policy();
    inherited_secrets.secret_mode = SecretMode::InheritedEnv;
    assert_local_only_runtime_policy_rejected(inherited_secrets, "local_secret_environment");
}

#[tokio::test]
async fn production_wiring_validation_accepts_production_safe_runtime_policy_shape() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_policy(hosted_dev_runtime_policy());

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("other local/test defaults still prevent production validation");

    assert!(
        !report.contains(
            ProductionWiringComponent::RuntimePolicy,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "hosted runtime policy should satisfy runtime-policy guardrail: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_accepts_persistent_resource_governor_component() {
    let dir = tempfile::tempdir().unwrap();
    let governor = Arc::new(PersistentResourceGovernor::new(
        JsonFileResourceGovernorStore::new(dir.path().join("resource-governor.json")),
    ));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        governor,
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_filesystem_resource_governor(Arc::clone(&scoped));

    let governor = services.resource_governor();
    let scope = sample_scope(InvocationId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();
    let reservation = governor
        .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
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
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
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
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_concurrency_slots(1),
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

#[tokio::test]
async fn production_wiring_validation_classifies_combined_store_as_run_state_and_approvals() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_classifies_in_memory_backed_lease_store_as_local_only() {
    // Regression guard for arch-simplification §4.3 (deleting
    // `InMemoryCapabilityLeaseStore`): the production-wiring classifier keyed the
    // now-deleted store as an explicit `LocalOnly` type, and unknown component
    // types default to `ProductionCandidate`. Its replacement — the production
    // `FilesystemCapabilityLeaseStore<InMemoryBackend>` the no-durable build and
    // every test seam wire — must classify the same way, or a volatile in-memory
    // lease store could silently satisfy production readiness. Drive the real
    // `HostRuntimeServices` caller so the classification is exercised through the
    // monomorphized `with_capability_leases::<T>` type capture, not just the
    // classifier helper.
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_capability_leases(Arc::new(in_memory_backed_capability_lease_store()));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("in-memory-backed lease store must not pass production validation");

    assert!(
        report.contains(
            ProductionWiringComponent::CapabilityLeases,
            ProductionWiringIssueKind::LocalOnlyImplementation,
        ),
        "FilesystemCapabilityLeaseStore<InMemoryBackend> must classify local-only: {report:?}"
    );
    assert!(
        !report.contains(
            ProductionWiringComponent::CapabilityLeases,
            ProductionWiringIssueKind::Missing,
        ),
        "a wired lease store must satisfy capability-lease presence: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_classifies_in_memory_backed_run_state_and_approval_stores_as_local_only()
 {
    // Regression guard for arch-simplification §4.3 (deleting
    // `InMemoryRunStateStore` / `InMemoryApprovalRequestStore`): the classifier
    // keyed the now-deleted stores as explicit `LocalOnly` types, and unknown
    // component types default to `ProductionCandidate`. Their replacements — the
    // production `Filesystem*Store<InMemoryBackend>` pair the no-durable build
    // and every test seam wire — must classify the same way, or volatile
    // in-memory run-state/approval stores could silently satisfy production
    // readiness. Drive the real `HostRuntimeServices` caller so classification
    // is exercised through the monomorphized `with_run_state::<T>` /
    // `with_approval_requests::<T>` type capture, not just the classifier
    // helper (the combined-store path hard-codes `LocalOnly` and bypasses it).
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_run_state(Arc::new(
        ironclaw_run_state::in_memory_backed_run_state_store(),
    ))
    .with_approval_requests(Arc::new(
        ironclaw_run_state::in_memory_backed_approval_request_store(),
    ));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err(
            "in-memory-backed run-state/approval stores must not pass production validation",
        );

    for component in [
        ProductionWiringComponent::RunState,
        ProductionWiringComponent::ApprovalRequests,
    ] {
        assert!(
            report.contains(
                component,
                ProductionWiringIssueKind::LocalOnlyImplementation
            ),
            "Filesystem store<InMemoryBackend> for {component:?} must classify local-only: {report:?}"
        );
        assert!(
            !report.contains(component, ProductionWiringIssueKind::Missing),
            "a wired store must satisfy {component:?} presence: {report:?}"
        );
    }
}

#[tokio::test]
async fn production_wiring_validation_rejects_unsupported_runtime_requirements() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
// graph restart over `DiskFilesystem`) and the `ironclaw_run_state`
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_turn_state_selection_accepts_filesystem_turn_state_store() {
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("turn-state.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let scoped = libsql_scoped_turns_fs(Arc::clone(&db)).await;

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_rejects_noop_turn_wake_notifier() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_accepts_configured_turn_wake_notifier() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_uses_configured_runtime_requirements() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_sees_underlying_in_memory_durable_logs() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_rejects_direct_durable_sink_wrappers_as_unverified() {
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let audit_log: Arc<dyn DurableAuditLog> = Arc::new(InMemoryDurableAuditLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_accepts_verified_host_http_egress_shape() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn host_http_egress_helper_requires_graph_secret_store() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_requires_credential_broker_when_configured() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]).require_credential_broker())
        .expect_err("production credential broker requirement must fail closed when missing");

    assert!(
        report.contains(
            ProductionWiringComponent::CredentialAccountStore,
            ProductionWiringIssueKind::Missing
        ),
        "missing credential account store should be reported: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::CredentialSessionStore,
            ProductionWiringIssueKind::Missing
        ),
        "missing credential session store should be reported: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_rejects_local_only_credential_broker() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_credential_broker(broker);

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]).require_credential_broker())
        .expect_err("in-memory credential broker must not satisfy production guardrail");

    assert!(
        report.contains(
            ProductionWiringComponent::CredentialAccountStore,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory credential account store should be reported as local-only: {report:?}"
    );
    assert!(
        report.contains(
            ProductionWiringComponent::CredentialSessionStore,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "in-memory credential session store should be reported as local-only: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_rejects_unverified_runtime_http_egress() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::new()));

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

#[tokio::test]
async fn production_wiring_validation_tracks_process_port_for_builtin_shell() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_builtin_first_party_package()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err("default local process port must not satisfy production shell wiring");

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "builtin shell should make the local process port visible to production guardrails: {report:?}"
    );

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_builtin_first_party_package()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(Arc::new(ProductionCandidateProcessPort));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err("other local defaults should still keep this graph non-production");

    assert!(
        !report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::LocalOnlyImplementation
        ),
        "custom process port should clear the process-port local-only issue: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_tracks_tenant_sandbox_process_port_for_builtin_shell() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_builtin_first_party_package()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_policy(hosted_dev_runtime_policy());

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err("tenant sandbox process policy must require a sandbox process port");

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::Missing
        ),
        "tenant sandbox process backend should require the tenant sandbox process port: {report:?}"
    );

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_builtin_first_party_package()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_policy(hosted_dev_runtime_policy())
    .with_tenant_sandbox_process_port(Arc::new(TenantSandboxProcessPort::new(Arc::new(
        ProductionCandidateSandboxTransport,
    ))));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err(
            "sandbox port readiness must remain explicit until a production transport is wired",
        );

    assert!(
        !report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::Missing
        ) && report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "configured tenant sandbox process port should clear missing but remain unverified: {report:?}"
    );

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_builtin_first_party_package()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_policy(hosted_dev_runtime_policy())
    .with_production_tenant_sandbox_process_port(Arc::new(TenantSandboxProcessPort::new(
        Arc::new(ProductionCandidateSandboxTransport),
    )));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err("test service graph still uses local-only backing stores");

    assert!(
        !report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::Missing
        ) && !report.contains(
            ProductionWiringComponent::RuntimeProcessPort,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "verified tenant sandbox process port should satisfy the process-port gate: {report:?}"
    );
}

#[tokio::test]
async fn production_wiring_validation_rejects_empty_verified_wasm_credentials() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_rejects_wasm_credentials_added_after_adapter() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn production_wiring_validation_rejects_wasm_credentials_replaced_after_adapter() {
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(WASM_HTTP_SUCCESS_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
    let filesystem = Arc::new(DiskFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(GrantAuthorizer::new());
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_libsql_root_filesystem(filesystem);

    assert_services_use_combined_store_for_atomic_approval_block(
        services,
        "approval after root filesystem selection",
    )
    .await;
}

#[tokio::test]
async fn host_runtime_services_writes_runtime_events_to_durable_event_log_metadata_only() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(DiskFilesystem::new());
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
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(SentinelApprovalAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
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
    assert_eq!(entry.stage, AuditStage::ApprovalResolved);
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
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(SentinelApprovalAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
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
    let processes_filesystem = ironclaw_processes::in_memory_backed_processes_filesystem();
    let inner_process_store = Arc::new(FilesystemProcessStore::new(Arc::clone(
        &processes_filesystem,
    )));
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let process_store =
        Arc::new(obligation_services.process_obligation_lifecycle_store(inner_process_store));
    let durable_event_log: Arc<dyn DurableEventLog> = event_log.clone();
    process_store.set_event_sink(Arc::new(DurableEventSink::new(durable_event_log)));
    let result_store = Arc::new(FilesystemProcessResultStore::new(processes_filesystem));
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
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
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
async fn host_runtime_services_resume_missing_runtime_secret_returns_auth_gate() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("approval_resume_token").unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle,
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approval then auth"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&services, &scope, gate.approval_request_id, None).await;

    let resumed = runtime
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

    match resumed {
        RuntimeCapabilityOutcome::AuthRequired(auth_gate) => {
            assert_eq!(auth_gate.capability_id, script_capability_id());
            assert!(
                auth_gate.required_secrets.is_empty(),
                "secret handles are not product-visible until auth recovery projections carry them"
            );
        }
        other => panic!("expected auth-required resume outcome, got {other:?}"),
    }
    let run = run_state
        .get(&scope, scope.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::BlockedAuth);
    assert_eq!(run.error_kind.as_deref(), Some("AuthRequired"));
    // A missing-credential bounce parks the run at BlockedAuth (non-terminal):
    // the claimed approval lease is intentionally preserved, not revoked, so the
    // same invocation can reuse it on auth-resume without a second human approval.
    assert_eq!(
        capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Claimed
    );
    assert!(
        script_runtime.recorded_mounts().is_empty(),
        "missing credential must block before dispatch"
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
    let invalid_context_error = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            invalid_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        invalid_context_error,
        ironclaw_host_runtime::HostRuntimeError::InvalidRequest { .. }
    ));
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
async fn host_runtime_services_resume_runtime_policy_denial_fails_matching_blocked_run() {
    let fixture = approval_resume_fixture_with_manifest(
        SCRIPT_NETWORK_MANIFEST,
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    );
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "policy reduced before resume"});

    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let denied_runtime = resume_runtime_with_policy(&fixture, network_denied_runtime_policy());

    let outcome = denied_runtime
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

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    let failed_run = fixture
        .run_state
        .get(&scope, context.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed_run.status, RunStatus::Failed);
    assert_eq!(failed_run.approval_request_id, None);
    assert_eq!(
        failed_run.error_kind.as_deref(),
        Some("process_backend_none")
    );
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "runtime-policy preflight failure must not claim or consume the approval lease"
    );
    assert!(fixture.events.events().is_empty());
}

#[tokio::test]
async fn host_runtime_services_resume_rejects_changed_actor_before_preflight_mutates_run() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let alice_context =
        with_authenticated_actor(execution_context_without_grants(), Some("slack-alice"));
    let scope = alice_context.resource_scope.clone();
    let invocation_id = alice_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "actor-sealed approval resume"});
    let gate = block_for_approval(
        &runtime,
        alice_context.clone(),
        estimate.clone(),
        input.clone(),
    )
    .await;
    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id, None)
            .await;
    let event_count_before_resume = fixture.events.events().len();
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    for attempted_actor in [Some("slack-bob"), None] {
        let attempted_context = with_authenticated_actor(alice_context.clone(), attempted_actor);
        let outcome = broken_runtime
            .resume_capability(RuntimeCapabilityResumeRequest::new(
                attempted_context,
                gate.approval_request_id,
                script_capability_id(),
                estimate.clone(),
                input.clone(),
                trust_decision_with_dispatch_authority(),
            ))
            .await
            .unwrap();

        assert_actor_policy_denied(outcome);
        assert_alice_run_status(
            fixture.run_state.as_ref(),
            &scope,
            invocation_id,
            RunStatus::BlockedApproval,
        )
        .await;
        assert_eq!(
            fixture
                .capability_leases
                .get(&scope, lease.grant.id)
                .await
                .unwrap()
                .status,
            CapabilityLeaseStatus::Active,
            "actor rejection must happen before the approval lease is claimed"
        );
        assert_eq!(
            fixture.events.events().len(),
            event_count_before_resume,
            "actor rejection must happen before runtime dispatch events"
        );
    }

    let valid_alice_outcome = broken_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            alice_context,
            gate.approval_request_id,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(valid_alice_outcome, RuntimeFailureKind::MissingRuntime);
    assert_alice_run_status(
        fixture.run_state.as_ref(),
        &scope,
        invocation_id,
        RunStatus::Failed,
    )
    .await;
}

#[tokio::test]
async fn host_runtime_services_auth_resume_rejects_changed_actor_before_preflight_mutates_run() {
    let fixture = approval_resume_fixture();
    let alice_context =
        with_authenticated_actor(execution_context_without_grants(), Some("slack-alice"));
    let scope = alice_context.resource_scope.clone();
    let invocation_id = alice_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "actor-sealed auth resume"});
    fixture
        .run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: script_capability_id(),
            authenticated_actor_user_id: alice_context.authenticated_actor_user_id.clone(),
        })
        .await
        .unwrap();
    fixture
        .run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    for attempted_actor in [Some("slack-bob"), None] {
        let attempted_context = with_authenticated_actor(alice_context.clone(), attempted_actor);
        let outcome = broken_runtime
            .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
                attempted_context,
                script_capability_id(),
                estimate.clone(),
                input.clone(),
                trust_decision_with_dispatch_authority(),
                None,
            ))
            .await
            .unwrap();

        assert_actor_policy_denied(outcome);
        assert_alice_run_status(
            fixture.run_state.as_ref(),
            &scope,
            invocation_id,
            RunStatus::BlockedAuth,
        )
        .await;
        assert!(
            fixture.events.events().is_empty(),
            "actor rejection must happen before runtime dispatch events"
        );
    }

    let valid_alice_outcome = broken_runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            alice_context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
            None,
        ))
        .await
        .unwrap();

    assert_failed_outcome(valid_alice_outcome, RuntimeFailureKind::MissingRuntime);
    assert_alice_run_status(
        fixture.run_state.as_ref(),
        &scope,
        invocation_id,
        RunStatus::Failed,
    )
    .await;
}

#[tokio::test]
async fn host_runtime_services_resume_spawn_rejects_changed_actor_before_input_and_preflight() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        process_services.clone(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor));
    let runtime = services.host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let alice_context = with_authenticated_actor(
        execution_context_without_grants_for_scope(scope.clone()),
        Some("slack-alice"),
    );
    let input = process_sandbox_input();
    let estimate = process_sandbox_estimate();
    let blocked = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            alice_context.clone(),
            process_sandbox_capability_id(),
            estimate.clone(),
            input.clone(),
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();
    let approval_request_id = match blocked {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate.approval_request_id,
        other => panic!("expected approval gate, got {other:?}"),
    };
    let lease = approve_spawn_for_services(&services, &scope, approval_request_id, None).await;
    let broken_runtime = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .host_runtime_for_local_testing();

    for attempted_actor in [Some("slack-bob"), None] {
        let attempted_context = with_authenticated_actor(alice_context.clone(), attempted_actor);
        let outcome = broken_runtime
            .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
                attempted_context,
                approval_request_id,
                process_sandbox_capability_id(),
                estimate.clone(),
                input.clone(),
                process_sandbox_trust_decision(),
            ))
            .await
            .unwrap();

        assert_actor_policy_denied(outcome);
        assert_alice_run_status(
            run_state.as_ref(),
            &scope,
            scope.invocation_id,
            RunStatus::BlockedApproval,
        )
        .await;
        assert_eq!(
            capability_leases
                .get(&scope, lease.grant.id)
                .await
                .unwrap()
                .status,
            CapabilityLeaseStatus::Active,
            "actor rejection must happen before the spawn approval lease is claimed"
        );
        assert!(sandbox_executor.requests().is_empty());
        assert!(
            process_store
                .records_for_scope(&scope)
                .await
                .unwrap()
                .is_empty(),
            "actor rejection must happen before process creation"
        );
    }

    let invalid_input_outcome = broken_runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            with_authenticated_actor(alice_context.clone(), Some("slack-bob")),
            approval_request_id,
            process_sandbox_capability_id(),
            estimate.clone(),
            invalid_process_sandbox_input(),
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();
    assert_actor_policy_denied(invalid_input_outcome);
    assert_alice_run_status(
        run_state.as_ref(),
        &scope,
        scope.invocation_id,
        RunStatus::BlockedApproval,
    )
    .await;

    let valid_alice_outcome = broken_runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            alice_context,
            approval_request_id,
            process_sandbox_capability_id(),
            estimate,
            input,
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(valid_alice_outcome, RuntimeFailureKind::MissingRuntime);
    assert_alice_run_status(
        run_state.as_ref(),
        &scope,
        scope.invocation_id,
        RunStatus::Failed,
    )
    .await;
    assert!(sandbox_executor.requests().is_empty());
    assert!(
        process_store
            .records_for_scope(&scope)
            .await
            .unwrap()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Happy-path auth-resume: BlockedAuth run with credential present → dispatch+complete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn host_runtime_services_auth_resume_dispatches_blocked_auth_run() {
    // Setup: uses ApprovalThenSecretObligationAuthorizer so the first invoke
    // fires an approval gate, and the first resume (missing credential) bounces
    // to BlockedAuth.  After adding the credential we verify that
    // auth_resume_capability dispatches and completes the run.
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("auth_resume_token").unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle.clone(),
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "auth-resume dispatch"});

    // Phase 1: invoke → approval gate.
    let gate = block_for_approval(&runtime, context.clone(), estimate.clone(), input.clone()).await;
    approve_dispatch_for_services(&services, &scope, gate.approval_request_id, None).await;

    // Phase 2: resume with credential absent → AuthRequired / BlockedAuth.
    let auth_gate = runtime
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
    assert!(
        matches!(auth_gate, RuntimeCapabilityOutcome::AuthRequired(_)),
        "expected AuthRequired after credential-missing resume, got {auth_gate:?}"
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "pre-condition: run must be BlockedAuth"
    );

    // Phase 3: add credential, then auth_resume → dispatch + complete.
    secret_store
        .put(
            scope.clone(),
            secret_handle,
            SecretMaterial::from("test-secret-value"),
            None,
        )
        .await
        .unwrap();

    let auth_resumed = runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            context.clone(),
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
            Some(gate.approval_request_id),
        ))
        .await
        .unwrap();

    match auth_resumed {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, input);
        }
        other => panic!("expected completed auth-resume outcome, got {other:?}"),
    }
    let completed_run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        completed_run.status,
        RunStatus::Completed,
        "auth_resume must complete the BlockedAuth run"
    );
    assert_eq!(
        script_runtime.recorded_mounts().len(),
        1,
        "dispatch must have been called exactly once"
    );
}

// ---------------------------------------------------------------------------
// auth-resume preflight rejection must fail the BlockedAuth run record
//
// Before the fix, `auth_resume_capability` returned a terminal failure outcome
// on preflight errors (policy/trust) WITHOUT transitioning the BlockedAuth run
// to Failed — leaving a stale resumable gate after the caller saw a terminal
// failure.  The approval-resume path (`resume_capability`) already called
// `fail_matching_blocked_resume_on_preflight_error`.  This test verifies the
// equivalent now exists for auth-resume.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn host_runtime_services_auth_resume_trust_preflight_failure_fails_blocked_auth_run() {
    // Setup: use the standard fixture so we get a real run_state/approval_requests.
    let fixture = approval_resume_fixture();
    let _runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "auth-resume preflight fix"});

    // Put the run in BlockedAuth directly (mirrors what happens after approval →
    // resume_json auth bounce: run is BlockedAuth).
    fixture
        .run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: script_capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    fixture
        .run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let run = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "pre-condition: run must be BlockedAuth"
    );

    // Build a broken runtime (empty extension registry → trust preflight fails).
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    // Wrong-scope call: preflight fails with wrong scope, must NOT fail the
    // matching BlockedAuth run (different scope = different invocation).
    let wrong_scope = ResourceScope {
        user_id: UserId::new("other-user").unwrap(),
        ..scope.clone()
    };
    let wrong_context = execution_context_without_grants_for_scope(wrong_scope);
    let wrong_outcome = broken_runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            wrong_context,
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
            None,
        ))
        .await
        .unwrap();
    assert_failed_outcome(wrong_outcome, RuntimeFailureKind::MissingRuntime);

    // Matching run must still be BlockedAuth (wrong scope → guard skips it).
    let run_after_wrong = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_after_wrong.status,
        RunStatus::BlockedAuth,
        "wrong-scope preflight failure must not affect the matching BlockedAuth run"
    );

    // Matching-scope call: preflight fails → must transition the BlockedAuth run to Failed.
    // Pre-fix: the run was left as stale BlockedAuth because
    // fail_matching_blocked_auth_resume_on_preflight_error was not called.
    let matching_outcome = broken_runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            context.clone(),
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
            None,
        ))
        .await
        .unwrap();
    assert_failed_outcome(matching_outcome, RuntimeFailureKind::MissingRuntime);

    let failed_run = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed_run.status,
        RunStatus::Failed,
        "matching-scope auth-resume preflight failure must transition BlockedAuth run to Failed \
         (pre-fix: run was left as stale BlockedAuth)"
    );
}

// ---------------------------------------------------------------------------
// approval-then-auth path: auth-resume with approval_request_id = Some(id)
// must still fail a BlockedAuth run whose record has approval_request_id = None
//
// When a run goes through approval → resume → BlockedAuth, the BlockedAuth
// transition explicitly clears the persisted approval_request_id to None.
// The subsequent auth-resume request still carries the original
// approval_request_id so it can claim the approval lease.  Before the fix,
// the guard in fail_matching_blocked_auth_resume_on_preflight_error compared
// record.approval_request_id (None) against the request's Some(id) and
// returned early without failing the run, leaving it stuck as BlockedAuth.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn host_runtime_services_auth_resume_with_approval_id_fails_blocked_auth_run_on_preflight_error()
 {
    let fixture = approval_resume_fixture();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approval-then-auth preflight fix"});

    // Directly put the run into BlockedAuth with approval_request_id = None
    // (this mirrors what block_auth does: it always clears approval_request_id).
    fixture
        .run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: script_capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    fixture
        .run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let run = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "pre-condition: run must be BlockedAuth"
    );
    assert_eq!(
        run.approval_request_id, None,
        "pre-condition: BlockedAuth record must have approval_request_id = None"
    );

    // Build a broken runtime (empty extension registry → trust preflight fails).
    let broken_runtime = resume_runtime_with_empty_registry(&fixture);

    // Auth-resume carries a non-None approval_request_id (the original gate id
    // from the approval phase).  Before the fix, the guard compared
    // record.approval_request_id (None) != Some(id) and returned early, leaving
    // the run stuck as BlockedAuth.
    let orphan_approval_id = ApprovalRequestId::new();
    let outcome = broken_runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            context.clone(),
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
            Some(orphan_approval_id),
        ))
        .await
        .unwrap();
    assert_failed_outcome(outcome, RuntimeFailureKind::MissingRuntime);

    // The BlockedAuth run must now be Failed, not stuck as BlockedAuth.
    let after = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status,
        RunStatus::Failed,
        "approval-then-auth preflight failure must transition BlockedAuth run to Failed \
         even when the request carries approval_request_id = Some(id) \
         (pre-fix: run was left stuck as BlockedAuth)"
    );
}

#[tokio::test]
async fn host_runtime_services_resume_without_backing_stores_fails_closed() {
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .host_runtime_for_local_testing();

    let health = runtime.health().await.unwrap();

    assert!(!health.ready);
    assert_eq!(health.missing_runtime_backends, vec![RuntimeKind::Script]);
}

#[tokio::test]
async fn host_runtime_routes_system_process_sandbox_to_configured_executor() {
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let result_store = process_services.result_store();
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let process_id = ProcessId::new();

    let handle = runtime
        .spawn_process(process_sandbox_start(process_id, scope.clone()))
        .await
        .unwrap();

    assert_eq!(handle.process_id, process_id);
    assert_eq!(handle.capability_id, process_sandbox_capability_id());
    wait_for_sandbox_process_result(&sandbox_executor, &scope, process_id, result_store.as_ref())
        .await;
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_routes_approved_request_to_configured_executor() {
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let result_store = process_services.result_store();
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());

    let outcome = runtime
        .spawn_capability(process_sandbox_runtime_request_for_scope(scope.clone()))
        .await
        .unwrap();

    let process_id = match outcome {
        RuntimeCapabilityOutcome::SpawnedProcess(handle) => {
            assert_eq!(handle.capability_id, process_sandbox_capability_id());
            handle.process_id
        }
        other => panic!("expected spawned process, got {other:?}"),
    };
    wait_for_sandbox_process_result(&sandbox_executor, &scope, process_id, result_store.as_ref())
        .await;
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_rejects_invalid_plan_before_executor() {
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let mut request = process_sandbox_runtime_request_for_scope(scope);
    request.input = invalid_process_sandbox_input();

    // A malformed/invalid plan is model-fixable: it must surface as a
    // recoverable, model-visible tool error (InvalidInput) so the run
    // continues and the model can correct its arguments — never a terminal
    // HostRuntimeError that kills the whole run.
    let outcome = runtime
        .spawn_capability(request)
        .await
        .expect("invalid sandbox plans must not be a terminal host runtime error");

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
            assert_eq!(
                failure.disposition(),
                ironclaw_host_runtime::CapabilityFailureDisposition::ModelVisibleToolError,
            );
            assert!(
                failure
                    .message
                    .as_deref()
                    .unwrap_or_default()
                    .contains("SandboxProcessPlan")
            );
        }
        other => panic!("expected recoverable InvalidInput failure, got {other:?}"),
    }
    assert!(
        sandbox_executor.requests().is_empty(),
        "invalid sandbox plan must not reach process spawn"
    );
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_runtime_policy_denial_fails_before_executor() {
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .with_runtime_policy(network_denied_runtime_policy())
    .host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());

    let outcome = runtime
        .spawn_capability(process_sandbox_runtime_request_for_scope(scope))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(
        sandbox_executor.requests().is_empty(),
        "runtime policy denial must fail before process spawn"
    );
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_host_failure_fails_after_preflight() {
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let runtime = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor))
    .host_runtime_for_local_testing()
    .with_process_manager(Arc::new(FailingSpawnManager));
    let scope = sample_scope(InvocationId::new());

    let outcome = runtime
        .spawn_capability(process_sandbox_runtime_request_for_scope(scope))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert!(
        sandbox_executor.requests().is_empty(),
        "host spawn failure must not reach the process sandbox executor"
    );
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_blocks_for_approval_before_executor() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let result_store = process_services.result_store();
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        process_services,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor));
    let runtime = services.host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let context = execution_context_without_grants_for_scope(scope.clone());
    let input = process_sandbox_input();
    let estimate = process_sandbox_estimate();

    let blocked = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            process_sandbox_capability_id(),
            estimate.clone(),
            input.clone(),
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    let approval_request_id = match blocked {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            assert_eq!(gate.capability_id, process_sandbox_capability_id());
            gate.approval_request_id
        }
        other => panic!("expected approval gate, got {other:?}"),
    };
    assert!(
        sandbox_executor.requests().is_empty(),
        "process sandbox executor must not run before approval"
    );

    approve_spawn_for_services(&services, &scope, approval_request_id, None).await;
    let resumed = runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            context,
            approval_request_id,
            process_sandbox_capability_id(),
            estimate,
            input,
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    let process_id = match resumed {
        RuntimeCapabilityOutcome::SpawnedProcess(handle) => handle.process_id,
        other => panic!("expected spawned process after approval, got {other:?}"),
    };
    wait_for_sandbox_process_result(&sandbox_executor, &scope, process_id, result_store.as_ref())
        .await;
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_resume_changed_input_fails_before_executor() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor));
    let runtime = services.host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let context = execution_context_without_grants_for_scope(scope.clone());
    let input = process_sandbox_input();
    let estimate = process_sandbox_estimate();

    let blocked = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            process_sandbox_capability_id(),
            estimate.clone(),
            input,
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    let approval_request_id = match blocked {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate.approval_request_id,
        other => panic!("expected approval gate, got {other:?}"),
    };
    let lease = approve_spawn_for_services(&services, &scope, approval_request_id, None).await;

    let outcome = runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            context,
            approval_request_id,
            process_sandbox_capability_id(),
            estimate,
            json!({"run": {"command": "echo", "args": ["changed"]}}),
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Authorization);
    assert!(
        sandbox_executor.requests().is_empty(),
        "changed resume input must fail before process spawn"
    );
    assert_eq!(
        capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "fingerprint mismatch must fail before lease claim/consume"
    );
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_resume_invalid_plan_fails_before_executor() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor));
    let runtime = services.host_runtime_for_local_testing();
    let scope = sample_scope(InvocationId::new());
    let context = execution_context_without_grants_for_scope(scope.clone());
    let input = process_sandbox_input();
    let estimate = process_sandbox_estimate();

    let blocked = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            process_sandbox_capability_id(),
            estimate.clone(),
            input,
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    let approval_request_id = match blocked {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate.approval_request_id,
        other => panic!("expected approval gate, got {other:?}"),
    };
    let lease = approve_spawn_for_services(&services, &scope, approval_request_id, None).await;

    // Same recoverable contract on the resume path: a malformed/invalid plan
    // is model-fixable, so it must surface as a recoverable InvalidInput tool
    // error rather than a terminal host runtime error.
    let outcome = runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            context,
            approval_request_id,
            process_sandbox_capability_id(),
            estimate,
            invalid_process_sandbox_input(),
            process_sandbox_trust_decision(),
        ))
        .await
        .expect("invalid sandbox resume input must not be a terminal host runtime error");

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
            assert_eq!(
                failure.disposition(),
                ironclaw_host_runtime::CapabilityFailureDisposition::ModelVisibleToolError,
            );
            assert!(
                failure
                    .message
                    .as_deref()
                    .unwrap_or_default()
                    .contains("SandboxProcessPlan")
            );
        }
        other => panic!("expected recoverable InvalidInput failure, got {other:?}"),
    }
    assert!(
        sandbox_executor.requests().is_empty(),
        "invalid resume plan must not reach process spawn"
    );
    assert_eq!(
        capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active,
        "invalid resume input must fail before lease claim/consume"
    );
}

#[tokio::test]
async fn host_runtime_spawn_process_sandbox_resume_host_failure_fails_after_approval() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let sandbox_executor = Arc::new(RecordingSandboxProcessExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_host_bundled_manifest(
            PROCESS_SANDBOX_MANIFEST,
        )),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "system.process_sandbox",
        process_sandbox_authority_effects(),
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_process_sandbox_executor(Arc::clone(&sandbox_executor));
    let runtime = services
        .host_runtime_for_local_testing()
        .with_process_manager(Arc::new(FailingSpawnManager));
    let scope = sample_scope(InvocationId::new());
    let context = execution_context_without_grants_for_scope(scope.clone());
    let input = process_sandbox_input();
    let estimate = process_sandbox_estimate();

    let blocked = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            process_sandbox_capability_id(),
            estimate.clone(),
            input.clone(),
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    let approval_request_id = match blocked {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate.approval_request_id,
        other => panic!("expected approval gate, got {other:?}"),
    };
    approve_spawn_for_services(&services, &scope, approval_request_id, None).await;

    let outcome = runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            context,
            approval_request_id,
            process_sandbox_capability_id(),
            estimate,
            input,
            process_sandbox_trust_decision(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert!(
        sandbox_executor.requests().is_empty(),
        "host resume-spawn failure must not reach the process sandbox executor"
    );
}

#[tokio::test]
async fn host_runtime_services_installs_builtin_obligation_handler_with_audit_sink() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(DiskFilesystem::new());
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
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(Vec::new())),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(Vec::new())),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_runtime_http_egress(Arc::new(RecordingRuntimeHttpEgress::new()))
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::UseScopedMounts {
                mounts: scoped_mounts.clone(),
            },
        ])),
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
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
        ironclaw_processes::in_memory_backed_process_services(),
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
    let filesystem = Arc::new(DiskFilesystem::new());
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
        ironclaw_processes::in_memory_backed_process_services(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::clone(&governor),
        authorizer,
        ironclaw_processes::in_memory_backed_process_services(),
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
            None,
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
            ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_network_egress_bytes(10)
                .set_output_bytes(100),
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
    assert_eq!(snapshot.entries[0].stage, AuditStage::Before);
    assert_eq!(snapshot.entries[1].stage, AuditStage::After);
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
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
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
        Arc::new(DiskFilesystem::new()),
        Arc::clone(&governor),
        authorizer,
        ironclaw_processes::in_memory_backed_process_services(),
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
            ResourceEstimate::default()
                .set_concurrency_slots(1)
                .set_output_bytes(1024),
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
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let reservation_id = ResourceReservationId::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ReserveResources { reservation_id },
        ])),
        ironclaw_processes::in_memory_backed_process_services(),
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
            ResourceEstimate::default().set_concurrency_slots(1),
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
    let filesystem = Arc::new(DiskFilesystem::new());
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
        ironclaw_processes::in_memory_backed_process_services(),
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
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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
            None,
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
async fn host_runtime_services_wasm_http_rejects_secret_store_lease_before_transport() {
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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
            None,
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
    assert_eq!(
        network.requests(),
        Vec::new(),
        "direct secret-store lease credentials must be rejected before network transport"
    );
}

#[tokio::test]
async fn host_runtime_services_wasm_http_missing_staged_secret_stays_before_transport() {
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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
    let parsed_manifest = parse_manifest(WASM_HTTP_SUCCESS_MANIFEST);
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
        ironclaw_processes::in_memory_backed_process_services(),
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

#[tokio::test]
async fn host_runtime_services_wasm_input_encode_releases_prepared_reservation() {
    // Regression guard: the WASM dispatch path must take its resource reservation
    // and wrap it in a `ReservationGuard` (RAII) *before* the fallible input-encode
    // step, so that an encode failure — or any other early return before the guard
    // is settled — releases the reservation via `Drop` instead of leaking it.
    //
    // This is a structural check rather than a behavioural one because
    // `serde_json::to_string(&Value)` is effectively infallible, so the
    // input-encode error branch is not triggerable at runtime. The guard's actual
    // Drop/release behaviour (including the cancellation path) is covered by the
    // unit tests in `services::wasm_execution::tests`. We assert ordering here:
    // reservation bound -> guard constructed -> input encoded.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/wasm_execution.rs"),
    )
    .unwrap();
    let reservation_index = source
        .find("let reservation = match request.resource_reservation")
        .expect("WASM execution must bind the dispatch reservation");
    let guard_index = source
        .find("ReservationGuard::new(request.governor, reservation.id)")
        .expect("WASM dispatch must wrap the prepared reservation in a ReservationGuard");
    let input_index = source
        .find("let input_json = match serde_json::to_string(&request.input)")
        .expect("WASM dispatch must encode the tool input after taking the reservation");

    assert!(
        reservation_index < guard_index && guard_index < input_index,
        "WASM adapters must wrap the prepared reservation in a ReservationGuard before \
         input encoding so encode (and any other) early-return failures release it via Drop"
    );
}

#[tokio::test]
async fn host_runtime_services_cancel_and_status_share_process_result_and_cancellation_graph() {
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(DiskFilesystem::new()),
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
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(DiskFilesystem::new()),
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
    let process_services = ironclaw_processes::in_memory_backed_process_services();
    let process_store = process_services.process_store();
    let result_store = process_services.result_store();
    let cancellation_registry = process_services.cancellation_registry();
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let runtime = HostRuntimeServices::new(
        registry,
        Arc::new(DiskFilesystem::new()),
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
    let inner_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let estimate = ResourceEstimate::default()
        .set_process_count(1)
        .set_concurrency_slots(1);
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
    let inner_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
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
    let inner_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
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
    let inner_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        governor.clone(),
    );
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id);
    let process_id = ProcessId::new();
    let estimate = ResourceEstimate::default()
        .set_process_count(1)
        .set_concurrency_slots(1);
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
    let inner_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
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
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
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
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
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
async fn host_runtime_services_wasm_operation_failed_reconciles_usage_after_host_effect() {
    let wat = http_then_operation_failed_wat();
    let runtime = wasm_runtime_for_component(
        WASM_OPERATION_FAILED_MANIFEST,
        "wasm-accounting.operation_failed",
        "wasm/operation-failed.wasm",
        &wat,
    )
    .await;

    let outcome = runtime
        .runtime
        .invoke_capability(wasm_runtime_request(
            runtime.capability_id,
            json!({"call": "operation-failed"}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::OperationFailed);
    assert_eq!(runtime.http.requests().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the capability returns an operation failure"
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

    assert_failed_outcome(outcome, RuntimeFailureKind::InvalidOutput);
    assert_eq!(runtime.http.requests().len(), 1);
    assert_eq!(
        runtime
            .governor
            .usage_for(&sample_account())
            .network_egress_bytes,
        5,
        "host-mediated HTTP request bytes must be reconciled even when the capability returns malformed output"
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
async fn host_runtime_services_wasm_operation_failed_reconciles_wall_clock_after_host_effect() {
    let wat = http_without_body_then_operation_failed_wat();
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

    assert_failed_outcome(outcome, RuntimeFailureKind::OperationFailed);
    assert_eq!(runtime.http.requests().len(), 1);
    let usage = runtime.governor.usage_for(&sample_account());
    assert!(
        usage.wall_clock_ms > 0,
        "wall-clock usage must be reconciled even when an operation failure has no byte/token/process usage"
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

/// `invoke_capability` on a capability that requires a credential + requires
/// approval must return `AuthRequired` without persisting an approval request
/// when the credential is absent.
///
/// Old ordering (bug): approval gate fires, human approves, then dispatch fails
/// with AuthRequired — burning the approval.
/// New ordering (fix): pre-flight sees missing credential, returns AuthRequired
/// immediately, approval gate never fires.
#[tokio::test]
async fn invoke_capability_missing_credential_returns_auth_before_approval() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    // Note: the secret "script_api_token" is deliberately NOT inserted.
    let secret_handle = SecretHandle::new("script_api_token").unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_CREDENTIAL_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle,
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs credential"});

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
        RuntimeCapabilityOutcome::AuthRequired(auth_gate) => {
            assert_eq!(
                auth_gate.capability_id,
                script_capability_id(),
                "auth gate must reference the invoked capability"
            );
        }
        other => panic!("expected AuthRequired before approval gate, got {other:?}"),
    }

    // No approval request should have been persisted — the approval gate must
    // not have fired at all.
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        pending.is_empty(),
        "approval must not be persisted when credential is absent; got {pending:?}"
    );

    // Dispatch must not have been called.
    assert!(
        script_runtime.recorded_mounts().is_empty(),
        "script executor must not be reached when credential pre-flight fails"
    );
}

/// `invoke_capability` with a credential present must proceed to the approval
/// gate as it did before Fix B — the pre-flight must not block happy-path flows.
#[tokio::test]
async fn invoke_capability_present_credential_proceeds_to_approval() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("script_api_token").unwrap();
    // Build the request context FIRST so we can seed the secret under the same
    // resource_scope that the invocation will use. Using a separate
    // execution_context_without_grants() for seeding would produce a different
    // InvocationId (and thus a different ResourceScope), causing the pre-flight
    // to find the secret absent even though it was inserted.
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    // Seed the required credential so pre-flight passes.
    secret_store
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from("token-value"),
            None,
        )
        .await
        .unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_CREDENTIAL_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle,
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "has credential"});

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

    // Credential is present — must reach the approval gate.
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(_) => {}
        other => panic!("expected ApprovalRequired when credential is present, got {other:?}"),
    }

    // An approval request must have been persisted.
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        !pending.is_empty(),
        "approval must be persisted when credential is present"
    );
}

/// `spawn_capability` with a credential present must proceed to the approval
/// gate — mirrors `invoke_capability_present_credential_proceeds_to_approval`
/// through the spawn dispatch lane, guarding against a spawn-only regression
/// that over-eagerly returns AuthRequired when the credential is present.
///
/// A present `SecretHandle` credential is seeded on the request's own
/// `ResourceScope`. The pre-flight must NOT block, and the outcome must be
/// `ApprovalRequired` (not a false `AuthRequired`).
#[tokio::test]
async fn spawn_capability_present_credential_proceeds_to_approval() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("script_api_token").unwrap();
    // Build the request context FIRST so we can seed the secret under the same
    // resource_scope that the invocation will use. Using a separate
    // execution_context_without_grants() for seeding would produce a different
    // InvocationId (and thus a different ResourceScope), causing the pre-flight
    // to find the secret absent even though it was inserted.
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    // Seed the required credential so pre-flight passes.
    secret_store
        .put(
            scope.clone(),
            secret_handle.clone(),
            SecretMaterial::from("token-value"),
            None,
        )
        .await
        .unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_CREDENTIAL_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        // ApprovalThenGrantAuthorizer implements authorize_spawn_with_trust correctly:
        // RequireApproval when grants are empty, delegates to GrantAuthorizer when grants
        // are present. ApprovalThenSecretObligationAuthorizer only implements the dispatch
        // variant and would fall back to the default deny for spawn calls.
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "spawn has credential"});

    let outcome = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    // Credential is present — spawn must reach the approval gate, NOT return a
    // false AuthRequired.
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(_) => {}
        other => panic!(
            "expected ApprovalRequired when credential is present on spawn path, got {other:?}"
        ),
    }

    // An approval request must have been persisted (approval gate fired).
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        !pending.is_empty(),
        "approval must be persisted when credential is present on spawn path"
    );
}

/// `invoke_capability` on a capability with NO credential requirement must be
/// unaffected by the pre-flight change — the pre-flight is a no-op when the
/// descriptor declares no `runtime_credentials`.
#[tokio::test]
async fn invoke_capability_no_credential_requirement_proceeds_normally() {
    // Use the plain SCRIPT_MANIFEST which has no runtime_credentials.
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "no credential needed"});

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

    // The `ApprovalThenGrantAuthorizer` (used by `approval_resume_fixture`)
    // requires approval on the first call (no grants). Confirm we reach the
    // approval gate, not a spurious AuthRequired.
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(_) => {}
        other => panic!("expected ApprovalRequired for no-credential capability, got {other:?}"),
    }
}

/// `spawn_capability` on a capability that requires a credential + requires
/// approval must return `AuthRequired` without persisting an approval request
/// when the credential is absent — mirrors the `invoke_capability` pre-flight
/// path through the spawn dispatch lane.
#[tokio::test]
async fn spawn_capability_missing_credential_returns_auth_before_approval() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    // Note: the secret "script_api_token" is deliberately NOT inserted.
    let secret_handle = SecretHandle::new("script_api_token").unwrap();
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_CREDENTIAL_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle,
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs credential via spawn"});

    let outcome = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(auth_gate) => {
            assert_eq!(
                auth_gate.capability_id,
                script_capability_id(),
                "auth gate must reference the spawned capability"
            );
        }
        other => panic!("expected AuthRequired before approval gate on spawn path, got {other:?}"),
    }

    // No approval request should have been persisted.
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        pending.is_empty(),
        "approval must not be persisted when credential is absent on spawn path; got {pending:?}"
    );

    // Script executor must not have been reached.
    assert!(
        script_runtime.recorded_mounts().is_empty(),
        "script executor must not be reached when credential pre-flight fails on spawn path"
    );
}

/// `invoke_capability` with the secret store wired but a capability that
/// declares zero required credentials must proceed past the pre-flight
/// (which short-circuits at `required_secrets.is_empty()`) and reach the
/// approval gate normally.
#[tokio::test]
async fn invoke_capability_no_credential_requirement_with_wired_store_proceeds_normally() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    // SCRIPT_MANIFEST has no runtime_credentials; wire a secret store anyway to
    // confirm the is_empty() early-exit branch is taken, not the no-store branch.
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("any_token").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenSecretObligationAuthorizer {
            handle: secret_handle,
        }),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store))
    .with_script_runtime(Arc::new(RecordingScriptExecutor::default()));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "no credential needed"});

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

    // With no required credentials the pre-flight exits at is_empty() and the
    // flow reaches the approval gate (ApprovalThenSecretObligationAuthorizer
    // requires approval when grants are empty).
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(_) => {}
        other => panic!(
            "expected ApprovalRequired when no credential is declared (wired store), got {other:?}"
        ),
    }

    // The approval request must have been persisted.
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        !pending.is_empty(),
        "approval must be persisted when credential pre-flight is a no-op (no required credentials)"
    );
}

/// A transient secret-store `metadata()` error must NOT let an uncredentialed call
/// through. Two layers are proven:
///
/// 1. **Pre-flight (ordering) fails open.** On the first `invoke_capability`, the
///    pre-flight probes the (erroring) store and must NOT short-circuit with
///    `AuthRequired` — a store error is not a missing credential. The flow proceeds
///    to the approval gate (`ApprovalRequired`); dispatch is not reached.
/// 2. **Dispatch-time obligation backstop fails closed.** After approval, the run is
///    resumed with a grant that DOES include the required `script_api_token` handle
///    plus the `UseSecret` effect, so dispatch authorization PASSES and control
///    reaches `BuiltinObligationHandler::preflight_secret_injection`. That backstop
///    re-probes the store via `metadata()` — which errors — and fails closed
///    (`secret_obligation_failed`), so the resumed call is `Failed`.
///
/// To prove the resume failure comes from the obligation backstop and not from a
/// premature authorization denial (both surface as `RuntimeFailureKind::Authorization`),
/// the store counts `metadata()` calls. The counter is reset after step 1, so a
/// non-zero count after resume can only come from the obligation handler probing the
/// store — `resume_capability` does not itself run the pre-flight. `ApprovalThenGrantAuthorizer`
/// injects no secret obligation of its own; enforcement is the manifest
/// `runtime_credentials` backstop.
#[tokio::test]
async fn invoke_capability_secret_store_error_skips_preflight() {
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let script_runtime = Arc::new(RecordingScriptExecutor::default());
    // Counts metadata() probes so we can prove the obligation backstop ran on resume.
    let metadata_calls = Arc::new(AtomicUsize::new(0));
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_CREDENTIAL_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        // ApprovalThenGrantAuthorizer: requires approval first, grants on resume, and
        // injects NO secret obligation of its own. Credential enforcement on the resume
        // path comes solely from the manifest runtime_credentials backstop.
        Arc::new(ApprovalThenGrantAuthorizer),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    // Wire the erroring, call-counting store — the pre-flight must skip on Err (not
    // return AuthRequired), and the dispatch-time obligation backstop must fail closed
    // when it re-probes the same erroring store on resume.
    .with_secret_store(Arc::new(CountingErrorSecretStore {
        metadata_calls: Arc::clone(&metadata_calls),
    }))
    .with_script_runtime(Arc::clone(&script_runtime));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "store errors"});

    // Step 1: store error → pre-flight skips → approval gate fires.
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            script_capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    // The pre-flight must NOT have short-circuited with AuthRequired — a store
    // error is not a missing credential.
    assert!(
        !matches!(outcome, RuntimeCapabilityOutcome::AuthRequired(_)),
        "pre-flight store error must not produce AuthRequired; got {outcome:?}"
    );

    // The flow must have reached the approval gate (pre-flight skipped).
    let gate = match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => {
            panic!("expected ApprovalRequired after pre-flight skip (store error); got {other:?}")
        }
    };

    // The approval request must have been persisted.
    let pending = approval_requests.records_for_scope(&scope).await.unwrap();
    assert!(
        !pending.is_empty(),
        "approval gate must have fired after pre-flight was skipped on store error; got {pending:?}"
    );

    // Dispatch must not have been called (blocked at approval gate).
    assert!(
        script_runtime.recorded_mounts().is_empty(),
        "script executor must not be reached when blocked at approval gate"
    );

    // Reset the metadata probe counter: any probe observed from here on can only
    // come from the resume path's obligation handler (resume does not run pre-flight).
    metadata_calls.store(0, Ordering::SeqCst);

    // Step 2: approve WITH the required secret handle granted, so dispatch
    // authorization PASSES and the resumed call reaches the dispatch-time credential
    // backstop inside the obligation handler (not the earlier grant-matching gate).
    // The grant lists `script_api_token` (the manifest's required runtime_credential)
    // plus the UseSecret effect, so grant evaluation against the manifest emits the
    // secret-injection obligation (ApprovalThenGrantAuthorizer adds no obligation of its
    // own). On resume, BuiltinObligationHandler::preflight_secret_injection probes the
    // erroring store via `metadata()`, which errors — and the backstop FAILS CLOSED
    // (`secret_obligation_failed`) instead of injecting. This is the exact PR contract:
    // a transient store error during the pre-flight skip cannot let an uncredentialed
    // call execute, because the dispatch-time obligation backstop re-checks presence and
    // fails closed on the same store error.
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_dispatch(
            &scope,
            gate.approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: vec![SecretHandle::new("script_api_token").unwrap()],
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .unwrap();

    let resumed = runtime
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

    // Prove the resume actually reached the obligation backstop: it must have probed
    // the store via `metadata()` at least once on the resume path. A premature
    // authorization denial (the wrong reason) would block BEFORE the obligation handler
    // and never probe the store — so this distinguishes the two even though both map to
    // `RuntimeFailureKind::Authorization`.
    assert!(
        metadata_calls.load(Ordering::SeqCst) >= 1,
        "resume must reach the dispatch-time obligation backstop and re-probe the store; \
         a zero metadata count means authorization was denied before the backstop ran"
    );

    // The backstop re-probes the required secret via `metadata()`. Against the erroring
    // store that probe fails, and the handler fails closed (`secret_obligation_failed`),
    // so the resumed dispatch is blocked — proving a transient store error in the
    // pre-flight skip path does not allow an uncredentialed call to execute.
    match &resumed {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(
                failure.capability_id,
                script_capability_id(),
                "dispatch-time credential backstop must reference the resumed capability"
            );
        }
        other => {
            panic!(
                "expected Failed from the dispatch-time obligation backstop on resume path; \
                 got {other:?}. The obligation handler must fail closed when metadata() errors \
                 for a required runtime_credentials handle."
            );
        }
    }
}
