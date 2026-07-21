use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_spawn_for_approval_without_starting_process() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let host = capability_host(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "background approval"});

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    assert!(dispatcher.call_count() == 0);
    assert!(!process_manager.has_start());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    let approval_id = run.approval_request_id.unwrap();
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(
        approval.request.invocation_fingerprint,
        Some(
            InvocationFingerprint::for_spawn(&scope, &capability_id(), &estimate, &input).unwrap()
        )
    );
}

#[tokio::test]
async fn capability_host_adds_sanitized_shell_command_to_spawn_approval_reason() {
    let manifest_toml = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme"
name = "Acme"
version = "0.1.0"
description = "Acme test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "acme.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "acme.shell"
description = "Runs a shell command."
effects = ["dispatch_capability", "spawn_process", "execute_code", "network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/shell.input.v1.json"
output_schema_ref = "schemas/shell.output.v1.json"
"#;
    let manifest = ExtensionManifest::parse(
        manifest_toml,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/acme").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let host = capability_host(&registry, &dispatcher, &ShellSpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let capability_id = CapabilityId::new("acme.shell").unwrap();
    let input = json!({
        "command": "curl -H 'Authorization: Bearer sk-secret' https://example.test/reset/sk-secret?token=secret"
    });

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id,
            estimate: ResourceEstimate::default(),
            input,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    assert!(!process_manager.has_start());
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        approval
            .request
            .reason
            .contains("Command:\ncurl -H 'Authorization: [redacted]'")
    );
    assert!(
        approval
            .request
            .reason
            .contains("https://example.test/reset/[redacted]?...")
    );
    assert!(!approval.request.reason.contains("sk-secret"));
    assert!(!approval.request.reason.contains("token=secret"));
}

#[tokio::test]
async fn capability_host_resumes_approved_spawn_and_consumes_matching_lease() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();
    let block_host = capability_host(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let mut context = execution_context(CapabilitySet::default());
    context.authenticated_actor_user_id = Some(UserId::new("slack-alice").unwrap());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved background"});

    block_host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_spawn(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
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
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let mut forged_context = context.clone();
    forged_context.authenticated_actor_user_id = Some(UserId::new("slack-bob").unwrap());
    let forged_error = resume_host
        .resume_spawn_json(CapabilityResumeRequest {
            context: forged_context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        forged_error,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::PolicyDenied,
            ..
        }
    ));
    assert!(dispatcher.call_count() == 0);
    assert!(!process_manager.has_start());
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::BlockedApproval
    );
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );

    let result = resume_host
        .resume_spawn_json(CapabilityResumeRequest {
            context: context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
        })
        .await
        .unwrap();

    assert!(dispatcher.call_count() == 0);
    let start = process_manager.take_start();
    assert_eq!(start.scope, context.resource_scope);
    assert_eq!(
        start
            .authenticated_actor_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("slack-alice")
    );
    assert_eq!(start.capability_id, capability_id());
    assert!(
        start
            .grants
            .grants
            .iter()
            .any(|grant| grant.id == lease.grant.id),
        "resumed spawned process must inherit the approved lease grant"
    );
    assert_eq!(result.process.process_id, start.process_id);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let consumed = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
}

