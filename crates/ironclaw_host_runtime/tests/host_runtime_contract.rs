// arch-exempt: large_file, mechanical lease-store test repoint to FilesystemCapabilityLeaseStore<InMemoryBackend> helper (arch-simplification §4.3), no new test logic, plan #6168
mod support;

use support::legacy_capability_fixture_to_v2;

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::{
    GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer,
    in_memory_backed_capability_lease_store,
};
use ironclaw_extensions::{
    ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::{FilesystemError, FilesystemOperation, InMemoryBackend};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CancelReason, CancelRuntimeWorkRequest, CapabilitySurfacePolicy, CapabilitySurfaceVersion,
    DefaultHostRuntime, HostRuntime, HostRuntimeError, IdempotencyKey, RuntimeBackendHealth,
    RuntimeCapabilityRequest, RuntimeStatusRequest, RuntimeWorkId, SurfaceKind,
    VisibleCapabilityRequest,
};
use ironclaw_processes::{
    FilesystemProcessResultStore, FilesystemProcessStore, ProcessCancellationRegistry,
    ProcessError, ProcessRecord, ProcessResultStore, ProcessStart, ProcessStatus, ProcessStore,
};
use ironclaw_run_state::{
    ApprovalRecord, ApprovalRequestStore, InMemoryApprovalRequestStore, InMemoryRunStateStore,
    RunRecord, RunStart, RunStateApprovalStore, RunStateError, RunStateStore,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::json;

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalDev,
    ))
    .unwrap()
}

#[test]
fn bounded_contract_strings_share_validation_semantics() {
    assert!(IdempotencyKey::new("").is_err());
    assert!(IdempotencyKey::new("turn\n1").is_err());
    assert!(IdempotencyKey::new("x".repeat(257)).is_err());
    assert!(CapabilitySurfaceVersion::new("surface\t1").is_err());
    assert!(CapabilitySurfaceVersion::new("x".repeat(129)).is_err());
    assert!(SurfaceKind::new("").is_err());
    assert!(SurfaceKind::new("agent\n0").is_err());
    assert!(SurfaceKind::new("x".repeat(65)).is_err());

    let idempotency = IdempotencyKey::new("turn-1/tool-1").unwrap();
    let surface = CapabilitySurfaceVersion::new("surface-v1").unwrap();
    let surface_kind = SurfaceKind::new("agent_loop").unwrap();
    assert_eq!(idempotency.as_str(), "turn-1/tool-1");
    assert_eq!(surface.as_str(), "surface-v1");
    assert_eq!(surface_kind.as_str(), "agent_loop");
}

#[tokio::test]
async fn default_runtime_returns_completed_outcome_for_authorized_dispatch() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());

    let runtime = DefaultHostRuntime::new(
        registry.clone(),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state.clone())
    .with_approval_requests(approval_requests.clone());

    let context = execution_context_with_dispatch_grant();
    let request = RuntimeCapabilityRequest::new(
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
        trust_decision_with_dispatch_authority(),
    )
    .with_idempotency_key(IdempotencyKey::new("turn-1/tool-1").unwrap());

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.has_request());
}

