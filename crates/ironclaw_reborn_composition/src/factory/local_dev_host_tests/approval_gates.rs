use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use ironclaw_approvals::{ApprovalResolver, DenyApproval, LeaseApproval};
use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern, Principal,
    ResourceEstimate, RuntimeKind, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, ECHO_CAPABILITY_ID, RuntimeApprovalGate, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeFailureKind,
    SHELL_CAPABILITY_ID,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use super::*;
use crate::local_dev_capability_policy::local_dev_one_shot_lease_approval;

#[tokio::test]
async fn local_dev_ask_destructive_shell_invocation_blocks_then_resumes_with_one_shot_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-approval-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo approved"});
    let context = shell_execution_context("local-dev-approval-owner", "thread-local-dev-approval");

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("shell invocation returns approval gate");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = blocked else {
        panic!("expected approval gate, got {blocked:?}");
    };
    assert_eq!(gate.capability_id, capability_id);
    let approval = local_runtime
        .approval_requests
        .get(&context.resource_scope, gate.approval_request_id)
        .await
        .expect("approval store read")
        .expect("approval request persisted");
    assert_eq!(approval.status, ApprovalStatus::Pending);

    approve_shell_dispatch(local_runtime, &context, &gate).await;

    let resumed = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            capability_id,
            estimate,
            input,
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("approved shell invocation resumes");
    assert!(
        matches!(resumed, RuntimeCapabilityOutcome::Completed(_)),
        "approved one-shot lease should allow resume, got {resumed:?}"
    );
    let leases = local_runtime
        .capability_leases
        .leases_for_scope(&context.resource_scope)
        .await;
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn local_dev_approved_shell_uses_injected_tenant_sandbox_process_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = Arc::new(RecordingSandboxTransport::default());
    let process_port = Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        transport.clone(),
    ));
    let services = build_reborn_services(
        RebornBuildInput::local_dev("sandbox-port-owner", dir.path().join("local-dev"))
            .with_runtime_policy(tenant_sandbox_process_policy())
            .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(
                process_port,
            )),
    )
    .await
    .expect("local-dev services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");
    let host_runtime = services.host_runtime.as_ref().expect("host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo composed sandbox", "timeout": 9});
    let context = shell_execution_context("sandbox-port-owner", "sandbox-port-thread");

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("shell invocation returns approval gate");
    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = blocked else {
        panic!("expected approval gate, got {blocked:?}");
    };
    approve_shell_dispatch(local_runtime, &context, &gate).await;
    let resumed = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            capability_id,
            estimate,
            input,
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("approved shell invocation resumes");
    let RuntimeCapabilityOutcome::Completed(completed) = resumed else {
        panic!("expected completed shell resume, got {resumed:?}");
    };

    assert_eq!(completed.output["sandboxed"], serde_json::json!(true));
    assert_eq!(
        completed.output["output"],
        serde_json::json!("sandbox port: echo composed sandbox")
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].command, "echo composed sandbox");
    assert_eq!(requests[0].timeout_secs, Some(9));
}

#[tokio::test]
async fn local_dev_yolo_shell_invocation_still_completes_without_approval_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");
    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-approval-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .await
    .expect("local-dev-yolo services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let context = shell_execution_context(
        "local-dev-yolo-approval-owner",
        "thread-local-yolo-approval",
    );

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo yolo"}),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("local-dev-yolo shell invocation succeeds");

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "local-dev-yolo should not gate shell through local-dev AskDestructive, got {outcome:?}"
    );
    assert!(
        local_runtime
            .approval_requests
            .records_for_scope(&context.resource_scope)
            .await
            .expect("approval store records")
            .into_iter()
            .filter(|record| record.status == ApprovalStatus::Pending)
            .count()
            == 0,
        "local-dev-yolo must not create a pending approval"
    );
}

#[derive(Debug, Default)]
struct RecordingSandboxTransport {
    requests: Mutex<Vec<ironclaw_host_runtime::CommandExecutionRequest>>,
}

