use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ironclaw_approvals::{
    ApprovalResolver, AutoApproveSettingInput, AutoApproveSettingStore,
    CapabilityPermissionOverrideStore, DenyApproval, LeaseApproval, PersistentApprovalAction,
    PersistentApprovalPolicyInput, PersistentApprovalPolicyStore, ToolPermissionOverride,
    ToolPermissionOverrideInput,
};
use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern, Principal,
    ResourceEstimate, ResourceScope, RunId, RuntimeKind, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, ECHO_CAPABILITY_ID,
    RuntimeApprovalGate, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeFailureKind, SHELL_CAPABILITY_ID,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus};

use super::*;
use crate::builtin_capability_policy::builtin_one_shot_lease_approval;

use crate::approval_test_support::disable_global_auto_approve;

#[tokio::test]
async fn local_dev_ask_destructive_shell_invocation_blocks_then_resumes_with_one_shot_lease() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only fixture setup.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-approval-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only local-dev fixture setup.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only service fixture invariant.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only service fixture invariant.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: constant capability id.
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo approved"});
    let context = shell_execution_context("local-dev-approval-owner", "thread-local-dev-approval");
    disable_global_auto_approve(local_runtime, &context).await;

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
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
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only fixture setup.
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
    .expect("local-dev services build"); // safety: test-only local-dev fixture setup.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only service fixture invariant.
    let host_runtime = services.host_runtime.as_ref().expect("host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"command": "echo composed sandbox", "timeout": 9});
    let context = shell_execution_context("sandbox-port-owner", "sandbox-port-thread");
    disable_global_auto_approve(local_runtime, &context).await;

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
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
async fn local_dev_yolo_shell_invocation_asks_when_global_auto_approve_is_off() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only fixture setup.
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
    disable_global_auto_approve(local_runtime, &context).await;

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo yolo"}),
        ))
        .await
        .expect("local-dev-yolo shell invocation resolves");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!(
            "global auto-approve off should gate local-dev-yolo shell invocation, got {outcome:?}"
        );
    };
    assert_eq!(gate.capability_id, capability_id);
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "local-dev-yolo with global auto-approve off must create a pending approval"
    );
}

#[tokio::test]
async fn local_dev_auto_approve_setting_update_skips_next_shell_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-auto-approve-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only local-dev fixture setup.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only service fixture invariant.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime");
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability");
    let context = shell_execution_context(
        "local-dev-auto-approve-owner",
        "thread-local-dev-auto-approve",
    );

    local_runtime
        .auto_approve_settings
        .set(AutoApproveSettingInput {
            scope: context.resource_scope.clone(),
            enabled: true,
            updated_by: Principal::User(context.user_id.clone()),
        })
        .await
        .expect("auto-approve setting update");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo auto approve"}),
        ))
        .await
        .expect("auto-approved shell invocation succeeds");

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "updated auto-approve setting should skip the shell approval gate, got {outcome:?}"
    );
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        0,
        "auto-approved invocation must not create a pending approval"
    );
}

#[tokio::test]
async fn local_dev_default_allow_echo_auto_approves_when_global_unset() {
    // Caller-level proof of the PR's promise: a fresh user (auto-approve setting
    // never written → defaults ON) has an eligible tool auto-approved at
    // dispatch, with no approval gate. No disable call — the default must carry.
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only fixture setup.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-echo-default-on", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy()),
    )
    .await
    .expect("local-dev services build"); // safety: test-only local-dev fixture setup.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only service fixture invariant.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only service fixture invariant.
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability"); // safety: constant capability id.
    let context =
        echo_spawn_execution_context("local-dev-echo-default-on", "thread-echo-default-on");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"message": "auto approve echo"}),
        ))
        .await
        .expect("echo invocation resolves"); // safety: test-only capability invocation assertion.

    if !matches!(outcome, RuntimeCapabilityOutcome::Completed(_)) {
        panic!(
            "unset global auto-approve defaults ON, so eligible echo must auto-approve, got {outcome:?}"
        );
    }
    let pending_count = pending_approval_count(local_runtime, &context).await;
    if pending_count != 0 {
        panic!("default-on auto-approve must not create a pending approval");
    }
}

#[tokio::test]
async fn local_dev_default_allow_echo_asks_when_global_auto_approve_is_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-echo-default-ask", dir.path().join("local-dev"))
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
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability");
    let context = echo_spawn_execution_context("local-dev-echo-default-ask", "thread-echo-ask");
    disable_global_auto_approve(local_runtime, &context).await;

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            ResourceEstimate::default(),
            serde_json::json!({"message": "ask for echo"}),
        ))
        .await
        .expect("echo invocation resolves");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!(
            "default-allow builtin.echo should ask when global auto-approve is off, got {outcome:?}"
        );
    };
    assert_eq!(gate.capability_id, capability_id);
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "default-allow builtin.echo must create a pending approval when global auto-approve is off"
    );
}