#[tokio::test]
async fn default_runtime_surfaces_approval_required_with_persisted_request_id() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone())
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = RuntimeCapabilityRequest::new(
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id());
            let record = run_state
                .get(&context.resource_scope, context.invocation_id)
                .await
                .unwrap()
                .expect("run record persisted");
            assert_eq!(record.approval_request_id, Some(gate.approval_request_id));
        }
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_uses_combined_store_for_atomic_approval_block() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let combined_store = Arc::new(RecordingCombinedRunStateApprovalStore::new());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state_approval_store(combined_store.clone())
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = RuntimeCapabilityRequest::new(
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id());
            assert_eq!(combined_store.combined_calls(), 1);
            assert_eq!(combined_store.separate_save_calls(), 0);
            let record = RunStateStore::get(
                combined_store.as_ref(),
                &context.resource_scope,
                context.invocation_id,
            )
            .await
            .unwrap()
            .expect("run record persisted");
            assert_eq!(record.approval_request_id, Some(gate.approval_request_id));
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
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_propagates_unavailable_when_run_state_lookup_fails_during_approval() {
    // Regression: an earlier implementation swallowed `RunStateError` from
    // the approval-request lookup via `.ok().flatten()`, which masked storage
    // outages as a misleading "approval not persisted" Failed outcome. The
    // host runtime must instead surface persistence outages as
    // `HostRuntimeError::Unavailable` so callers can distinguish between a
    // missing record and a broken backend.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let inner_run_state = Arc::new(InMemoryRunStateStore::new());
    let run_state: Arc<dyn RunStateStore> = Arc::new(FailingGetRunStateStore {
        inner: inner_run_state.clone(),
    });
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await;

    let error = outcome.expect_err("run-state lookup outage must surface as host runtime error");
    match error {
        ironclaw_host_runtime::HostRuntimeError::Unavailable { reason } => {
            assert!(
                !reason.contains("/"),
                "unavailable reason must be infrastructure-opaque, got {reason:?}"
            );
        }
        other => panic!("expected HostRuntimeError::Unavailable, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_returns_failed_for_unknown_capability() {
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let request = RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(
                failure.kind,
                ironclaw_host_runtime::RuntimeFailureKind::MissingRuntime
            );
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert!(
        !dispatcher.has_request(),
        "unknown capabilities must fail during trust evaluation before dispatch"
    );
    assert!(
        run_state
            .records_for_scope(&scope)
            .await
            .unwrap()
            .is_empty(),
        "unknown capabilities must fail before starting a capability-host run record"
    );
}

#[tokio::test]
async fn default_runtime_surfaces_authorization_failure_when_authorizer_denies() {
    // Pins the deny path: a Decision::Deny from the authorizer must surface
    // as Failed with kind=Authorization, not bubble up as a HostRuntimeError
    // or get swallowed.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(DenyAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let context = execution_context_with_dispatch_grant();
    let request = RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({}),
        trust_decision_with_dispatch_authority(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(
                failure.kind,
                ironclaw_host_runtime::RuntimeFailureKind::Authorization
            );
        }
        other => panic!("expected Failed(Authorization), got {:?}", other),
    }
    // Deny must short-circuit before dispatch runs.
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn default_runtime_idempotency_key_is_advisory_and_does_not_dedupe() {
    // Pins the documented limitation: idempotency_key is advisory only at
    // this layer. Two invocations carrying the same key both reach dispatch.
    // If a future change wires dedupe through the capability host, this test
    // is the canary that flags the contract change.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(CountingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()));

    let key = IdempotencyKey::new("turn-1/tool-1").unwrap();

    let context_a = execution_context_with_dispatch_grant();
    let request_a = RuntimeCapabilityRequest::new(
        context_a,
        capability_id(),
        ResourceEstimate::default(),
        json!({"n": 1}),
        trust_decision_with_dispatch_authority(),
    )
    .with_idempotency_key(key.clone());
    let _ = runtime.invoke_capability(request_a).await.unwrap();

    let context_b = execution_context_with_dispatch_grant();
    let request_b = RuntimeCapabilityRequest::new(
        context_b,
        capability_id(),
        ResourceEstimate::default(),
        json!({"n": 2}),
        trust_decision_with_dispatch_authority(),
    )
    .with_idempotency_key(key);
    let _ = runtime.invoke_capability(request_b).await.unwrap();

    assert_eq!(
        dispatcher.count(),
        2,
        "idempotency_key is advisory only — dedupe is not enforced at this layer"
    );
}

#[tokio::test]
async fn default_runtime_status_returns_default_when_no_run_state_attached() {
    // Pins the no-run-state branch: callers must get an empty status rather
    // than a panic or an Unavailable error.
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let context = execution_context_with_dispatch_grant();
    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope,
            context.correlation_id,
        ))
        .await
        .unwrap();

    assert!(status.active_work.is_empty());
}