#[async_trait::async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: ironclaw_host_runtime::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_runtime::CommandExecutionOutput,
        ironclaw_host_runtime::RuntimeProcessError,
    > {
        let command = request.command.clone();
        self.requests.lock().unwrap().push(request); // safety: test transport records requests under #[cfg(test)].
        Ok(ironclaw_host_runtime::CommandExecutionOutput {
            output: format!("sandbox port: {command}"),
            saved_output: None,
            exit_code: 0,
            // The injected transport acts as the sandbox, so the output is
            // sandboxed from the host process's perspective.
            sandboxed: true,
            duration: Duration::from_millis(5),
        })
    }
}

#[tokio::test]
async fn local_dev_denied_shell_approval_does_not_issue_resume_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-deny-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");
    let host_runtime = services.host_runtime.as_ref().expect("host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo denied"});
    let context = shell_execution_context("local-dev-deny-owner", "local-dev-deny-thread");

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("shell invocation returns approval gate");
    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = blocked else {
        panic!("expected approval gate, got {blocked:?}");
    };

    let resolver = ApprovalResolver::new(
        local_runtime.approval_requests.as_ref(),
        local_runtime.capability_leases.as_ref(),
    );
    resolver
        .deny(
            &context.resource_scope,
            gate.approval_request_id,
            DenyApproval {
                denied_by: Principal::HostRuntime,
            },
        )
        .await
        .expect("deny approval");

    let resumed = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            capability_id,
            estimate,
            input,
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("denied shell invocation returns failed outcome");
    let RuntimeCapabilityOutcome::Failed(failure) = resumed else {
        panic!("denied approval must not resume successfully, got {resumed:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert!(
        local_runtime
            .capability_leases
            .leases_for_scope(&context.resource_scope)
            .await
            .is_empty(),
        "denying approval must not issue a capability lease"
    );
}

fn shell_execution_context(user_id: &str, thread_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("local-dev-test-loop").expect("extension id"); // safety: static test id is valid.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: static test id is valid.
    let grantee = Principal::Extension(extension_id.clone());
    let grants = CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id,
            grantee,
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: shell_allowed_effects(),
                mounts: MountView::default(),
                network: shell_network_policy(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    };
    let mut context = ExecutionContext::local_default(
        UserId::new(user_id).expect("user id"), // safety: callers pass static valid test ids.
        extension_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .expect("execution context"); // safety: fixed test context should validate.
    let thread_id = ThreadId::new(thread_id).expect("thread id"); // safety: callers pass static valid test ids.
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id);
    context.validate().expect("thread-scoped context"); // safety: fixed test context should validate.
    context
}

fn shell_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::Network,
    ]
}

fn shell_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

async fn approve_shell_dispatch(
    local_runtime: &RebornLocalRuntimeServices,
    context: &ExecutionContext,
    gate: &RuntimeApprovalGate,
) {
    ApprovalResolver::new(
        local_runtime.approval_requests.as_ref(),
        local_runtime.capability_leases.as_ref(),
    )
    .approve_dispatch(
        &context.resource_scope,
        gate.approval_request_id,
        shell_lease_approval(),
    )
    .await
    .expect("approval issues shell lease"); // safety: test resolver should accept fixed approval.
}

fn shell_lease_approval() -> LeaseApproval {
    local_dev_one_shot_lease_approval(GrantConstraints {
        allowed_effects: shell_allowed_effects(),
        mounts: MountView::default(),
        network: shell_network_policy(),
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    })
}

fn trust_decision(allowed_effects: Vec<EffectKind>) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
}

fn tenant_sandbox_process_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    let mut policy = local_dev_policy();
    policy.process_backend = ironclaw_host_api::runtime_policy::ProcessBackendKind::TenantSandbox;
    policy
}

fn local_dev_minimal_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    let mut policy = local_dev_policy();
    // Minimal is a profile-scoped bypass, so model the resolver's local-yolo
    // output instead of only overriding the approval enum.
    policy.requested_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
    policy.resolved_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
    policy.approval_policy = ironclaw_host_api::runtime_policy::ApprovalPolicy::Minimal;
    policy
}

fn local_dev_minimal_enterprise_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy
{
    let mut policy = local_dev_policy();
    policy.resolved_profile =
        ironclaw_host_api::runtime_policy::RuntimeProfile::EnterpriseYoloDedicated;
    policy.approval_policy = ironclaw_host_api::runtime_policy::ApprovalPolicy::Minimal;
    policy
}

