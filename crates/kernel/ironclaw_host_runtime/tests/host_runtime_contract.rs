// arch-exempt: large_file, mechanical lease-store test repoint to CapabilityLeaseStore<InMemoryBackend> helper (arch-simplification §4.3), no new test logic, plan #6168
mod support;

use support::legacy_capability_fixture_to_v2;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::{
    ApprovalRecord, ApprovalRequestStorePort, ApprovalResolver, ApprovalStoreError, LeaseApproval,
};
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStorePort, GrantAuthorizer,
    TrustAwareCapabilityDispatchAuthorizer, in_memory_backed_capability_lease_store,
};
use ironclaw_extension_registry::{
    ExtensionManifest, ExtensionManifestRecord, ExtensionPackage, ExtensionRegistry,
    MANIFEST_SCHEMA_VERSION_V3, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, ScopedFilesystem,
};
use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_host_api::dispatch_test_support::TestDispatcher;
use ironclaw_host_api::result_meta::FailureKind;
use ironclaw_host_api::{
    action::{Action, NetworkPolicy},
    approval::ApprovalRequest,
    capability::{
        CapabilityDescriptor, CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints,
    },
    decision::{Decision, DenyReason, Obligations},
    dispatch::{CapabilityDispatchResult, DispatchError},
    host_port::HostPortCatalog,
    ids::{
        ApprovalRequestId, CapabilityGrantId, CapabilityId, ExtensionId, InvocationId, PackageId,
        ProcessId, ResourceReservationId, RunId, UserId,
    },
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::{
        ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceScope, ResourceUsage,
    },
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{
    CancelReason, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, DefaultHostRuntime,
    HostRuntime, HostRuntimeError, IdempotencyKey, RuntimeBackendHealth, RuntimeStatusRequest,
    RuntimeWorkId, SurfaceKind, VisibleCapabilityRequest,
};
use ironclaw_processes::{
    ProcessCancellationRegistry, ProcessInvocationError, ProcessInvocationRecord,
    ProcessInvocationStart, ProcessInvocationStatePort, ProcessInvocationStatus,
    ProcessInvocationStore, ProcessJournalStore, ProcessResultStore, ProcessResultStorePort,
    ProcessServices, ProcessStart, ProcessStatus, capability_process_record,
    submit_capability_process,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::json;

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalHost,
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());

    let runtime = DefaultHostRuntime::new(
        registry.clone(),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone())
    .with_approval_requests(approval_requests.clone());

    let context = execution_context_with_dispatch_grant();
    let request = (
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

/// A capability bound to a standard messaging op (`standard_op:
/// Some(SendMessage)`) whose dispatch returns a shape that violates the
/// op's canonical output schema (missing `message_ref`) must not complete —
/// the runtime must reclassify it as a model-visible `Failed` outcome with
/// the same `InvalidOutput` kind wasm `InvalidResult` dispatch errors
/// already produce (see `failure_kind_from` in `production.rs`), carrying
/// the bounded "standard op output failed validation" summary.
///
/// `default_runtime_returns_completed_outcome_for_authorized_dispatch` above
/// is the paired regression check that a bespoke capability
/// (`standard_op: None`) dispatching the exact same bogus `{"ok": true}`
/// shape completes untouched — it already asserts `Completed` for the echo
/// capability with that output, so a hook that (incorrectly) fired for every
/// capability regardless of `standard_op` would fail it immediately.
#[tokio::test]
async fn standard_op_output_violation_becomes_failed_outcome() {
    let registry = Arc::new(registry_with_standard_op_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result_with_output(
        standard_op_capability_id(),
        standard_op_extension_id(),
        json!({"ok": true}),
    )));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(AllowAllAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let context = execution_context_for_standard_op();
    let request = (
        context,
        standard_op_capability_id(),
        ResourceEstimate::default(),
        json!({"conversation": "C1", "text": "hi"}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, FailureKind::InvalidResult);
            let message = failure.message.as_deref().unwrap_or_default();
            assert!(
                message.contains("standard op output failed validation"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_surfaces_approval_required_with_persisted_request_id() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone())
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = (
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
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
async fn default_runtime_persists_approval_and_blocks_invocation() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let stores = Arc::new(RecordingInvocationApprovalStores::new());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(stores.clone())
    .with_approval_requests(stores.clone())
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = (
        context.clone(),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id());
            assert_eq!(stores.save_calls(), 1);
            let record = ProcessInvocationStatePort::get(
                stores.as_ref(),
                &context.resource_scope,
                context.invocation_id,
            )
            .await
            .unwrap()
            .expect("run record persisted");
            assert_eq!(record.approval_request_id, Some(gate.approval_request_id));
            assert!(
                ApprovalRequestStorePort::get(
                    stores.as_ref(),
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
    // Regression: an earlier implementation swallowed `ApprovalStoreError` from
    // the approval-request lookup via `.ok().flatten()`, which masked storage
    // outages as a misleading "approval not persisted" Failed outcome. The
    // host runtime must instead surface persistence outages as
    // `HostRuntimeError::Unavailable` so callers can distinguish between a
    // missing record and a broken backend.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let inner_run_state =
        Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let run_state: Arc<dyn ProcessInvocationStatePort> = Arc::new(FailingGetRunStateStore {
        inner: inner_run_state.clone(),
    });
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state)
    .with_approval_requests(approval_requests)
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = (
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
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
async fn default_runtime_fresh_approval_without_persisted_request_fails_authorization() {
    // Regression: the response processor's Fresh-mode `AuthorizationRequiresApproval`
    // arm reads the approval request id back through `lookup_approval_request_id`
    // rather than trusting the capability kernel's in-band signal alone. If that
    // read-back finds no persisted record — even though the capability host's own
    // write succeeded — the invocation must fail as a model-visible authorization
    // failure instead of fabricating an `ApprovalRequired` gate the caller could
    // never resolve.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(ApprovalAuthorizer);
    let inner_run_state =
        Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let run_state: Arc<dyn ProcessInvocationStatePort> = Arc::new(InvisibleApprovalRunStateStore {
        inner: inner_run_state.clone(),
    });
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
        Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state)
    .with_approval_requests(approval_requests)
    .with_capability_leases(leases);

    let context = execution_context_with_dispatch_grant();
    let request = (
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
    );

    let outcome = runtime
        .invoke_capability(request)
        .await
        .expect("missing persisted approval must surface as a Failed outcome, not a host error");

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, FailureKind::Authorization);
            let message = failure.message.as_deref().unwrap_or_default();
            assert!(
                message.contains("no approval request was persisted"),
                "unexpected message: {message}"
            );
        }
        other => panic!(
            "expected Failed outcome with no approval gate, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn default_runtime_returns_failed_for_unknown_capability() {
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let request = (
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::MissingRuntime);
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert_eq!(
        dispatcher.call_count(),
        0,
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(DenyAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    );

    let context = execution_context_with_dispatch_grant();
    let request = (
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::Authorization);
        }
        other => panic!("expected Failed(Authorization), got {:?}", other),
    }
    // Deny must short-circuit before dispatch runs.
    assert_eq!(dispatcher.call_count(), 0);
}

// The four tests below extend caller-level coverage of
// `crate::capability_response_processor::process_capability_response` — the
// centralized mapping fresh invoke, approval resume, and auth resume all cross.
// They drive `DefaultHostRuntime`'s public entry points, not the processor
// directly, so a regression here pins the seam as callers actually exercise it.

#[tokio::test]
async fn default_runtime_fresh_dispatch_error_becomes_failed_and_fails_run_state() {
    // Fresh-invocation pin for the processor's `CapabilityInvocationError::Dispatch`
    // arm: a dispatcher failure distinct from AuthRequired/RequireApproval must
    // surface as `Failed`, and the processor's `fail_dispatch_run` side effect
    // must transition the already-started run-state record to `Failed`.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let request = (
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::UnknownProvider);
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);
    let record = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .expect("run record persisted before dispatch");
    assert_eq!(
        record.status,
        ProcessInvocationStatus::Failed,
        "a Dispatch error on fresh invocation must fail the started run-state record"
    );
}

#[tokio::test]
async fn default_runtime_fresh_dispatch_error_preserves_failure_when_fail_transition_fails() {
    // A fresh invocation's dispatch failure is model-visible even when the
    // best-effort durable transition cannot find or update its run record. The
    // transition failure must not replace the actionable provider failure with
    // HostRuntimeError::Unavailable.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let inner_run_state =
        Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let run_state = Arc::new(FailingDispatchTransitionRunStateStore::new(
        inner_run_state.clone(),
    ));
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state);

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let outcome = runtime
        .invoke_capability((
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .expect("fresh dispatch failure must remain model-visible");

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::UnknownProvider);
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);
    let record = inner_run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .expect("fresh invocation remains recorded when its transition fails");
    assert_eq!(record.status, ProcessInvocationStatus::Running);
}

#[tokio::test]
async fn default_runtime_fresh_auth_required_gate_is_stable_across_calls() {
    // Fresh-invocation pin for the processor's `AuthorizationRequiresAuth` arm:
    // two independent fresh invocations that hit the same auth requirement must
    // resolve to the same stable `RuntimeGateId`, since the gate id is derived
    // deterministically from capability + secrets + credential requirements
    // (production.rs::stable_auth_gate_id), not from invocation identity.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::auth_required());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()));

    let first = runtime
        .invoke_capability((
            execution_context_with_dispatch_grant(),
            capability_id(),
            ResourceEstimate::default(),
            json!({"n": 1}),
        ))
        .await
        .unwrap();
    let second = runtime
        .invoke_capability((
            execution_context_with_dispatch_grant(),
            capability_id(),
            ResourceEstimate::default(),
            json!({"n": 2}),
        ))
        .await
        .unwrap();

    let ironclaw_host_runtime::RuntimeCapabilityOutcome::AuthRequired(first_gate) = first else {
        panic!("expected AuthRequired outcome, got {:?}", first);
    };
    let ironclaw_host_runtime::RuntimeCapabilityOutcome::AuthRequired(second_gate) = second else {
        panic!("expected AuthRequired outcome, got {:?}", second);
    };
    assert_eq!(first_gate.capability_id, capability_id());
    assert_eq!(first_gate.gate_id, second_gate.gate_id);
    assert_eq!(dispatcher.call_count(), 2);
}

#[tokio::test]
async fn default_runtime_approval_resume_completes_and_consumes_grant() {
    // Approval-resume success: an approved gate resumed through
    // `resume_capability` re-authorizes with the injected lease grant and
    // completes, driving the processor's `Ok(dispatch)` arm under
    // `InlineInvocationMode::ApprovalResume`.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ApprovalThenGrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases = Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone())
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases.clone());

    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "hello"});

    let gate = match runtime
        .invoke_capability((
            context.clone(),
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap()
    {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    };

    let lease = ApprovalResolver::new(approval_requests.as_ref(), leases.as_ref())
        .approve_dispatch(
            &scope,
            gate.approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .expect("approve dispatch");

    let outcome = runtime
        .resume_capability((
            context,
            gate.approval_request_id,
            capability_id(),
            estimate,
            input,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
        }
        other => panic!("expected Completed resume outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);

    // The name promises the grant is consumed by the resumed dispatch, not
    // just that the outcome is `Completed` — assert the lease itself flips
    // to `Consumed` so a regression that leaves the grant re-usable (or
    // reverts to re-authorizing from a stale `Active` lease) fails here.
    let consumed_lease = leases
        .get(&scope, lease.grant.id)
        .await
        .expect("lease still present after resume");
    assert_eq!(consumed_lease.status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn default_runtime_approval_resume_repeated_approval_requirement_fails_without_reopening_gate()
 {
    // Pins the processor's `ApprovalResume` arm for `AuthorizationRequiresApproval`:
    // when the resumed authorization decision requires approval again (rather
    // than completing), the processor must surface a `Failed` outcome instead of
    // opening a second approval loop (see the "resume must never start a second
    // approval loop" comment in capability_response_processor.rs).
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(AlwaysRequireApprovalAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases = Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone())
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases.clone());

    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "hello"});

    let gate = match runtime
        .invoke_capability((
            context.clone(),
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap()
    {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    };

    ApprovalResolver::new(approval_requests.as_ref(), leases.as_ref())
        .approve_dispatch(
            &scope,
            gate.approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .expect("approve dispatch");

    let outcome = runtime
        .resume_capability((
            context,
            gate.approval_request_id,
            capability_id(),
            estimate,
            input,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::Authorization);
        }
        other => panic!(
            "expected Failed outcome (no second approval loop), got {:?}",
            other
        ),
    }
    // The authorizer requiring approval again must never re-dispatch.
    assert_eq!(dispatcher.call_count(), 0);
}

#[tokio::test]
async fn default_runtime_auth_resume_completes_blocked_auth_run() {
    // Auth-resume success (approval_request_id = None): a run parked in
    // `BlockedAuth` that resumes with an authorized grant completes, driving the
    // processor's `Ok(dispatch)` arm under `InlineInvocationMode::AuthResume`.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    run_state
        .start(ProcessInvocationStart {
            invocation_id,
            capability_id: capability_id(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .expect("seed running invocation");
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .expect("park invocation in BlockedAuth");

    let outcome = runtime
        .auth_resume_capability((
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            None,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
        }
        other => panic!("expected Completed auth-resume outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, ProcessInvocationStatus::Completed);
}

#[tokio::test]
async fn default_runtime_auth_resume_repeated_auth_requirement_reuses_stable_gate_id() {
    // Pins the processor's `AuthorizationRequiresAuth` arm on auth-resume: a
    // BlockedAuth run resumed against a dispatcher that still needs the same
    // credential surfaces `AuthRequired` again with the *same* stable gate id a
    // fresh invocation for the identical capability/requirements would have
    // produced — the gate id is a deterministic function of capability +
    // secrets + credential requirements, not invocation or mode identity.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::auth_required());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone());

    // A fresh invocation against the same always-auth-required dispatcher
    // establishes the baseline gate id.
    let fresh_gate = match runtime
        .invoke_capability((
            execution_context_with_dispatch_grant(),
            capability_id(),
            ResourceEstimate::default(),
            json!({"n": 1}),
        ))
        .await
        .unwrap()
    {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::AuthRequired(gate) => gate,
        other => panic!("expected AuthRequired outcome, got {:?}", other),
    };

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    run_state
        .start(ProcessInvocationStart {
            invocation_id,
            capability_id: capability_id(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .expect("seed running invocation");
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .expect("park invocation in BlockedAuth");

    let outcome = runtime
        .auth_resume_capability((
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"n": 1}),
            None,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::AuthRequired(resumed_gate) => {
            assert_eq!(resumed_gate.capability_id, capability_id());
            assert_eq!(resumed_gate.gate_id, fresh_gate.gate_id);
        }
        other => panic!(
            "expected AuthRequired outcome (no second approval loop), got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn default_runtime_approval_resume_dispatch_error_fails_and_terminalizes_run_state() {
    // Pins the processor's `CapabilityInvocationError::Dispatch` arm under
    // `InlineInvocationMode::ApprovalResume`: a dispatcher failure on a resumed
    // approval must surface `Failed` *and* terminalize the durable run-state
    // record, not leave it parked in a blocked/dispatching status forever
    // (deferred defect from #7686 CodeRabbit thread).
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ApprovalThenGrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases = Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone())
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases.clone());

    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "hello"});

    let gate = match runtime
        .invoke_capability((
            context.clone(),
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap()
    {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    };

    ApprovalResolver::new(approval_requests.as_ref(), leases.as_ref())
        .approve_dispatch(
            &scope,
            gate.approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .expect("approve dispatch");

    let outcome = runtime
        .resume_capability((
            context,
            gate.approval_request_id,
            capability_id(),
            estimate,
            input,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::UnknownProvider);
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);
    let record = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .expect("run record persisted before dispatch");
    assert_eq!(
        record.status,
        ProcessInvocationStatus::Failed,
        "a Dispatch error on approval resume must terminalize the run-state record"
    );
}

#[tokio::test]
async fn default_runtime_auth_resume_dispatch_error_fails_and_terminalizes_run_state() {
    // Pins the processor's `CapabilityInvocationError::Dispatch` arm under
    // `InlineInvocationMode::AuthResume`: a dispatcher failure on a resumed
    // auth run must surface `Failed` *and* terminalize the durable run-state
    // record instead of leaving it parked in `BlockedAuth` forever (deferred
    // defect from #7686 CodeRabbit thread).
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    run_state
        .start(ProcessInvocationStart {
            invocation_id,
            capability_id: capability_id(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .expect("seed running invocation");
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .expect("park invocation in BlockedAuth");

    let outcome = runtime
        .auth_resume_capability((
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            None,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, FailureKind::UnknownProvider);
        }
        other => panic!("expected Failed outcome, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 1);
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        ProcessInvocationStatus::Failed,
        "a Dispatch error on auth resume must terminalize the run-state record"
    );
}

#[tokio::test]
async fn default_runtime_approval_resume_surfaces_unavailable_when_fail_transition_fails() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ApprovalThenGrantAuthorizer);
    let inner_run_state =
        Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let run_state = Arc::new(FailingDispatchTransitionRunStateStore::new(
        inner_run_state.clone(),
    ));
    let approval_requests = Arc::new(ironclaw_approvals::in_memory_backed_approval_request_store());
    let leases = Arc::new(in_memory_backed_capability_lease_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state)
    .with_approval_requests(approval_requests.clone())
    .with_capability_leases(leases.clone());

    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "hello"});
    let gate = match runtime
        .invoke_capability((
            context.clone(),
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap()
    {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected ApprovalRequired outcome, got {:?}", other),
    };

    ApprovalResolver::new(approval_requests.as_ref(), leases.as_ref())
        .approve_dispatch(
            &scope,
            gate.approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .expect("approve dispatch");

    let error = runtime
        .resume_capability((
            context,
            gate.approval_request_id,
            capability_id(),
            estimate,
            input,
        ))
        .await
        .expect_err("durable fail-transition failure must be unavailable");
    assert_eq!(
        error,
        HostRuntimeError::Unavailable {
            reason: "process invocation backend unavailable".to_string(),
        }
    );

    let record = inner_run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .expect("blocked approval run remains durably recorded");
    assert_eq!(record.status, ProcessInvocationStatus::BlockedApproval);
    assert_eq!(record.approval_request_id, Some(gate.approval_request_id));
}

#[tokio::test]
async fn default_runtime_auth_resume_surfaces_unavailable_when_fail_transition_fails() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::responding(|_, _| {
        Err(DispatchError::UnknownProvider {
            capability: capability_id(),
            provider: extension_id(),
        })
    }));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let inner_run_state =
        Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let run_state = Arc::new(FailingDispatchTransitionRunStateStore::new(
        inner_run_state.clone(),
    ));

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    inner_run_state
        .start(ProcessInvocationStart {
            invocation_id,
            capability_id: capability_id(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .expect("seed running invocation");
    inner_run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .expect("park invocation in BlockedAuth");

    let error = runtime
        .auth_resume_capability((
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            None,
        ))
        .await
        .expect_err("durable fail-transition failure must be unavailable");
    assert_eq!(
        error,
        HostRuntimeError::Unavailable {
            reason: "process invocation backend unavailable".to_string(),
        }
    );

    let record = inner_run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .expect("blocked auth run remains durably recorded");
    assert_eq!(record.status, ProcessInvocationStatus::BlockedAuth);
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
            GrantAuthorizer
                .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
                .await
        }
    }
}

/// Always requires approval, ignoring any grant injected by the resume
/// preamble — pins that a resume decision landing back on
/// `AuthorizationRequiresApproval` fails the resume instead of reopening a
/// second approval loop.
struct AlwaysRequireApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for AlwaysRequireApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
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
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

#[tokio::test]
async fn default_runtime_repeated_invocations_are_not_deduped_by_host_runtime_request_shape() {
    // Host-runtime requests no longer carry a caller-provided idempotency key.
    // Loop-host owns invocation replay/dedup before this boundary; repeated
    // host-runtime invocations are ordinary independent dispatches.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()));

    let context_a = execution_context_with_dispatch_grant();
    let request_a = (
        context_a,
        capability_id(),
        ResourceEstimate::default(),
        json!({"n": 1}),
    );
    let _ = runtime.invoke_capability(request_a).await.unwrap();

    let context_b = execution_context_with_dispatch_grant();
    let request_b = (
        context_b,
        capability_id(),
        ResourceEstimate::default(),
        json!({"n": 2}),
    );
    let _ = runtime.invoke_capability(request_b).await.unwrap();

    assert_eq!(
        dispatcher.call_count(),
        2,
        "dedupe is enforced by loop-host before the host-runtime request boundary"
    );
}

#[tokio::test]
async fn default_runtime_status_returns_default_when_no_run_state_attached() {
    // Pins the no-run-state branch: callers must get an empty status rather
    // than a panic or an Unavailable error.
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
async fn default_runtime_status_propagates_unavailable_on_invocation_state_error() {
    // Parallel to the approval-lookup path: a records_for_scope outage must
    // surface as HostRuntimeError::Unavailable with a redacted reason, not
    // leak the underlying filesystem string.
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let inner = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let invocation_state: Arc<dyn ProcessInvocationStatePort> =
        Arc::new(FailingRecordsRunStateStore { inner });
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(invocation_state);

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
            assert_eq!(reason, "process invocation backend unavailable");
        }
        other => panic!("expected HostRuntimeError::Unavailable, got {:?}", other),
    }
}

#[tokio::test]
async fn default_runtime_status_redacts_process_filesystem_errors() {
    let registry = Arc::new(ExtensionRegistry::new());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    // Real process store over a fault backend armed to fail `records_for_scope`
    // (its ordered row query) with a leaky reason, so the test proves the host-runtime
    // path maps `ProcessError::Filesystem` to the sanitized, path-free
    // "process filesystem unavailable" through the production store.
    let backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::Query)
                .backend("simulated read failure: /tmp/processes.db connection refused"),
        ),
    );
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").unwrap(),
        VirtualPath::new("/engine/processes").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts));
    let process_store = Arc::new(ProcessJournalStore::new(scoped));
    let process_services = ProcessServices::new(
        process_store,
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
    );
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_services(process_services);

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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();

    let running_id = InvocationId::new();
    let completed_id = InvocationId::new();
    let failed_id = InvocationId::new();

    for invocation_id in [running_id, completed_id, failed_id] {
        run_state
            .start(ironclaw_processes::ProcessInvocationStart {
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_services = ProcessServices::in_memory();
    let process_runtime = process_services.process_runtime();
    let run_state = Arc::new(ProcessInvocationStore::new(Arc::clone(&process_runtime)));

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    run_state
        .start(ironclaw_processes::ProcessInvocationStart {
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
    let second_worker = ProcessInvocationStore::new(process_runtime);
    assert!(
        second_worker
            .records_for_scope(&context.resource_scope)
            .await
            .unwrap()
            .is_empty(),
        "fresh inline invocation state is local until its first durable edge"
    );
}

#[tokio::test]
async fn default_runtime_cancel_reports_running_invocations_as_unsupported() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone());

    let context = execution_context_with_dispatch_grant();
    run_state
        .start(ProcessInvocationStart {
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let process_services = ProcessServices::from_parts(
        process_store.clone(),
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
        cancellation_registry.clone(),
    );
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_services(process_services);

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    submit_capability_process(process_store.as_ref(), process_start(&context, process_id))
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
    let record =
        capability_process_record(process_store.as_ref(), &context.resource_scope, process_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(record.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn spawn_process_returns_unavailable_when_process_manager_is_none() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let process_services = ProcessServices::new(
        process_store.clone(),
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
    );
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_services(process_services);

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    submit_capability_process(process_store.as_ref(), process_start(&context, process_id))
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let processes_filesystem = ironclaw_processes::in_memory_backed_processes_filesystem();
    let process_store = Arc::new(ProcessJournalStore::new(Arc::clone(&processes_filesystem)));
    let result_store = Arc::new(ProcessResultStore::new(processes_filesystem));
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let process_services = ProcessServices::from_parts(
        process_store.clone(),
        result_store.clone(),
        cancellation_registry.clone(),
    );
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_process_services(process_services);

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    submit_capability_process(process_store.as_ref(), process_start(&context, process_id))
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_processes::in_memory_backed_process_invocation_state_store());
    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    let process_services = ProcessServices::new(
        process_store.clone(),
        Arc::new(ironclaw_processes::in_memory_backed_process_result_store()),
    );
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_invocation_state(run_state.clone())
    .with_process_services(process_services);

    let context = execution_context_with_dispatch_grant();
    let process_id = ProcessId::new();
    run_state
        .start(ProcessInvocationStart {
            invocation_id: context.invocation_id,
            capability_id: capability_id(),
            scope: context.resource_scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    submit_capability_process(process_store.as_ref(), process_start(&context, process_id))
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
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
        authorized_continuation: None,
        input: json!({"message": "background"}),
    }
}

/// Wraps an invocation store but fails every terminal dispatch transition.
///
/// The underlying record remains in its blocked state so callers can retry
/// after the durable backend recovers.
struct FailingDispatchTransitionRunStateStore {
    inner:
        Arc<ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>>,
}

impl FailingDispatchTransitionRunStateStore {
    fn new(
        inner: Arc<
            ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>,
        >,
    ) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ProcessInvocationStatePort for FailingDispatchTransitionRunStateStore {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        Err(ProcessInvocationError::Backend(
            "simulated dispatch transition failure: /private/users/secret/runstate.db".to_string(),
        ))
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        self.inner.records_for_scope(scope).await
    }
}

/// Wraps an [`ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>`] but fails every `records_for_scope`
/// call so we can exercise the runtime-status error-propagation path.
struct FailingRecordsRunStateStore {
    inner:
        Arc<ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>>,
}

#[async_trait]
impl ProcessInvocationStatePort for FailingRecordsRunStateStore {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        Err(ProcessInvocationError::Backend(
            "simulated read failure: /private/users/secret/runstate.db".to_string(),
        ))
    }
}

/// Wraps an [`ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>`] but fails every `get` call so we can
/// exercise the approval-lookup error-propagation path. Writes pass through
/// to the inner store so the capability host can complete its own
/// `start`/`block_approval` writes before we reach the broken read.
struct FailingGetRunStateStore {
    inner:
        Arc<ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>>,
}

#[async_trait]
impl ProcessInvocationStatePort for FailingGetRunStateStore {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        Err(ProcessInvocationError::Backend(
            "simulated read failure: /tmp/runstate.db connection refused".to_string(),
        ))
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        self.inner.records_for_scope(scope).await
    }
}

/// Wraps an [`ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>`]
/// but always answers `get` with `Ok(None)`, even after `start`/`block_approval`
/// writes have succeeded against the inner store. Simulates a read-your-write
/// gap between the capability host's approval-block write and the host
/// runtime's own approval-lookup read, so the response processor's
/// `AuthorizationRequiresApproval` arm sees no persisted record despite the
/// kernel having required approval.
struct InvisibleApprovalRunStateStore {
    inner:
        Arc<ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>>,
}

#[async_trait]
impl ProcessInvocationStatePort for InvisibleApprovalRunStateStore {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        Ok(None)
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        self.inner.records_for_scope(scope).await
    }
}

struct RecordingInvocationApprovalStores {
    runs: ironclaw_processes::ProcessInvocationStateStore<ironclaw_filesystem::InMemoryBackend>,
    approvals: ironclaw_approvals::ApprovalRequestStore<ironclaw_filesystem::InMemoryBackend>,
    save_calls: AtomicUsize,
}

impl RecordingInvocationApprovalStores {
    fn new() -> Self {
        Self {
            runs: ironclaw_processes::in_memory_backed_process_invocation_state_store(),
            approvals: ironclaw_approvals::in_memory_backed_approval_request_store(),
            save_calls: AtomicUsize::new(0),
        }
    }

    fn save_calls(&self) -> usize {
        self.save_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ProcessInvocationStatePort for RecordingInvocationApprovalStores {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.runs.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.runs
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.runs.block_auth(scope, invocation_id, error_kind).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.runs.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.runs.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        self.runs.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        self.runs.records_for_scope(scope).await
    }
}

#[async_trait]
impl ApprovalRequestStorePort for RecordingInvocationApprovalStores {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.save_calls.fetch_add(1, Ordering::SeqCst);
        self.approvals.save_pending(scope, request).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError> {
        self.approvals.get(scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.approvals.approve(scope, request_id).await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.approvals.deny(scope, request_id).await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.approvals.discard_pending(scope, request_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, ApprovalStoreError> {
        self.approvals.records_for_scope(scope).await
    }
}

fn dispatch_result() -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: capability_id(),
        provider: extension_id(),
        runtime: RuntimeKind::Wasm,
        output: json!({"ok": true}),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: ResourceScope::system(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

/// [`dispatch_result`] parameterized over capability/provider/output, for
/// scenarios (like the standard-op output check) that need a specific bogus
/// or canonical output shape rather than the fixed echo `{"ok": true}`.
fn dispatch_result_with_output(
    capability_id: CapabilityId,
    provider: ExtensionId,
    output: serde_json::Value,
) -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id,
        provider,
        runtime: RuntimeKind::Wasm,
        output,
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: ResourceScope::system(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

/// Unconditional-allow authorizer for scenarios that only need to reach
/// dispatch and don't care about grant/trust-ceiling matching (the standard-op
/// output check runs strictly after authorization succeeds).
struct AllowAllAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for AllowAllAuthorizer {
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
}

/// v3 manifest binding one tool to the `send_message` standard op (mirrors
/// `zeta_standard_op_manifest` in
/// `ironclaw_extensions/tests/manifest_v3_contract.rs`, trimmed to the
/// fields the standard-op binding rule actually requires: no credentials/auth
/// section, since this fixture never dispatches through a real WASM runtime
/// or credential path).
fn standard_op_manifest(extension_id: &str) -> String {
    format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "{extension_id}"
name = "Standard op test extension"
version = "0.1.0"
description = "Standard-op output validation test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"

[[tools]]
standard_op = "send_message"
id = "{extension_id}.send_message"
description = "Sends a message."
effects = ["external_write"]
default_permission = "allow"
"#
    )
}

/// Registry holding one capability bound to `StandardMessagingOp::SendMessage`
/// (`descriptor.standard_op == Some(SendMessage)`), built through the real v3
/// manifest parse path (`ExtensionManifestRecord::from_toml` ->
/// `ExtensionManifest` -> `ExtensionPackage`) rather than a hand-built
/// descriptor, so the same binding/validation rules a real extension install
/// goes through are exercised here too.
fn registry_with_standard_op_capability() -> ExtensionRegistry {
    let extension = standard_op_extension_id().as_str().to_string();
    let record = ExtensionManifestRecord::from_toml(
        standard_op_manifest(&extension),
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        None,
        &capability_provider_contracts(),
        None,
    )
    .expect("standard-op v3 manifest should parse");
    let manifest = ExtensionManifest::try_from(record.manifest().clone())
        .expect("manifest rebuild should succeed");
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{extension}")).unwrap(),
    )
    .expect("standard-op test package should build");
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn standard_op_capability_id() -> CapabilityId {
    CapabilityId::new("msgstd.send_message").unwrap()
}

fn standard_op_extension_id() -> ExtensionId {
    ExtensionId::new("msgstd").unwrap()
}

/// Execution context for the standard-op scenarios: no grants, because
/// [`AllowAllAuthorizer`] doesn't check them.
fn execution_context_for_standard_op() -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
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
        &capability_provider_contracts(),
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

/// A context with no pre-existing dispatch grant — the approval/auth resume
/// tests use this so the initial dispatch requires approval instead of
/// bypassing it, mirroring `execution_context_without_grants` in
/// `host_runtime_persistent_approvals_contract.rs`.
fn execution_context_without_grants() -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
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

fn capability_provider_contracts() -> ironclaw_extension_registry::HostApiContractRegistry {
    let mut contracts = ironclaw_extension_registry::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extension_registry::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