// Finding (IronLoop, §5.3.2), resume-SPAWN path: an approved spawn whose lease
// expired between approval and resume must fail CLOSED at the witness-consume
// gate — the sealed witness is bounded by the claimed lease's `expires_at`, so a
// past expiry makes `dispatch_inputs_from_witness` fail before `ProcessManager::
// spawn`. Previously the expiry was only noticed on the later lease consume, so
// an expired-lease resume-spawn could start a process. Drives the production
// `resume_spawn_json` caller. The real filesystem lease store rejects an expired
// lease at match/claim (so the path is unreachable through it); an
// `ExpiredApprovalLeaseStore` double bypasses that gate — exactly as the
// in-`host.rs` `expired_resume_witness_dispatches_nothing_fails_run_and_revokes_lease`
// unit does — so the witness-consume fail-close is what is exercised.
#[tokio::test]
async fn capability_host_resume_spawn_fails_closed_on_expired_lease_witness() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = ExpiredApprovalLeaseStore::default();
    let block_host = capability_host(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let mut context = execution_context(CapabilitySet::default());
    context.authenticated_actor_user_id = Some(UserId::new("slack-alice").unwrap());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved background"});

    block_host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Approve with an already-past `expires_at`: the issued lease's expiry is the
    // frozen fact the sealed witness deadline is bounded by, so the witness is
    // expired the moment it is consumed on the resume.
    let past_expiry = chrono::Utc::now() - chrono::Duration::minutes(1);
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_spawn(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: Some(past_expiry),
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .unwrap();

    // Always-allow spawn authorizer that ALSO obliges a resource reservation, so
    // authorization passes and the resume reaches the witness-consume gate under
    // test with a non-empty prepared `obligation_outcome` (a grant-aware
    // authorizer would deny the expired lease grant one step earlier — a
    // different fail-close). Mirrors the in-`host.rs` unit's `AllowAuthorizer`.
    let reservation_id = ResourceReservationId::new();
    let resume_authorizer =
        ObligatingSpawnAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    // Recording handler so the witness-expiry cleanup's `abort_obligations` is
    // observable: `prepare` stages a reservation, `abort` records the release.
    let handler = RecordingSpawnObligationHandler::with_reservation(ResourceReservation {
        id: reservation_id,
        scope: scope.clone(),
        estimate: ResourceEstimate::default(),
    });
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases)
        .with_obligation_handler(&handler);

    let error = resume_host
        .resume_spawn_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
        })
        .await
        .unwrap_err();

    // Terminal fail-closed denial carrying the internal-invariant reason.
    assert!(
        matches!(
            error,
            CapabilityInvocationError::AuthorizationDenied {
                reason: DenyReason::InternalInvariantViolation,
                ..
            }
        ),
        "expired-lease resume-spawn must fail closed with InternalInvariantViolation, got {error:?}"
    );
    // No process was ever started, and the runtime was not dispatched inline.
    assert!(dispatcher.call_count() == 0);
    assert!(
        !process_manager.has_start(),
        "an expired-lease resume-spawn must NOT start a process"
    );
    // The run transitioned to Failed with the witness-expiry error kind.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("WitnessExpired"));
    // The lease was claimed (claim-before-witness ordering) and then REVOKED by
    // the terminal expiry cleanup — never consumed.
    assert!(
        *leases.claimed.lock().unwrap(),
        "the resume-spawn tail must claim the lease before the witness check"
    );
    assert_eq!(
        *leases.revoked.lock().unwrap(),
        Some(lease.grant.id),
        "an expired-lease resume-spawn must REVOKE the claimed lease (terminal), not consume it"
    );
    // The prepared reservation was staged and then RELEASED via the obligation
    // abort path (`dispatch_inputs_from_witness` → `abort_obligations`), not
    // leaked.
    assert!(
        handler.prepared(),
        "the resume-spawn tail must prepare the reservation obligation before the witness check"
    );
    assert!(
        handler.aborted(),
        "an expired-lease resume-spawn must ABORT the prepared reservation obligation, not leak it"
    );
}

// Finding (IronLoop, §5.3.2), resume-SPAWN path: an approved, non-expired
// spawn-resume whose `ExecutionContext` carries neither `origin` nor `run_id`
// (so `resolved_origin()` is `None`) must fail CLOSED at the spawn gate — deny
// with `InternalInvariantViolation`, start NO process, fail the run, AND revoke
// the already-claimed lease so a terminal refusal does not strand it. Every
// production ingress stamps an origin, so an origin-less resume is not a real
// ingress and an un-attributable process start must not proceed. Drives the
// production `resume_spawn_json` caller. Uses the SAME setup as
// `capability_host_resumes_approved_spawn_and_consumes_matching_lease` (the
// control shape, whose context resolves an origin) with the origin/run_id
// stripped from the resume context.
#[tokio::test]
async fn capability_host_resume_spawn_fails_closed_on_origin_less_context() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();
    let block_host = capability_host(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let mut context = execution_context(CapabilitySet::default());
    context.authenticated_actor_user_id = Some(UserId::new("slack-alice").unwrap());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved background"});

    block_host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    // Approve with no expiry so the deny is unambiguously the origin-less
    // fail-close, not an expiry-based one.
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_spawn(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
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
        .unwrap();

    // Always-allow spawn authorizer that ALSO obliges a resource reservation, so
    // the resume claims the lease, prepares a non-empty `obligation_outcome`, and
    // reaches the origin-less fail-close with a staged reservation to release.
    let reservation_id = ResourceReservationId::new();
    let resume_authorizer =
        ObligatingSpawnAuthorizer::new(vec![Obligation::ReserveResources { reservation_id }]);
    // Recording handler so the fail-close's `abort_obligations` is observable:
    // `prepare` stages a reservation, `abort` records the release.
    let handler = RecordingSpawnObligationHandler::with_reservation(ResourceReservation {
        id: reservation_id,
        scope: scope.clone(),
        estimate: ResourceEstimate::default(),
    });
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases)
        .with_obligation_handler(&handler);

    // Strip every ingress origin fact from the resume context: no `origin` and no
    // `run_id`, so `resolved_origin()` is `None`. The membrane-sealed actor is
    // left set to prove the deny is the origin-less fail-close, not a missing
    // actor or an actor mismatch.
    let mut origin_less = context.clone();
    origin_less.origin = None;
    origin_less.run_id = None;
    assert!(
        origin_less.resolved_origin().is_none(),
        "test fixture must be origin-less for this to exercise the fail-close"
    );

    let error = resume_host
        .resume_spawn_json(CapabilityResumeRequest {
            context: origin_less,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
        })
        .await
        .unwrap_err();

    // Terminal fail-closed denial carrying the internal-invariant reason.
    assert!(
        matches!(
            error,
            CapabilityInvocationError::AuthorizationDenied {
                reason: DenyReason::InternalInvariantViolation,
                ..
            }
        ),
        "origin-less resume-spawn must fail closed with InternalInvariantViolation, got {error:?}"
    );
    // No process was ever started, and the runtime was not dispatched inline.
    assert!(dispatcher.call_count() == 0);
    assert!(
        !process_manager.has_start(),
        "an origin-less resume-spawn must NOT start a process"
    );
    // The run transitioned to Failed with the authorization-denied error kind.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("AuthorizationDenied"));
    // The claimed lease is revoked (terminal), not left Claimed or consumed.
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Revoked,
        "an origin-less resume-spawn must REVOKE the claimed lease"
    );
    // The prepared reservation was staged and then RELEASED via the fail-close's
    // `abort_obligations` call (MEDIUM finding #4) — the origin-less denial must
    // not leak the reservation / staged handoff.
    assert!(
        handler.prepared(),
        "the resume-spawn tail must prepare the reservation obligation before the origin check"
    );
    assert!(
        handler.aborted(),
        "the origin-less fail-close must ABORT the prepared reservation obligation, not leak it"
    );
}