#[tokio::test]
async fn default_runtime_status_propagates_unavailable_on_run_state_error() {
    // Parallel to the approval-lookup path: a records_for_scope outage must
    // surface as HostRuntimeError::Unavailable with a redacted reason, not
    // leak the underlying filesystem string.
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let inner = Arc::new(InMemoryRunStateStore::new());
    let run_state: Arc<dyn RunStateStore> = Arc::new(FailingRecordsRunStateStore { inner });
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state);

    let context = execution_context_with_dispatch_grant();
    let error = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope,
            context.correlation_id,
        ))
        .await
        .expect_err("records_for_scope outage must surface as host runtime error");

    match error {
        ironclaw_host_runtime::HostRuntimeError::Unavailable { reason } => {
            assert!(
                !reason.contains("/private"),
                "sanitized reason must not leak filesystem paths, got {reason:?}"
            );
            assert_eq!(reason, "run-state filesystem unavailable");
        }
        other => panic!("expected HostRuntimeError::Unavailable, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_status_redacts_process_filesystem_errors() {
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_store: Arc<dyn ProcessStore> = Arc::new(FailingRecordsProcessStore {
        inner: Arc::new(ironclaw_processes::in_memory_backed_process_store()),
    });
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_store(process_store);

    let context = execution_context_with_dispatch_grant();
    let error = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope,
            context.correlation_id,
        ))
        .await
        .expect_err("process records_for_scope outage must surface as host runtime error");

    match error {
        HostRuntimeError::Unavailable { reason } => {
            assert!(
                !reason.contains("/tmp"),
                "sanitized reason must not leak filesystem paths, got {reason:?}"
            );
            assert_eq!(reason, "process filesystem unavailable");
        }
        other => panic!("expected HostRuntimeError::Unavailable, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_status_filters_to_running_records_only() {
    // Pins the filter: completed/failed/blocked records must not appear in
    // active_work. Surfacing terminal records as "active" would mislead
    // upper services about which work to wait on.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();

    let running_id = InvocationId::new();
    let completed_id = InvocationId::new();
    let failed_id = InvocationId::new();

    for invocation_id in [running_id, completed_id, failed_id] {
        run_state
            .start(ironclaw_run_state::RunStart {
                invocation_id,
                capability_id: capability_id(),
                scope: context.resource_scope.clone(),
                authenticated_actor_user_id: None,
            })
            .await
            .unwrap();
    }
    run_state
        .complete(&context.resource_scope, completed_id)
        .await
        .unwrap();
    run_state
        .fail(
            &context.resource_scope,
            failed_id,
            "BackendError".to_string(),
        )
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope.clone(),
            context.correlation_id,
        ))
        .await
        .unwrap();

    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Invocation(running_id)
    );
}

#[tokio::test]
async fn default_runtime_visible_capabilities_returns_empty_descriptors_for_empty_registry() {
    // Pins the empty-registry path: the surface still carries a deterministic
    // version derived from the configured base version and request policy.
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let context = execution_context_with_dispatch_grant();
    let surface = runtime
        .visible_capabilities(VisibleCapabilityRequest::new(
            context,
            SurfaceKind::new("agent_loop").unwrap(),
        ))
        .await
        .unwrap();

    assert_ne!(surface.version.as_str(), "surface-v1");
    assert!(surface.version.as_str().starts_with("sha256:"));
    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn default_runtime_returns_versioned_visible_surface_with_registry_descriptors() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry.clone(),
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()));

    let context = execution_context_with_dispatch_grant();
    let surface = runtime
        .visible_capabilities(visible_capability_request(context))
        .await
        .unwrap();

    assert_ne!(surface.version.as_str(), "surface-v1");
    assert!(surface.version.as_str().starts_with("sha256:"));
    assert_eq!(surface.capabilities.len(), 1);
    assert_eq!(surface.capabilities[0].descriptor.id, capability_id());
}

#[tokio::test]
async fn default_runtime_visible_surface_tracks_shared_registry_mutations() {
    let registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::from_shared_registry(
        Arc::clone(&registry),
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()));
    let empty_surface = runtime
        .visible_capabilities(visible_capability_request(
            execution_context_with_dispatch_grant(),
        ))
        .await
        .unwrap();
    assert!(empty_surface.capabilities.is_empty());

    registry
        .upsert(
            registry_with_echo_capability()
                .get_extension(&extension_id())
                .unwrap()
                .clone(),
        )
        .expect("insert visible extension");
    let populated_surface = runtime
        .visible_capabilities(visible_capability_request(
            execution_context_with_dispatch_grant(),
        ))
        .await
        .unwrap();
    assert_eq!(populated_surface.capabilities.len(), 1);
    assert_eq!(
        populated_surface.capabilities[0].descriptor.id,
        capability_id()
    );

    registry.remove(&extension_id());
    let removed_surface = runtime
        .visible_capabilities(visible_capability_request(
            execution_context_with_dispatch_grant(),
        ))
        .await
        .unwrap();
    assert!(removed_surface.capabilities.is_empty());
}

#[tokio::test]
async fn default_runtime_status_reports_running_invocations_only() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    run_state
        .start(ironclaw_run_state::RunStart {
            invocation_id: context.invocation_id,
            capability_id: capability_id(),
            scope: context.resource_scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope.clone(),
            context.correlation_id,
        ))
        .await
        .unwrap();

    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Invocation(context.invocation_id)
    );
    assert_eq!(status.active_work[0].capability_id, Some(capability_id()));
    assert_eq!(status.active_work[0].runtime, Some(RuntimeKind::Wasm));
}