/// Minimal approval policy must complete effectful capabilities without any approval gate.
/// Verifies the runtime-profile policy allows Minimal bypass only for a yolo
/// profile.
#[tokio::test]
async fn local_dev_minimal_policy_shell_invocation_completes_without_approval_gate() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-minimal-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_minimal_policy()),
    )
    .await
    .expect("local-dev minimal services build"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: test-only helper in #[cfg(test)] module.
    let context = shell_execution_context("local-dev-minimal-owner", "thread-minimal-approval");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo minimal"}),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("minimal shell invocation completes"); // safety: test-only helper in #[cfg(test)] module.

    // Minimal policy must not create an approval gate.
    assert!(matches!(outcome, RuntimeCapabilityOutcome::Completed(_))); // safety: test-only assertion in #[cfg(test)] module.
}

#[tokio::test]
async fn local_dev_minimal_with_enterprise_profile_still_gates_shell() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("ent-minimal-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_minimal_enterprise_policy()),
    )
    .await
    .expect("local-dev minimal enterprise services build"); // safety: test-only helper in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: test-only helper in #[cfg(test)] module.
    let context = shell_execution_context("ent-minimal-owner", "thread-ent-minimal");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo ent"}),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("enterprise minimal shell invocation resolves"); // safety: test-only helper in #[cfg(test)] module.

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!("enterprise profile must keep gating even under Minimal, got {outcome:?}");
    };
    let approval = local_runtime
        .approval_requests
        .get(&context.resource_scope, gate.approval_request_id)
        .await
        .expect("approval store read") // safety: test-only helper in #[cfg(test)] module.
        .expect("approval request persisted"); // safety: test-only helper in #[cfg(test)] module.
    assert_eq!(approval.status, ApprovalStatus::Pending); // safety: test-only assertion in #[cfg(test)] module.
}

/// `authorize_spawn_with_trust` RequireApproval-then-resume end-to-end.
/// Verifies the spawn gating and resume path is wired correctly.
#[tokio::test]
async fn local_dev_ask_destructive_spawn_capability_blocks_then_resumes() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-spawn-approval-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only helper in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: test-only helper in #[cfg(test)] module.
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo spawn-approved"});
    let context =
        shell_execution_context("local-dev-spawn-approval-owner", "thread-spawn-approval");

    let blocked = host_runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("spawn invocation returns approval gate"); // safety: test-only helper in #[cfg(test)] module.

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = blocked else {
        panic!("expected approval gate on spawn, got {blocked:?}");
    };
    assert_eq!(gate.capability_id, capability_id); // safety: test-only assertion in #[cfg(test)] module.

    ApprovalResolver::new(
        local_runtime.approval_requests.as_ref(),
        local_runtime.capability_leases.as_ref(),
    )
    .approve_spawn(
        &context.resource_scope,
        gate.approval_request_id,
        shell_lease_approval(),
    )
    .await
    .expect("approval issues spawn lease"); // safety: test-only helper in #[cfg(test)] module.

    let resumed = host_runtime
        .resume_spawn_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            capability_id,
            estimate,
            input,
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("approved spawn invocation resumes"); // safety: test-only helper in #[cfg(test)] module.
    // spawn_capability returns SpawnedProcess (a live process handle), not Completed.
    let spawn_ok = matches!(
        resumed,
        RuntimeCapabilityOutcome::Completed(_) | RuntimeCapabilityOutcome::SpawnedProcess(_)
    );
    assert!(spawn_ok); // safety: test-only assertion in #[cfg(test)] module.
}

/// Spawning a dispatch-only builtin still exercises SpawnProcess, so the
/// approval gate must fire even though builtin.echo declares no destructive
/// effect in its own descriptor. Regression guard for the spawn fail-open:
/// gating against the raw descriptor effects (which exclude SpawnProcess, and
/// where DispatchCapability is not in ask_destructive) let echo spawn as a live
/// process ungated under AskDestructive.
#[tokio::test]
async fn local_dev_ask_destructive_spawn_dispatch_only_capability_requires_approval() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-echo-spawn-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability"); // safety: test-only helper in #[cfg(test)] module.
    let context = echo_spawn_execution_context("local-dev-echo-spawn-owner", "thread-echo-spawn");

    let outcome = host_runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"message": "spawn echo"}),
            trust_decision(echo_spawn_allowed_effects()),
        ))
        .await
        .expect("spawn invocation resolves"); // safety: test-only helper in #[cfg(test)] module.

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::ApprovalRequired(_)),
        "dispatch-only builtin.echo must gate on spawn via SpawnProcess elevation, got {outcome:?}"
    ); // safety: test-only assertion in #[cfg(test)] module.
}

