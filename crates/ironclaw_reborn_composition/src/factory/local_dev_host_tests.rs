use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use ironclaw_approvals::{ApprovalResolver, LeaseApproval};
use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountPermissions, MountView, NetworkPolicy,
    NetworkTargetPattern, Principal, ResourceEstimate, RuntimeKind, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    RuntimeApprovalGate, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, SHELL_CAPABILITY_ID,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use super::*;

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
        self.requests.lock().unwrap().push(request);
        Ok(ironclaw_host_runtime::CommandExecutionOutput {
            output: format!("sandbox port: {command}"),
            exit_code: 0,
            sandboxed: false,
            duration: Duration::from_millis(5),
        })
    }
}

fn shell_execution_context(user_id: &str, thread_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("local-dev-test-loop").expect("extension id");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
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
        UserId::new(user_id).expect("user id"),
        extension_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .expect("execution context");
    let thread_id = ThreadId::new(thread_id).expect("thread id");
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id);
    context.validate().expect("thread-scoped context");
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
    .expect("approval issues shell lease");
}

fn shell_lease_approval() -> LeaseApproval {
    LeaseApproval {
        issued_by: Principal::HostRuntime,
        allowed_effects: shell_allowed_effects(),
        mounts: MountView::default(),
        network: shell_network_policy(),
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: Some(1),
    }
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

#[tokio::test]
async fn local_yolo_policy_mounts_confirmed_host_home_as_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home.clone()),
    )
    .await
    .expect("local-dev-yolo services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");

    let host_mount = local_runtime
        .workspace_mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/host")
        .expect("host mount exists");
    assert_eq!(host_mount.target.as_str(), "/projects/host");
    assert_eq!(host_mount.permissions, MountPermissions::read_write());

    let raw_host_home_alias = host_home
        .canonicalize()
        .expect("canonical host home")
        .to_string_lossy()
        .into_owned();
    let raw_host_home_mount = local_runtime
        .workspace_mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == raw_host_home_alias)
        .expect("raw host home mount exists");
    assert_eq!(raw_host_home_mount.target.as_str(), "/projects/host");
    assert_eq!(
        raw_host_home_mount.permissions,
        MountPermissions::read_write()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_yolo_policy_keeps_symlinked_host_home_raw_alias() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    let host_home_link = dir.path().join("home-link");
    std::fs::create_dir_all(&host_home).expect("host home root"); // safety: test-only setup in #[cfg(test)] module.
    std::os::unix::fs::symlink(&host_home, &host_home_link).expect("host home symlink"); // safety: test-only setup in #[cfg(test)] module.

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home_link.clone()),
    )
    .await
    .expect("local-dev-yolo services build"); // safety: test-only assertion in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.

    let raw_aliases = local_runtime
        .workspace_mounts
        .mounts
        .iter()
        .map(|mount| mount.alias.as_str())
        .collect::<Vec<_>>();
    let raw_alias_includes_original =
        raw_aliases.contains(&host_home_link.to_str().expect("utf-8 link path")); // safety: temp paths are test-owned.
    assert!(raw_alias_includes_original); // safety: test-only assertion in #[cfg(test)] module.
    let canonical_host_home = host_home
        .canonicalize()
        .expect("canonical home") // safety: test setup created this path.
        .to_str()
        .expect("utf-8 canonical path") // safety: temp paths are test-owned.
        .to_string();
    let raw_alias_includes_canonical = raw_aliases.contains(&canonical_host_home.as_str());
    assert!(raw_alias_includes_canonical); // safety: test-only assertion in #[cfg(test)] module.
}

#[tokio::test]
async fn local_yolo_policy_requires_confirmed_host_home_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy()),
    )
    .await
    .expect_err("host home policy needs confirmed root");

    assert!(format!("{error}").contains("confirmed host home root"));
}

#[tokio::test]
async fn confirmed_host_home_root_is_rejected_without_matching_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let error = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-host-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy())
            .with_local_dev_confirmed_host_home_root(host_home),
    )
    .await
    .expect_err("host home root needs matching policy");

    assert!(format!("{error}").contains("does not allow host home access"));
}

#[tokio::test]
async fn local_yolo_policy_rejects_confirmed_host_home_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home_file = dir.path().join("home-file");
    std::fs::write(&host_home_file, "not a directory").expect("host home file");

    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home_file),
    )
    .await
    .expect_err("host home root must be a directory");

    assert!(format!("{error}").contains("must be an existing directory"));
}

#[tokio::test]
async fn local_yolo_policy_rejects_confirmed_host_home_filesystem_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(filesystem_root()),
    )
    .await
    .expect_err("host home root must not be a filesystem root");

    assert!(format!("{error}").contains("must not be a filesystem root"));
}

fn local_yolo_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn local_dev_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::local_dev_runtime_policy().expect("local-dev policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn tenant_sandbox_process_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    let mut policy = local_dev_policy();
    policy.process_backend = ironclaw_host_api::runtime_policy::ProcessBackendKind::TenantSandbox;
    policy
}

fn filesystem_root() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("current dir"); // safety: test-only helper in #[cfg(test)] module.
    while let Some(parent) = path.parent() {
        path = parent.to_path_buf();
    }
    path
}