#[tokio::test]
async fn default_runtime_cancel_reports_running_invocations_as_unsupported() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    run_state
        .start(RunStart {
            invocation_id: context.invocation_id,
            capability_id: capability_id(),
            scope: context.resource_scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            context.resource_scope,
            context.correlation_id,
            CancelReason::TurnCancelled,
        ))
        .await
        .unwrap();

    assert!(outcome.cancelled.is_empty());
    assert!(outcome.already_terminal.is_empty());
    assert_eq!(
        outcome.unsupported,
        vec![RuntimeWorkId::Invocation(context.invocation_id)]
    );
}

#[tokio::test]
async fn default_runtime_cancel_kills_running_processes_and_cancels_tokens() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_store(process_store.clone())
    .with_process_cancellation_registry(cancellation_registry.clone());

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    process_store
        .start(process_start(&context, process_id))
        .await
        .unwrap();
    let cancellation_token = cancellation_registry.register(&context.resource_scope, process_id);

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            context.resource_scope.clone(),
            context.correlation_id,
            CancelReason::TurnCancelled,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    assert!(outcome.already_terminal.is_empty());
    assert!(outcome.unsupported.is_empty());
    assert!(cancellation_token.is_cancelled());
    let record = process_store
        .get(&context.resource_scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn spawn_process_returns_unavailable_when_process_manager_is_none() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );
    let context = execution_context_with_dispatch_grant();

    let error = runtime
        .spawn_process(process_start(&context, ProcessId::new()))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        HostRuntimeError::Unavailable { reason } if reason == "process manager unavailable"
    ));
}

#[tokio::test]
async fn default_runtime_status_includes_running_processes_from_process_store() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_store(process_store.clone());

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    process_store
        .start(process_start(&context, process_id))
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope,
            context.correlation_id,
        ))
        .await
        .unwrap();

    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Process(process_id)
    );
    assert_eq!(status.active_work[0].capability_id, Some(capability_id()));
    assert_eq!(status.active_work[0].runtime, Some(RuntimeKind::Wasm));
}

#[tokio::test]
async fn default_runtime_cancel_writes_killed_process_result_record() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let processes_filesystem = ironclaw_processes::in_memory_backed_processes_filesystem();
    let process_store = Arc::new(FilesystemProcessStore::new(Arc::clone(
        &processes_filesystem,
    )));
    let result_store = Arc::new(FilesystemProcessResultStore::new(processes_filesystem));
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_store(process_store.clone())
    .with_process_result_store(result_store.clone())
    .with_process_cancellation_registry(cancellation_registry.clone());

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    process_store
        .start(process_start(&context, process_id))
        .await
        .unwrap();
    cancellation_registry.register(&context.resource_scope, process_id);

    let outcome = runtime
        .cancel_work(CancelRuntimeWorkRequest::new(
            context.resource_scope.clone(),
            context.correlation_id,
            CancelReason::TurnCancelled,
        ))
        .await
        .unwrap();

    assert_eq!(outcome.cancelled, vec![RuntimeWorkId::Process(process_id)]);
    let result = result_store
        .get(&context.resource_scope, process_id)
        .await
        .unwrap()
        .expect("killed process result should be persisted");
    assert_eq!(result.status, ProcessStatus::Killed);
    assert_eq!(result.output, None);
    assert_eq!(result.output_ref, None);
    assert_eq!(result.error_kind, None);
}

#[tokio::test]
async fn default_runtime_status_does_not_duplicate_process_backed_invocations() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone())
    .with_process_store(process_store.clone());

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    run_state
        .start(RunStart {
            invocation_id: context.invocation_id,
            capability_id: capability_id(),
            scope: context.resource_scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    process_store
        .start(process_start(&context, process_id))
        .await
        .unwrap();

    let status = runtime
        .runtime_status(RuntimeStatusRequest::new(
            context.resource_scope,
            context.correlation_id,
        ))
        .await
        .unwrap();

    assert_eq!(status.active_work.len(), 1);
    assert_eq!(
        status.active_work[0].work_id,
        RuntimeWorkId::Process(process_id)
    );
}

#[tokio::test]
async fn default_runtime_health_reports_ready_when_registry_requires_no_backends() {
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let health = runtime.health().await.unwrap();

    assert!(health.ready);
    assert!(health.missing_runtime_backends.is_empty());
}

#[tokio::test]
async fn default_runtime_health_without_probe_reports_required_runtimes_missing() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let health = runtime.health().await.unwrap();

    assert!(!health.ready);
    assert_eq!(health.missing_runtime_backends, vec![RuntimeKind::Wasm]);
}

#[tokio::test]
async fn default_runtime_health_uses_configured_backend_probe() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_runtime_health(Arc::new(HealthyRuntimeProbe));

    let health = runtime.health().await.unwrap();

    assert!(health.ready);
    assert!(health.missing_runtime_backends.is_empty());
}