#[tokio::test]
async fn local_dev_ask_each_time_echo_approval_resume_uses_one_shot_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-echo-ask-resume", dir.path().join("local-dev"))
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
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability");
    let estimate = ResourceEstimate::default();
    let input = serde_json::json!({"message": "hello ask-each-time"});
    let context = echo_spawn_execution_context("local-dev-echo-ask-resume", "thread-echo-resume");

    local_runtime
        .tool_permission_overrides
        .set(ToolPermissionOverrideInput {
            scope: operator_tool_permission_scope_for_test(&context.resource_scope),
            capability_id: capability_id.clone(),
            state: ToolPermissionOverride::AskEachTime,
            updated_by: Principal::User(context.user_id.clone()),
        })
        .await
        .expect("tool permission override update");

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .expect("echo invocation resolves");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = blocked else {
        panic!("explicit ask_each_time should gate builtin.echo, got {blocked:?}");
    };
    assert_eq!(gate.capability_id, capability_id);
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "explicit ask_each_time must create a pending approval"
    );

    let premature_resume = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .expect("pending echo approval resume resolves to non-completion");
    assert!(
        !matches!(premature_resume, RuntimeCapabilityOutcome::Completed(_)),
        "pending ask_each_time approval must not allow echo resume, got {premature_resume:?}"
    );
    assert!(
        local_runtime
            .capability_leases
            .leases_for_scope(&context.resource_scope)
            .await
            .is_empty(),
        "pending approval must not issue an approval lease"
    );

    let lease = ApprovalResolver::new(
        local_runtime.approval_requests.as_ref(),
        local_runtime.capability_leases.as_ref(),
    )
    .approve_dispatch(
        &context.resource_scope,
        gate.approval_request_id,
        echo_dispatch_lease_approval(),
    )
    .await
    .expect("approval issues echo lease");
    let approved_record = local_runtime
        .approval_requests
        .get(&context.resource_scope, gate.approval_request_id)
        .await
        .expect("approval record lookup")
        .expect("approval record exists");
    assert_eq!(approved_record.status, ApprovalStatus::Approved);
    assert!(
        lease.invocation_fingerprint.is_some(),
        "approval lease must be tied to the approved invocation fingerprint"
    );
    assert_eq!(
        lease.invocation_fingerprint.as_ref(),
        approved_record.request.invocation_fingerprint.as_ref(),
        "approval lease fingerprint must match the approved request"
    );

    let resumed = host_runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            capability_id,
            estimate,
            input,
        ))
        .await
        .expect("approved echo invocation resumes");
    assert!(
        matches!(resumed, RuntimeCapabilityOutcome::Completed(_)),
        "approved ask_each_time one-shot lease should allow echo resume, got {resumed:?}"
    );
    let leases = local_runtime
        .capability_leases
        .leases_for_scope(&context.resource_scope)
        .await;
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn local_dev_legacy_persistent_echo_grant_does_not_override_global_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-echo-legacy-grant", dir.path().join("local-dev"))
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
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability");
    let context =
        echo_spawn_execution_context("local-dev-echo-legacy-grant", "thread-echo-legacy-grant");
    disable_global_auto_approve(local_runtime, &context).await;

    local_runtime
        .persistent_approval_policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id.clone(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: Some(ironclaw_host_api::ApprovalRequestId::new()),
        })
        .await
        .expect("legacy persistent approval policy");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            ResourceEstimate::default(),
            serde_json::json!({"message": "ask despite legacy grant"}),
        ))
        .await
        .expect("echo invocation resolves");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!("legacy persistent grant must not override global off, got {outcome:?}");
    };
    assert_eq!(gate.capability_id, capability_id);
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "legacy persistent grant with global auto-approve off must create a pending approval"
    );
}

#[tokio::test]
async fn local_dev_settings_page_always_allow_echo_overrides_global_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-echo-settings-allow",
            dir.path().join("local-dev"),
        )
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
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability");
    let context = echo_spawn_execution_context(
        "local-dev-echo-settings-allow",
        "thread-echo-settings-allow",
    );

    local_runtime
        .persistent_approval_policies
        .allow(PersistentApprovalPolicyInput {
            scope: operator_tool_permission_scope_for_test(&context.resource_scope),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id.clone(),
            grantee: Principal::Extension(
                ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("builtin provider"),
            ),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("settings-page persistent echo policy");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"message": "skip approval for echo"}),
        ))
        .await
        .expect("settings persistent echo invocation succeeds");

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "settings-page always_allow policy should override global off for builtin.echo, got {outcome:?}"
    );
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        0,
        "settings-page always_allow builtin.echo must not create a pending approval"
    );
}

#[tokio::test]
async fn local_dev_settings_page_always_allow_policy_skips_next_shell_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-settings-allow-owner",
            dir.path().join("local-dev"),
        )
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
    let context = shell_execution_context(
        "local-dev-settings-allow-owner",
        "thread-local-dev-settings-allow",
    );

    local_runtime
        .persistent_approval_policies
        .allow(PersistentApprovalPolicyInput {
            scope: operator_tool_permission_scope_for_test(&context.resource_scope),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id.clone(),
            grantee: Principal::Extension(
                ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("builtin provider"),
            ),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: shell_allowed_effects(),
                mounts: MountView::default(),
                network: shell_network_policy(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("settings-page persistent approval policy");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo settings allow"}),
        ))
        .await
        .expect("settings persistent approval shell invocation succeeds");

    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "settings-page always_allow policy should skip the shell approval gate, got {outcome:?}"
    );
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        0,
        "persistent always_allow policy must not create a pending approval"
    );
}

