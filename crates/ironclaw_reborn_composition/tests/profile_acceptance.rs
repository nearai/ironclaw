use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornReadiness, RebornReadinessState, local_dev_yolo_runtime_policy,
};

use ironclaw_host_api::runtime_policy::{RuntimeProfile, SecretMode};

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
    let policy = local_dev_yolo_runtime_policy().expect("policy resolves");

    assert_eq!(policy.requested_profile, RuntimeProfile::LocalYolo);
    assert_eq!(policy.resolved_profile, RuntimeProfile::LocalYolo);
    assert_eq!(policy.secret_mode, SecretMode::InheritedEnv);
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
