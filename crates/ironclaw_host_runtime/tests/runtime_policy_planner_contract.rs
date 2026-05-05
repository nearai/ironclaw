//! Vertical slice + enforcement tests for #3045 PR 5/6/7.
//!
//! PR 6 (local profile vertical slice): drive a `LocalSingleUser +
//! LocalDev` policy through the planner and assert the resolver +
//! planner agree on `HostWorkspace` filesystem and `LocalHost`
//! process backend for filesystem/shell capabilities. Coding-alias
//! capabilities (cargo test, npm test, ripgrep) are modelled as
//! capability descriptors with `SpawnProcess + ReadFilesystem`
//! effects.
//!
//! PR 7 (hosted/enterprise enforcement): drive `HostedMultiTenant +
//! HostedDev` and `EnterpriseDedicated + EnterpriseDev` and assert the
//! planner cannot — by construction — produce a plan that targets
//! `LocalHost` shell or `HostWorkspace` filesystem. The resolver's
//! fail-closed contract forbids those backends in those deployments;
//! the planner is downstream and forwards faithfully.
//!
//! These tests are integration-tier: they live in
//! `crates/ironclaw_host_runtime/tests/` so they exercise the
//! planner's *public* API and survive private-module refactors.

use ironclaw_host_api::runtime_policy::{
    DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile,
};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, PermissionMode, RuntimeKind,
    TrustClass,
};
use ironclaw_host_runtime::plan_capability;
use ironclaw_runtime_policy::{OrgPolicyConstraints, ResolveRequest, resolve};

fn descriptor(id: &str, effects: Vec<EffectKind>) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id.to_string()).unwrap(),
        provider: ExtensionId::new("test_extension".to_string()).unwrap(),
        runtime: RuntimeKind::Script,
        trust_ceiling: TrustClass::UserTrusted,
        description: format!("test capability {id}"),
        parameters_schema: serde_json::Value::Null,
        effects,
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
}

// -- PR 6: Local profile vertical slice -------------------------------------

#[test]
fn local_dev_filesystem_read_plans_against_host_workspace() {
    // Acceptance criterion: `LocalSingleUser + LocalDev` reaches a
    // `HostWorkspace` filesystem backend through the planner. This is
    // the canonical "agent edits files in the user's repo" path.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalDev,
    ))
    .unwrap();

    let read = descriptor("filesystem.read", vec![EffectKind::ReadFilesystem]);
    let plan = plan_capability(&read, &policy).unwrap();
    assert_eq!(
        plan.filesystem_backend,
        FilesystemBackendKind::HostWorkspace
    );
    assert_eq!(plan.process_backend, ProcessBackendKind::LocalHost);
}

#[test]
fn local_dev_coding_alias_capabilities_plan_against_local_host_shell() {
    // The coding-aliases (cargo test, npm test, ripgrep, etc.) all
    // declare `SpawnProcess + ReadFilesystem`. Under LocalDev the
    // planner must route them to the LocalHost process backend so the
    // user can run native development tooling.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalDev,
    ))
    .unwrap();

    for alias in [
        "shell.cargo_test",
        "shell.npm_test",
        "shell.ripgrep",
        "shell.git_status",
    ] {
        let cap = descriptor(
            alias,
            vec![EffectKind::SpawnProcess, EffectKind::ReadFilesystem],
        );
        let plan = plan_capability(&cap, &policy).unwrap();
        assert_eq!(
            plan.process_backend,
            ProcessBackendKind::LocalHost,
            "{alias} must plan against LocalHost under LocalDev"
        );
        assert_eq!(
            plan.filesystem_backend,
            FilesystemBackendKind::HostWorkspace,
            "{alias} must plan against HostWorkspace under LocalDev"
        );
    }
}

#[test]
fn local_safe_approval_preset_keeps_writes_supervised() {
    // LocalSafe is the cautious local mode: the resolver maps it to
    // `ApprovalPolicy::AskWrites`. The planner doesn't gate approvals
    // (that's the authorization/approvals layer's job) but the
    // resolver's approval choice must reach the policy that drives
    // downstream approval prompts.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalSafe,
    ))
    .unwrap();
    assert_eq!(
        policy.approval_policy,
        ironclaw_host_api::runtime_policy::ApprovalPolicy::AskWrites
    );
}

// -- PR 7: Hosted enforcement ----------------------------------------------

#[test]
fn hosted_dev_shell_run_never_plans_against_local_host() {
    // Acceptance criterion: hosted profiles cannot resolve to
    // provider-host filesystem or provider-host shell. The resolver's
    // `hosted_family_never_resolves_to_provider_host_filesystem_or_shell`
    // test locks this in at the resolver tier; this assertion locks
    // it in at the planner tier so a downstream regression that *re-
    // injected* a LocalHost backend before plan output would fail
    // here.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::HostedDev,
    ))
    .unwrap();

    let shell = descriptor("shell.run", vec![EffectKind::SpawnProcess]);
    let plan = plan_capability(&shell, &policy).unwrap();
    assert_eq!(plan.process_backend, ProcessBackendKind::TenantSandbox);
    assert_ne!(plan.process_backend, ProcessBackendKind::LocalHost);
    assert_ne!(
        plan.filesystem_backend,
        FilesystemBackendKind::HostWorkspace
    );
}

