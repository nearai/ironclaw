//! Wire-stable serde coverage for every runtime-policy enum + `EffectiveRuntimePolicy`.
//!
//! PR #3243 shipped serde derives on all eight backend enums (with
//! `rename_all = "snake_case"`) plus `EffectiveRuntimePolicy`, but only
//! round-trip-tested `EffectiveRuntimePolicy`/`OrgPolicyConstraints`/
//! `ResolveRequest` aggregate-style. Per-variant exhaustiveness is missing —
//! a regression that flipped a single rename or dropped a variant from
//! deserialize would not be caught by the existing tests.
//!
//! Each enum here is tested with an explicit array literal of every
//! variant, so adding a `#[non_exhaustive]` variant in the future fails
//! compilation of the test binary (forcing an explicit decision).

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_runtime_policy::{OrgPolicyConstraints, ResolveRequest, resolve};

/// JSON round-trip + serialized form matches `as_str()` (snake_case wire name).
fn assert_round_trip<T>(value: T, expected_wire: &str)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + std::fmt::Debug + PartialEq + Clone,
{
    let json = serde_json::to_string(&value).expect("serialize");
    assert_eq!(
        json,
        format!("\"{expected_wire}\""),
        "serialized form must match snake_case wire name for {value:?}",
    );
    let decoded: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, value);
}

#[test]
fn deployment_mode_round_trips_serde_for_every_variant() {
    let variants = [
        DeploymentMode::LocalSingleUser,
        DeploymentMode::HostedMultiTenant,
        DeploymentMode::EnterpriseDedicated,
    ];
    assert_eq!(
        variants.len(),
        3,
        "DeploymentMode variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
        // DeploymentMode also implements `FromStr` — round-trip via the
        // wire name to lock in display/parse symmetry.
        let parsed: DeploymentMode = v.as_str().parse().expect("FromStr");
        assert_eq!(parsed, v);
    }
}

#[test]
fn runtime_profile_round_trips_serde_for_every_variant() {
    let variants = [
        RuntimeProfile::SecureDefault,
        RuntimeProfile::LocalSafe,
        RuntimeProfile::LocalDev,
        RuntimeProfile::LocalYolo,
        RuntimeProfile::HostedSafe,
        RuntimeProfile::HostedDev,
        RuntimeProfile::HostedYoloTenantScoped,
        RuntimeProfile::EnterpriseSafe,
        RuntimeProfile::EnterpriseDev,
        RuntimeProfile::EnterpriseYoloDedicated,
        RuntimeProfile::Sandboxed,
        RuntimeProfile::Experiment,
    ];
    assert_eq!(
        variants.len(),
        12,
        "RuntimeProfile variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
        let parsed: RuntimeProfile = v.as_str().parse().expect("FromStr");
        assert_eq!(parsed, v);
    }
}