#[tokio::test]
async fn local_dev_yolo_explicit_ask_each_time_still_requires_approval_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");
    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-ask-owner",
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
    let context = shell_execution_context("local-dev-yolo-ask-owner", "thread-local-yolo-ask");

    local_runtime
        .tool_permission_overrides
        .set(ToolPermissionOverrideInput {
            scope: operator_tool_permission_scope_for_test(&context.resource_scope),
            capability_id: capability_id.clone(),
            state: ToolPermissionOverride::AskEachTime,
            updated_by: Principal::User(context.user_id.clone()),
        })
        .await
        .expect("tool permission override update");

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo yolo ask"}),
        ))
        .await
        .expect("local-dev-yolo shell invocation resolves");

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!("explicit ask_each_time should override yolo bypass, got {outcome:?}");
    };
    assert_eq!(gate.capability_id, capability_id);
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "explicit ask_each_time must create a pending approval"
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
    disable_global_auto_approve(local_runtime, &context).await;

    let blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
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
    context.run_id = Some(RunId::new());
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
    local_runtime: &RebornRuntimeSubstrate,
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

async fn pending_approval_count(
    local_runtime: &RebornRuntimeSubstrate,
    context: &ExecutionContext,
) -> usize {
    local_runtime
        .approval_requests
        .records_for_scope(&context.resource_scope)
        .await
        .expect("approval store records") // safety: test-only helper reads in-memory approval records from a constructed local runtime.
        .into_iter()
        .filter(|record| record.status == ApprovalStatus::Pending)
        .count()
}

fn operator_tool_permission_scope_for_test(scope: &ResourceScope) -> ResourceScope {
    ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: scope.invocation_id,
    }
}

fn shell_lease_approval() -> LeaseApproval {
    builtin_one_shot_lease_approval(GrantConstraints {
        allowed_effects: shell_allowed_effects(),
        mounts: MountView::default(),
        network: shell_network_policy(),
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    })
}

fn echo_dispatch_lease_approval() -> LeaseApproval {
    builtin_one_shot_lease_approval(GrantConstraints {
        allowed_effects: echo_dispatch_allowed_effects(),
        mounts: MountView::default(),
        network: NetworkPolicy::default(),
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    })
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

/// Minimal approval policy still honors the operator global approval switch.
/// With global auto-approve off, eligible tools ask even when the runtime
/// profile would otherwise bypass approval gates.
#[tokio::test]
async fn local_dev_minimal_policy_shell_invocation_asks_when_global_auto_approve_is_off() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only helper in #[cfg(test)] module.
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-minimal-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_minimal_policy()),
    )
    .await
    .expect("local-dev minimal services build"); // safety: test-only helper in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability"); // safety: test-only helper in #[cfg(test)] module.
    let context = shell_execution_context("local-dev-minimal-owner", "thread-minimal-approval");
    disable_global_auto_approve(local_runtime, &context).await;

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo minimal"}),
        ))
        .await
        .expect("minimal shell invocation resolves"); // safety: test-only helper in #[cfg(test)] module.

    let RuntimeCapabilityOutcome::ApprovalRequired(gate) = outcome else {
        panic!("global auto-approve off should gate minimal shell invocation, got {outcome:?}");
    };
    assert_eq!(gate.capability_id, capability_id); // safety: test-only assertion in #[cfg(test)] module.
    assert_eq!(
        pending_approval_count(local_runtime, &context).await,
        1,
        "minimal policy with global auto-approve off must create a pending approval"
    );
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
    disable_global_auto_approve(local_runtime, &context).await;

    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"command": "echo ent"}),
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
    disable_global_auto_approve(local_runtime, &context).await;

    let blocked = host_runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
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
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let host_runtime = services
        .host_runtime
        .as_ref()
        .expect("local-dev host runtime"); // safety: test-only helper in #[cfg(test)] module.
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability"); // safety: test-only helper in #[cfg(test)] module.
    let context = echo_spawn_execution_context("local-dev-echo-spawn-owner", "thread-echo-spawn");
    disable_global_auto_approve(local_runtime, &context).await;

    let outcome = host_runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id,
            ResourceEstimate::default(),
            serde_json::json!({"message": "spawn echo"}),
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
    context.run_id = Some(RunId::new());
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

fn echo_dispatch_allowed_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability]
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
    disable_global_auto_approve(local_runtime, &context).await;

    // First invocation — expect approval gate.
    let first_blocked = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability_id.clone(),
            estimate.clone(),
            input.clone(),
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
        ))
        .await
        .expect("second invocation"); // safety: test-only helper in #[cfg(test)] module.
    // Spent one-shot lease must not bypass approval on second invocation.
    let regated = matches!(second, RuntimeCapabilityOutcome::ApprovalRequired(_));
    assert!(regated); // safety: test-only assertion in #[cfg(test)] module.
}