#[test]
fn hosted_dev_filesystem_write_plans_against_tenant_workspace_only() {
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::HostedDev,
    ))
    .unwrap();

    let write = descriptor(
        "filesystem.write",
        vec![EffectKind::WriteFilesystem, EffectKind::ReadFilesystem],
    );
    let plan = plan_capability(&write, &policy).unwrap();
    assert_eq!(
        plan.filesystem_backend,
        FilesystemBackendKind::TenantWorkspace
    );
}

#[test]
fn hosted_multi_tenant_rejects_local_dev_at_resolver_before_planner_runs() {
    // The planner is only reachable when the resolver succeeded.
    // Acceptance criterion: `HostedMultiTenant + LocalDev` fails closed
    // at the resolver, not silently downgrades to a different profile
    // at plan time.
    let result = resolve(ResolveRequest::new(
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::LocalDev,
    ));
    assert!(
        result.is_err(),
        "HostedMultiTenant + LocalDev must fail closed at the resolver"
    );
}

#[test]
fn hosted_yolo_tenant_scoped_never_plans_against_local_host_or_host_filesystem() {
    // The "yolo means tenant-sandbox yolo, never provider-host yolo"
    // contract: even with the yolo profile selected and disclosure
    // acknowledged, the resolved policy + planner combination must
    // never reach LocalHost shell or HostWorkspace filesystem.
    let req = ResolveRequest {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedYoloTenantScoped,
        org_policy: OrgPolicyConstraints::default(),
        yolo_disclosure_acknowledged: true,
    };
    let policy = resolve(req).unwrap();

    let shell = descriptor("shell.run", vec![EffectKind::SpawnProcess]);
    let plan = plan_capability(&shell, &policy).unwrap();
    assert_eq!(plan.process_backend, ProcessBackendKind::TenantSandbox);
    assert_ne!(plan.process_backend, ProcessBackendKind::LocalHost);

    let read = descriptor("filesystem.read", vec![EffectKind::ReadFilesystem]);
    let plan = plan_capability(&read, &policy).unwrap();
    assert_ne!(
        plan.filesystem_backend,
        FilesystemBackendKind::HostWorkspace
    );
}

// -- PR 7: Enterprise enforcement ------------------------------------------

#[test]
fn enterprise_dev_process_run_plans_against_org_dedicated_runner() {
    // Acceptance criterion: enterprise direct-runner modes require
    // `EnterpriseDedicated`. The resolver maps EnterpriseDev to
    // `OrgDedicatedRunner`; the planner forwards that to the plan.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::EnterpriseDedicated,
        RuntimeProfile::EnterpriseDev,
    ))
    .unwrap();

    let cap = descriptor("process.run", vec![EffectKind::SpawnProcess]);
    let plan = plan_capability(&cap, &policy).unwrap();
    assert_eq!(plan.process_backend, ProcessBackendKind::OrgDedicatedRunner);
    assert_eq!(
        plan.filesystem_backend,
        FilesystemBackendKind::OrgDedicatedWorkspace
    );
}

#[test]
fn enterprise_yolo_dedicated_requires_admin_approval_at_resolver() {
    // Acceptance criterion: enterprise direct-runner yolo modes require
    // both `EnterpriseDedicated` deployment and explicit org admin
    // policy approval. Without `admin_approves_dedicated_yolo`, the
    // resolver fails closed and the planner is never reached.
    let req_no_admin = ResolveRequest {
        deployment: DeploymentMode::EnterpriseDedicated,
        requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
        org_policy: OrgPolicyConstraints::default(),
        yolo_disclosure_acknowledged: true,
    };
    assert!(resolve(req_no_admin).is_err());

    let req_with_admin = ResolveRequest {
        deployment: DeploymentMode::EnterpriseDedicated,
        requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
        org_policy: OrgPolicyConstraints {
            admin_approves_dedicated_yolo: true,
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: true,
    };
    assert!(resolve(req_with_admin).is_ok());
}

#[test]
fn experiment_profile_picks_disposable_smolvm_process_backend() {
    // Acceptance criterion (issue example): `Experiment + package
    // install -> disposable SmolVM/Docker workspace`. The resolver
    // picks SmolVm; the planner forwards.
    let policy = resolve(ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::Experiment,
    ))
    .unwrap();
    assert_eq!(policy.process_backend, ProcessBackendKind::SmolVm);
    assert_eq!(policy.network_mode, NetworkMode::Allowlist);
}
