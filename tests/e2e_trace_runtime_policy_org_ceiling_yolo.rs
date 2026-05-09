//! Resolver invariant tests: org-policy ceiling × yolo narrowing.
//!
//! Complements the in-crate resolver tests in
//! `crates/ironclaw_runtime_policy/src/resolver.rs:435-917` by promoting
//! a few high-stakes properties to the integration tier with **typed-error**
//! pattern matches (not `is_err()`) and explicit `was_reduced()` assertions
//! on the narrowed shape.
//!
//! All tests are pure resolver-property — no agent loop, no DB.

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    RuntimeProfile,
};
use ironclaw_runtime_policy::{OrgPolicyConstraints, ResolveError, ResolveRequest, resolve};

#[test]
fn local_yolo_with_local_safe_ceiling_narrows_to_local_safe_with_ask_writes_and_was_reduced_true() {
    let policy = resolve(ResolveRequest {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalYolo,
        org_policy: OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::LocalSafe),
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: true,
    })
    .expect("LocalYolo + LocalSafe ceiling resolves with disclosure");

    assert_eq!(policy.requested_profile, RuntimeProfile::LocalYolo);
    assert_eq!(policy.resolved_profile, RuntimeProfile::LocalSafe);
    assert_eq!(policy.approval_policy, ApprovalPolicy::AskWrites);
    assert_eq!(policy.process_backend, ProcessBackendKind::LocalHost);
    assert_eq!(
        policy.filesystem_backend,
        FilesystemBackendKind::HostWorkspace
    );
    assert!(
        policy.was_reduced(),
        "ceiling narrowing must report was_reduced() == true",
    );
}

#[test]
fn local_yolo_with_local_dev_ceiling_narrows_to_local_dev_with_ask_destructive() {
    let policy = resolve(ResolveRequest {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalYolo,
        org_policy: OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::LocalDev),
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: true,
    })
    .expect("LocalYolo + LocalDev ceiling resolves with disclosure");

    assert_eq!(policy.resolved_profile, RuntimeProfile::LocalDev);
    assert_eq!(policy.approval_policy, ApprovalPolicy::AskDestructive);
    assert!(policy.was_reduced());
}

#[test]
fn wider_within_family_ceiling_does_not_widen_resolved_profile() {
    // LocalDev request + LocalYolo ceiling: ceiling is wider than the
    // request, so it must not widen authority. Resolved == requested,
    // was_reduced() == false. Belt-and-suspenders against an inverted
    // narrowing check.
    let policy = resolve(ResolveRequest {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        org_policy: OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::LocalYolo),
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: false,
    })
    .expect("LocalDev with wider LocalYolo ceiling resolves");

    assert_eq!(policy.requested_profile, RuntimeProfile::LocalDev);
    assert_eq!(policy.resolved_profile, RuntimeProfile::LocalDev);
    assert!(
        !policy.was_reduced(),
        "wider ceiling must not narrow the resolved profile",
    );
}

#[test]
fn enterprise_yolo_dedicated_without_admin_approval_fails_closed_with_typed_error() {
    let err = resolve(ResolveRequest {
        deployment: DeploymentMode::EnterpriseDedicated,
        requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
        org_policy: OrgPolicyConstraints::default(),
        yolo_disclosure_acknowledged: true,
    })
    .expect_err("EnterpriseYoloDedicated without admin approval must fail");

    assert!(
        matches!(err, ResolveError::DedicatedYoloRequiresOrgAdminApproval),
        "expected DedicatedYoloRequiresOrgAdminApproval, got {err:?}",
    );
}

#[test]
fn enterprise_yolo_dedicated_with_admin_approval_and_disclosure_keeps_direct_logged_and_org_policy_approvals()
 {
    let policy = resolve(ResolveRequest {
        deployment: DeploymentMode::EnterpriseDedicated,
        requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
        org_policy: OrgPolicyConstraints {
            admin_approves_dedicated_yolo: true,
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: true,
    })
    .expect("EnterpriseYoloDedicated with admin approval + disclosure resolves");

    // Backend and approvals locked: this profile widens network to
    // DirectLogged (NOT Direct) and approvals to OrgPolicy (NOT Minimal —
    // the documented variance from other yolo profiles).
    assert_eq!(policy.network_mode, NetworkMode::DirectLogged);
    assert_eq!(policy.approval_policy, ApprovalPolicy::OrgPolicy);
    assert_eq!(
        policy.process_backend,
        ProcessBackendKind::OrgDedicatedRunner
    );
    assert_eq!(
        policy.filesystem_backend,
        FilesystemBackendKind::OrgDedicatedWorkspace
    );
}

#[test]
fn cross_family_ceiling_rejects_with_typed_error_variant() {
    // LocalDev request + HostedSafe ceiling: ceiling lives in a different
    // family — settings/blueprint layer should have caught this; resolver
    // surfaces the typed family-mismatch error as a fail-closed safety net.
    let err = resolve(ResolveRequest {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        org_policy: OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::HostedSafe),
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: false,
    })
    .expect_err("cross-family ceiling must fail closed");

    match err {
        ResolveError::OrgPolicyCeilingFamilyMismatch { requested, ceiling } => {
            assert_eq!(requested, RuntimeProfile::LocalDev);
            assert_eq!(ceiling, RuntimeProfile::HostedSafe);
        }
        other => panic!("expected OrgPolicyCeilingFamilyMismatch, got {other:?}"),
    }
}

#[test]
fn every_yolo_profile_without_disclosure_fails_closed_with_typed_error() {
    let cases = [
        (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalYolo),
        (
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedYoloTenantScoped,
        ),
        (
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseYoloDedicated,
        ),
    ];

    for (deployment, profile) in cases {
        let err = resolve(ResolveRequest {
            deployment,
            requested_profile: profile,
            org_policy: OrgPolicyConstraints {
                // Provide admin approval for the enterprise case so the
                // disclosure check is the *only* failing gate.
                admin_approves_dedicated_yolo: true,
                ..OrgPolicyConstraints::default()
            },
            yolo_disclosure_acknowledged: false,
        })
        .expect_err("yolo without disclosure must fail closed");

        match err {
            ResolveError::YoloRequiresDisclosure { profile: failed } => {
                assert_eq!(
                    failed, profile,
                    "error must carry the requested profile: deployment={deployment:?}",
                );
            }
            other => panic!(
                "expected YoloRequiresDisclosure for {profile:?} under {deployment:?}, got {other:?}",
            ),
        }
    }
}

#[test]
fn local_profile_under_hosted_deployment_fails_closed_with_typed_error() {
    let err = resolve(ResolveRequest {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::LocalDev,
        org_policy: OrgPolicyConstraints::default(),
        yolo_disclosure_acknowledged: false,
    })
    .expect_err("LocalDev under HostedMultiTenant must fail closed");

    match err {
        ResolveError::IncompatibleDeployment {
            deployment,
            profile,
        } => {
            assert_eq!(deployment, DeploymentMode::HostedMultiTenant);
            assert_eq!(profile, RuntimeProfile::LocalDev);
        }
        other => panic!("expected IncompatibleDeployment, got {other:?}"),
    }
}