fn echo_spawn_execution_context(user_id: &str, thread_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("local-dev-test-loop").expect("extension id"); // safety: static test id is valid.
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability"); // safety: static test id is valid.
    let grantee = Principal::Extension(extension_id.clone());
    let grants = CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id,
            grantee,
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: echo_spawn_allowed_effects(),
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    };
    let mut context = ExecutionContext::local_default(
        UserId::new(user_id).expect("user id"), // safety: callers pass static valid test ids.
        extension_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .expect("execution context"); // safety: fixed test context should validate.
    let thread_id = ThreadId::new(thread_id).expect("thread id"); // safety: callers pass static valid test ids.
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id);
    context.validate().expect("thread-scoped context"); // safety: fixed test context should validate.
    context
}

// builtin.echo declares only DispatchCapability in its descriptor. The grant and
// the trust authority ceiling must also cover SpawnProcess so the inner
// GrantAuthorizer authorizes the spawn (it authorizes against spawn_descriptor)
// and the request reaches the local-dev approval gate instead of being denied.
fn echo_spawn_allowed_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess]
}

/// A capability invoked without a matching grant must be denied, not upgraded to
/// RequireApproval. Verifies non-Allow pass-through in the profile approval
/// authorizer.
#[tokio::test]
async fn local_dev_ungranted_capability_returns_denied_not_approval_gate() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-deny-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    // Context grants only shell; apply_patch is not in the grant set.
    let context = shell_execution_context("local-dev-deny-owner", "thread-deny-passthrough");
    let capability_id =
        CapabilityId::new(APPLY_PATCH_CAPABILITY_ID).expect("apply_patch capability"); // safety: test-only helper in #[cfg(test)] module.

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({}),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("invocation completes (with failure)"); // safety: test-only helper in #[cfg(test)] module.

    // Ungranted capability must return Failed (Deny), not ApprovalRequired.
    assert!(matches!(outcome, RuntimeCapabilityOutcome::Failed(_))); // safety: test-only assertion in #[cfg(test)] module.
}

/// After a one-shot lease is consumed by the first resume, a second invocation
/// must present a new approval gate — not inherit the spent lease.
/// Verifies the one-shot property of `has_matching_one_shot_approval_grant`.
#[tokio::test]
async fn local_dev_one_shot_lease_regates_on_second_invocation() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-regate-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only helper in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: test-only helper in #[cfg(test)] module.
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo regate"});
    let context = shell_execution_context("local-dev-regate-owner", "thread-regate");

    // First invocation — expect approval gate.
    let first_blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("first invocation"); // safety: test-only helper in #[cfg(test)] module.
    let RuntimeCapabilityOutcome::ApprovalRequired(first_gate) = first_blocked else {
        panic!("expected first approval gate, got {first_blocked:?}");
    };

    // Approve and resume the first invocation.
    approve_shell_dispatch(local_runtime, &context, &first_gate).await;
    let first_resumed = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            first_gate.approval_request_id,
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("first resume"); // safety: test-only helper in #[cfg(test)] module.
    // First resume must complete.
    let first_ok = matches!(first_resumed, RuntimeCapabilityOutcome::Completed(_));
    assert!(first_ok); // safety: test-only assertion in #[cfg(test)] module.

    // Second invocation without a new approval — must gate again.
    // A fresh context is required because each invoke_capability call uses
    // context.invocation_id to key the run-state record; reusing the same
    // context would conflict with the completed first-invocation record.
    let context2 = shell_execution_context("local-dev-regate-owner", "thread-regate");
    let second = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context2,
            capability_id,
            estimate,
            input,
            trust_decision(shell_allowed_effects()),
        ))
        .await
        .expect("second invocation"); // safety: test-only helper in #[cfg(test)] module.
    // Spent one-shot lease must not bypass approval on second invocation.
    let regated = matches!(second, RuntimeCapabilityOutcome::ApprovalRequired(_));
    assert!(regated); // safety: test-only assertion in #[cfg(test)] module.
}