struct HealthyRuntimeProbe;

#[async_trait]
impl RuntimeBackendHealth for HealthyRuntimeProbe {
    async fn missing_runtime_backends(
        &self,
        _required: &[RuntimeKind],
    ) -> Result<Vec<RuntimeKind>, HostRuntimeError> {
        Ok(Vec::new())
    }
}

fn process_start(context: &ExecutionContext, process_id: ProcessId) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: context.process_id,
        invocation_id: context.invocation_id,
        scope: context.resource_scope.clone(),
        authenticated_actor_user_id: context.authenticated_actor_user_id.clone(),
        extension_id: extension_id(),
        capability_id: capability_id(),
        runtime: RuntimeKind::Wasm,
        grants: context.grants.clone(),
        mounts: context.mounts.clone(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: json!({"message": "background"}),
    }
}

/// Wraps an [`InMemoryRunStateStore`] but fails every `records_for_scope`
/// call so we can exercise the runtime-status error-propagation path.
struct FailingRecordsRunStateStore {
    inner: Arc<InMemoryRunStateStore>,
}

#[async_trait]
impl RunStateStore for FailingRecordsRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ironclaw_host_api::ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        Err(RunStateError::Filesystem(
            "simulated read failure: /private/users/secret/runstate.db".to_string(),
        ))
    }
}

/// Wraps a `FilesystemProcessStore` but fails every `records_for_scope` call
/// so runtime-status exercises process-store error sanitization through the
/// public host-runtime path.
struct FailingRecordsProcessStore {
    inner: Arc<FilesystemProcessStore<InMemoryBackend>>,
}

#[async_trait]
impl ProcessStore for FailingRecordsProcessStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        self.inner.start(start).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.complete(scope, process_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.fail(scope, process_id, error_kind).await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.kill(scope, process_id).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        Err(FilesystemError::Backend {
            path: VirtualPath::new("/users/user1/processes").unwrap(),
            operation: FilesystemOperation::ListDir,
            reason: "simulated read failure: /tmp/processes.db connection refused".to_string(),
        }
        .into())
    }
}

/// Wraps an [`InMemoryRunStateStore`] but fails every `get` call so we can
/// exercise the approval-lookup error-propagation path. Writes pass through
/// to the inner store so the capability host can complete its own
/// `start`/`block_approval` writes before we reach the broken read.
struct FailingGetRunStateStore {
    inner: Arc<InMemoryRunStateStore>,
}

#[async_trait]
impl RunStateStore for FailingGetRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        Err(RunStateError::Filesystem(
            "simulated read failure: /tmp/runstate.db connection refused".to_string(),
        ))
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

struct RecordingCombinedRunStateApprovalStore {
    runs: InMemoryRunStateStore,
    approvals: InMemoryApprovalRequestStore,
    combined_calls: AtomicUsize,
    separate_save_calls: AtomicUsize,
}

impl RecordingCombinedRunStateApprovalStore {
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
impl RunStateStore for RecordingCombinedRunStateApprovalStore {
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
impl ApprovalRequestStore for RecordingCombinedRunStateApprovalStore {
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
impl RunStateApprovalStore for RecordingCombinedRunStateApprovalStore {
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

#[derive(Default)]
struct CountingDispatcher {
    count: Mutex<usize>,
}

impl CountingDispatcher {
    fn count(&self) -> usize {
        *self.count.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait]
impl CapabilityDispatcher for CountingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self.count.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

struct DenyAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for DenyAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::PolicyDenied,
        }
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    request: Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingDispatcher {
    fn has_request(&self) -> bool {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self
            .request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

struct ApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

fn registry_with_echo_capability() -> ExtensionRegistry {
    registry_with_echo_capability_permission("allow")
}

fn registry_with_echo_capability_permission(permission: &str) -> ExtensionRegistry {
    let manifest = format!(
        r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "{permission}"
parameters_schema = {{}}
"#
    );
    let manifest = parse_manifest(&manifest);
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
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

fn execution_context_with_dispatch_grant() -> ExecutionContext {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
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

fn visible_capability_request(context: ExecutionContext) -> VisibleCapabilityRequest {
    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(BTreeMap::from([(
            extension_id(),
            trust_decision_with_dispatch_authority(),
        )]))
}

fn local_manifest_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("echo").unwrap(),
            "/system/extensions/echo/manifest.toml".to_string(),
            None,
            HostTrustAssignment::user_trusted(),
            vec![EffectKind::DispatchCapability],
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

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("echo").unwrap()
}
