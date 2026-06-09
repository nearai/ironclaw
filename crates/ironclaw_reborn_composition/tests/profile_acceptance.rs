use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness, RebornReadiness,
    RebornReadinessDiagnostic, RebornReadinessDiagnosticComponent, RebornReadinessDiagnosticReason,
    RebornReadinessDiagnosticStatus, RebornReadinessState, RebornWorkerReadiness,
    build_reborn_services, local_dev_yolo_runtime_policy,
};

use ironclaw_host_api::runtime_policy::{FilesystemBackendKind, RuntimeProfile, SecretMode};
use ironclaw_runtime_policy::ResolveError;
use serde_json::json;

#[test]
fn profile_parse_accepts_kebab_and_snake_case() {
    assert_eq!(
        "disabled".parse::<RebornCompositionProfile>().unwrap(),
        RebornCompositionProfile::Disabled
    );
    assert_eq!(
        "local_dev".parse::<RebornCompositionProfile>().unwrap(),
        RebornCompositionProfile::LocalDev
    );
    assert_eq!(
        "local_dev_yolo"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::LocalDevYolo
    );
    assert_eq!(
        "local-dev-yolo"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::LocalDevYolo
    );
    assert_eq!(
        "migration-dry-run"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::MigrationDryRun
    );
}

#[test]
fn full_graph_profiles_match_production_strictness() {
    assert!(!RebornCompositionProfile::Disabled.requires_production_shape());
    assert!(!RebornCompositionProfile::LocalDev.requires_production_shape());
    assert!(!RebornCompositionProfile::LocalDevYolo.requires_production_shape());
    assert!(RebornCompositionProfile::Production.requires_production_shape());
    assert!(RebornCompositionProfile::MigrationDryRun.requires_production_shape());
}

#[test]
fn local_dev_yolo_runtime_policy_inherits_host_environment() {
    let policy = local_dev_yolo_runtime_policy(true).expect("policy resolves");

    assert_eq!(policy.requested_profile, RuntimeProfile::LocalYolo);
    assert_eq!(policy.resolved_profile, RuntimeProfile::LocalYolo);
    assert_eq!(
        policy.filesystem_backend,
        FilesystemBackendKind::HostWorkspaceAndHome
    );
    assert_eq!(policy.secret_mode, SecretMode::InheritedEnv);
}

#[test]
fn local_dev_yolo_runtime_policy_requires_disclosure() {
    let error = local_dev_yolo_runtime_policy(false).expect_err("yolo requires confirmation");

    assert_eq!(
        error,
        ResolveError::YoloRequiresDisclosure {
            profile: RuntimeProfile::LocalYolo
        }
    );
}

#[test]
fn disabled_readiness_is_redaction_safe() {
    let json = serde_json::to_string(&RebornReadiness::disabled()).unwrap();
    assert!(json.contains("disabled"));
    assert!(!json.contains("postgres://"));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("secret"));
    assert_eq!(
        RebornReadiness::disabled().state,
        RebornReadinessState::Disabled
    );
}

#[test]
fn readiness_serializes_diagnostics_with_stable_redacted_vocabulary() {
    let readiness = readiness_for_contract(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        vec![RebornReadinessDiagnostic::production_blocker(
            RebornCompositionProfile::Production,
            RebornReadinessDiagnosticComponent::RuntimeHttpEgress,
            RebornReadinessDiagnosticReason::Unverified,
        )],
    );

    let value = serde_json::to_value(readiness).unwrap();

    assert_eq!(
        value,
        json!({
            "profile": "production",
            "state": "production-validated",
            "facades": {
                "host_runtime": true,
                "turn_coordinator": true,
                "product_auth": true
            },
            "workers": {
                "turn_runner": false,
                "trigger_poller": false
            },
            "diagnostics": [{
                "profile": "production",
                "component": "runtime_http_egress",
                "reason": "unverified",
                "status": "blocking",
                "blocks_production": true
            }]
        })
    );
}

#[test]
fn readiness_deserializes_legacy_payload_without_diagnostics() {
    let readiness: RebornReadiness = serde_json::from_value(json!({
        "profile": "production",
        "state": "production-validated",
        "facades": {
            "host_runtime": true,
            "turn_coordinator": true,
            "product_auth": false
        },
        "workers": {
            "turn_runner": false,
            "trigger_poller": false
        }
    }))
    .unwrap();

    assert!(readiness.diagnostics.is_empty());
    assert_eq!(readiness.state, RebornReadinessState::ProductionValidated);
}

#[test]
fn dev_only_profiles_are_visible_non_production_in_readiness() {
    for (profile, diagnostic) in [
        (
            RebornCompositionProfile::LocalDev,
            RebornReadinessDiagnostic::local_dev(),
        ),
        (
            RebornCompositionProfile::LocalDevYolo,
            RebornReadinessDiagnostic::local_dev_yolo(),
        ),
    ] {
        assert_eq!(diagnostic.profile, profile);
        assert_eq!(
            diagnostic.component,
            RebornReadinessDiagnosticComponent::CompositionProfile
        );
        assert_eq!(
            diagnostic.reason,
            RebornReadinessDiagnosticReason::DevOnlyProfile
        );
        assert_eq!(diagnostic.status, RebornReadinessDiagnosticStatus::Warning);
        assert!(diagnostic.blocks_production);
    }
}

#[tokio::test]
async fn local_dev_factory_readiness_includes_non_production_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "readiness-contract-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();

    assert_eq!(
        services.readiness.profile,
        RebornCompositionProfile::LocalDev
    );
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    assert_eq!(
        services.readiness.diagnostics,
        vec![RebornReadinessDiagnostic::local_dev()]
    );
}

#[test]
fn readiness_diagnostics_do_not_carry_sensitive_detail_fields() {
    let readiness = readiness_for_contract(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        vec![
            RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::SecretStore,
                RebornReadinessDiagnosticReason::Missing,
            ),
            RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::ApprovalRequests,
                RebornReadinessDiagnosticReason::LocalOnly,
            ),
            RebornReadinessDiagnostic::production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::RuntimeBackend,
                RebornReadinessDiagnosticReason::Unsupported,
            ),
        ],
    );
    let json = serde_json::to_string(&readiness).unwrap();

    assert!(!json.contains("postgres://user:password@db.example"));
    assert!(!json.contains("sslmode"));
    assert!(!json.contains("/root/workspace"));
    assert!(!json.contains("crate::"));
    assert!(!json.contains("ironclaw_host_runtime::"));
    assert!(!json.contains("approval_id"));
    assert!(!json.contains("lease_id"));
}

fn readiness_for_contract(
    profile: RebornCompositionProfile,
    state: RebornReadinessState,
    diagnostics: Vec<RebornReadinessDiagnostic>,
) -> RebornReadiness {
    RebornReadiness {
        profile,
        state,
        facades: RebornFacadeReadiness {
            host_runtime: true,
            turn_coordinator: true,
            product_auth: true,
        },
        workers: RebornWorkerReadiness {
            turn_runner: false,
            trigger_poller: false,
        },
        diagnostics,
    }
}