#[tokio::test]
async fn capability_host_denies_spawn_when_trust_ceiling_omits_spawn_effect() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let authorizer = GrantAuthorizer::new();
    // The kernel computes trust in-fold (§5.3.2/§9); inject a trust policy whose
    // authority ceiling omits the SpawnProcess effect so the trust-aware
    // authorizer denies the spawn on the trust ceiling.
    let trust_policy = FixedTrustPolicy::with_effects(vec![EffectKind::DispatchCapability]);
    let host =
        capability_host_with_trust_policy(&registry, &dispatcher, &authorizer, &trust_policy)
            .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![spawn_grant()],
    });

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked spawn"}),
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
    assert!(dispatcher.call_count() == 0);
    assert!(!process_manager.has_start());
}

#[tokio::test]
async fn capability_host_returns_spawn_result_when_run_completion_fails_after_spawn() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let run_state = FailCompleteRunStateStore::new();
    let authorizer = SpawnAuthorizer;
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background"}),
        })
        .await
        .unwrap();

    assert!(dispatcher.call_count() == 0);
    let start = process_manager.take_start();
    assert_eq!(result.process.process_id, start.process_id);
}

#[tokio::test]
async fn capability_host_spawns_authorized_process_without_dispatching_inline() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let process_manager = RecordingProcessManager::default();
    let authorizer = SpawnAuthorizer;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background"}),
        })
        .await
        .unwrap();

    assert!(dispatcher.call_count() == 0);
    let start = process_manager.take_start();
    assert_eq!(start.scope, context.resource_scope);
    assert_eq!(start.capability_id, capability_id());
    assert_eq!(start.extension_id, ExtensionId::new("echo").unwrap());
    assert_eq!(start.runtime, RuntimeKind::Wasm);
    assert_eq!(start.input, json!({"message": "background"}));
    assert_eq!(result.process.process_id, start.process_id);
}

/// Lease-store double for the expired-witness resume-spawn test. Unlike the real
/// filesystem store, it does NOT filter or reject on expiry: it returns the
/// issued lease from `active_leases_for_context` and lets `claim` succeed even
/// though `expires_at` is in the past, so `resume_spawn_json` reaches the
/// witness-consume gate that is the code under test (the real store rejects an
/// expired lease at match/claim, hiding that gate). Records the single `claim`
/// and the single terminal `revoke`; `consume` must never be reached on the
/// expiry path. Mirrors `ClaimRecordingLeaseStore` in `host.rs`, extended with
/// `issue`/`active_leases_for_context` so the full `resume_spawn_json` caller can
/// find and claim the lease.
#[derive(Default)]
struct ExpiredApprovalLeaseStore {
    lease: Mutex<Option<CapabilityLease>>,
    claimed: Mutex<bool>,
    revoked: Mutex<Option<CapabilityGrantId>>,
}