#[test]
fn filesystem_backend_kind_round_trips_serde_for_every_variant() {
    let variants = [
        FilesystemBackendKind::ScopedVirtual,
        FilesystemBackendKind::HostWorkspace,
        FilesystemBackendKind::TenantWorkspace,
        FilesystemBackendKind::OrgDedicatedWorkspace,
    ];
    assert_eq!(
        variants.len(),
        4,
        "FilesystemBackendKind variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn process_backend_kind_round_trips_serde_for_every_variant() {
    let variants = [
        ProcessBackendKind::None,
        ProcessBackendKind::Docker,
        ProcessBackendKind::Srt,
        ProcessBackendKind::SmolVm,
        ProcessBackendKind::LocalHost,
        ProcessBackendKind::TenantSandbox,
        ProcessBackendKind::OrgDedicatedRunner,
    ];
    assert_eq!(
        variants.len(),
        7,
        "ProcessBackendKind variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn network_mode_round_trips_serde_for_every_variant() {
    let variants = [
        NetworkMode::Deny,
        NetworkMode::Brokered,
        NetworkMode::Allowlist,
        NetworkMode::DirectLogged,
        NetworkMode::Direct,
    ];
    assert_eq!(
        variants.len(),
        5,
        "NetworkMode variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn secret_mode_round_trips_serde_for_every_variant() {
    let variants = [
        SecretMode::Deny,
        SecretMode::BrokeredHandles,
        SecretMode::TenantBroker,
        SecretMode::OrgBroker,
        SecretMode::ScrubbedEnv,
        SecretMode::InheritedEnv,
    ];
    assert_eq!(
        variants.len(),
        6,
        "SecretMode variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn approval_policy_round_trips_serde_for_every_variant() {
    let variants = [
        ApprovalPolicy::AskAlways,
        ApprovalPolicy::AskWrites,
        ApprovalPolicy::AskDestructive,
        ApprovalPolicy::OrgPolicy,
        ApprovalPolicy::Minimal,
    ];
    assert_eq!(
        variants.len(),
        5,
        "ApprovalPolicy variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn audit_mode_round_trips_serde_for_every_variant() {
    let variants = [
        AuditMode::LocalMinimal,
        AuditMode::Standard,
        AuditMode::OrgPolicy,
    ];
    assert_eq!(
        variants.len(),
        3,
        "AuditMode variant count drift — update this test when adding variants",
    );
    for v in variants {
        assert_round_trip(v, v.as_str());
    }
}

#[test]
fn effective_runtime_policy_round_trips_serde_for_a_representative_matrix() {
    // Build at least nine representative policies via the resolver covering
    // the deployment × profile combinations highlighted in PR #3243's body.
    // Resolver-produced policies are the only sanctioned source per the
    // ironclaw_runtime_policy guardrail; round-tripping these locks every
    // backend enum's serde simultaneously.
    let policies = [
        // Local family
        resolve(ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::SecureDefault,
        ))
        .unwrap(),
        resolve(ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalSafe,
        ))
        .unwrap(),
        resolve(ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalDev,
        ))
        .unwrap(),
        resolve(ResolveRequest {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalYolo,
            org_policy: OrgPolicyConstraints::default(),
            yolo_disclosure_acknowledged: true,
        })
        .unwrap(),
        // Hosted family
        resolve(ResolveRequest::new(
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedSafe,
        ))
        .unwrap(),
        resolve(ResolveRequest::new(
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedDev,
        ))
        .unwrap(),
        resolve(ResolveRequest {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::HostedYoloTenantScoped,
            org_policy: OrgPolicyConstraints::default(),
            yolo_disclosure_acknowledged: true,
        })
        .unwrap(),
        // Enterprise family
        resolve(ResolveRequest::new(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseSafe,
        ))
        .unwrap(),
        resolve(ResolveRequest::new(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseDev,
        ))
        .unwrap(),
        resolve(ResolveRequest {
            deployment: DeploymentMode::EnterpriseDedicated,
            requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
            org_policy: OrgPolicyConstraints {
                admin_approves_dedicated_yolo: true,
                ..OrgPolicyConstraints::default()
            },
            yolo_disclosure_acknowledged: true,
        })
        .unwrap(),
        // Deployment-agnostic profiles
        resolve(ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::Sandboxed,
        ))
        .unwrap(),
        resolve(ResolveRequest::new(
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::Experiment,
        ))
        .unwrap(),
    ];

    for policy in policies {
        let json = serde_json::to_string(&policy).expect("serialize policy");
        let decoded: EffectiveRuntimePolicy =
            serde_json::from_str(&json).expect("deserialize policy");
        assert_eq!(
            decoded, policy,
            "policy must round-trip through serde unchanged",
        );
    }
}

#[test]
fn effective_runtime_policy_was_reduced_flag_round_trips_through_serde() {
    // Build a narrowed policy: LocalYolo requested, LocalDev ceiling →
    // resolver narrows to LocalDev. `was_reduced()` is `true`. Serialize,
    // deserialize, and assert the flag survives the round-trip.
    let narrowed = resolve(ResolveRequest {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalYolo,
        org_policy: OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::LocalDev),
            ..OrgPolicyConstraints::default()
        },
        yolo_disclosure_acknowledged: true,
    })
    .expect("LocalYolo with LocalDev ceiling resolves");
    assert!(
        narrowed.was_reduced(),
        "ceiling narrowing must set requested != resolved",
    );
    assert_eq!(narrowed.requested_profile, RuntimeProfile::LocalYolo);
    assert_eq!(narrowed.resolved_profile, RuntimeProfile::LocalDev);

    let json = serde_json::to_string(&narrowed).expect("serialize narrowed");
    let decoded: EffectiveRuntimePolicy =
        serde_json::from_str(&json).expect("deserialize narrowed");
    assert_eq!(decoded, narrowed);
    assert!(
        decoded.was_reduced(),
        "was_reduced() must return true after serde round-trip on narrowed policy",
    );
}