impl ExpiredApprovalLeaseStore {
    fn stored(&self) -> CapabilityLease {
        self.lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("lease must be issued before it is read")
    }

    fn stored_as_vec(&self) -> Vec<CapabilityLease> {
        self.lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .into_iter()
            .collect()
    }
}

#[async_trait]
impl CapabilityLeaseStore for ExpiredApprovalLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
        *self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(lease.clone());
        Ok(lease)
    }

    async fn revoke(
        &self,
        _scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        *self
            .revoked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(lease_id);
        let mut lease = self.stored();
        lease.status = CapabilityLeaseStatus::Revoked;
        Ok(lease)
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        Some(self.stored())
    }

    async fn claim(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
        _invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        *self
            .claimed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        let mut lease = self.stored();
        // Bypass the real store's expiry gate: the whole point is to reach the
        // witness-consume check with a claimed-but-expired lease.
        lease.status = CapabilityLeaseStatus::Claimed;
        Ok(lease)
    }

    async fn consume(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        unimplemented!("an expired resume-spawn witness must fail before the lease consume")
    }

    async fn begin_dispatch_claimed(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
        _invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        unimplemented!("resume-spawn does not use dispatch-claimed transitions")
    }

    async fn abort_dispatch_claimed(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        unimplemented!("resume-spawn does not use dispatch-claimed transitions")
    }

    async fn leases_for_scope(&self, _scope: &ResourceScope) -> Vec<CapabilityLease> {
        self.stored_as_vec()
    }

    async fn active_leases_for_context(&self, _context: &ExecutionContext) -> Vec<CapabilityLease> {
        // Deliberately ignores expiry so the expired lease is still matched by
        // `matching_approval_lease` — the real store would filter it out.
        self.stored_as_vec()
    }
}

/// Always-allow spawn authorizer that attaches a fixed obligation set on the
/// spawn path, so `prepare_obligations` produces a non-empty `obligation_outcome`
/// the fail-close branches then abort. Dispatch is denied (unused on this path).
/// Mirrors `ObligatingAuthorizer` in `capability_obligation_handler_contract.rs`,
/// scoped to the spawn arm the resume-spawn fail-close tests exercise.
struct ObligatingSpawnAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingSpawnAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingSpawnAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

/// Obligation handler that stages a reservation on `prepare` and records the
/// `abort` release, so the resume-spawn fail-close paths can PROVE the prepared
/// obligation was aborted (not leaked). Models `EffectObligationHandler` in
/// `capability_obligation_handler_contract.rs`, adding a `prepared` flag.
struct RecordingSpawnObligationHandler {
    reservation: Option<ResourceReservation>,
    prepared: AtomicBool,
    aborted: AtomicBool,
}

impl RecordingSpawnObligationHandler {
    fn with_reservation(reservation: ResourceReservation) -> Self {
        Self {
            reservation: Some(reservation),
            prepared: AtomicBool::new(false),
            aborted: AtomicBool::new(false),
        }
    }

    fn prepared(&self) -> bool {
        self.prepared.load(Ordering::SeqCst)
    }

    fn aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CapabilityObligationHandler for RecordingSpawnObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        self.prepared.store(true, Ordering::SeqCst);
        Ok(CapabilityObligationOutcome {
            mounts: None,
            resource_reservation: self.reservation.clone(),
        })
    }

    async fn abort(
        &self,
        _request: CapabilityObligationAbortRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.aborted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingProcessManager {
    start: Mutex<Option<ProcessStart>>,
}

impl RecordingProcessManager {
    fn take_start(&self) -> ProcessStart {
        self.start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap()
    }

    fn has_start(&self) -> bool {
        self.start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

struct FailCompleteRunStateStore {
    inner: ironclaw_run_state::FilesystemRunStateStore<ironclaw_filesystem::InMemoryBackend>,
}

impl FailCompleteRunStateStore {
    fn new() -> Self {
        Self {
            inner: ironclaw_run_state::in_memory_backed_run_state_store(),
        }
    }
}

#[async_trait]
impl RunStateStore for FailCompleteRunStateStore {
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
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "complete transition unavailable".to_string(),
        ))
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
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

#[async_trait]
impl ProcessManager for RecordingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        *self
            .start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(start.clone());
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
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

struct SpawnApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::SpawnCapability {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "spawn approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

struct ShellSpawnApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ShellSpawnApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
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
    }
}

struct SpawnAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

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
