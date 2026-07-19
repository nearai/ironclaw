// Only the `#[cfg(not(feature = "libsql"))]` no-libsql regression test below
// consumes this error type; `webui-v2-beta` (hence CI) pulls in `libsql`.
#[cfg(not(feature = "libsql"))]
use ironclaw_reborn_composition::RebornRuntimeProfileError;
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness, RebornReadiness,
    RebornReadinessDiagnostic, RebornReadinessDiagnosticComponent, RebornReadinessDiagnosticReason,
    RebornReadinessDiagnosticStatus, RebornReadinessState, RebornRuntimeProfileOptions,
    RebornWorkerReadiness, build_reborn_services, hosted_single_tenant_volume_runtime_policy,
    local_dev_yolo_runtime_policy, local_runtime_build_input_with_options,
};

use ironclaw_host_api::runtime_policy::{FilesystemBackendKind, RuntimeProfile, SecretMode};
use ironclaw_host_runtime::{
    ProductionWiringComponent, ProductionWiringIssue, ProductionWiringIssueKind,
    ProductionWiringReport,
};
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
        "hosted_single_tenant"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::HostedSingleTenant
    );
    assert_eq!(
        "hosted-single-tenant"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::HostedSingleTenant
    );
    assert_eq!(
        "hosted_single_tenant_volume"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        "hosted-single-tenant-volume"
            .parse::<RebornCompositionProfile>()
            .unwrap(),
        RebornCompositionProfile::HostedSingleTenantVolume
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
    assert!(!RebornCompositionProfile::HostedSingleTenant.requires_production_shape());
    assert!(!RebornCompositionProfile::HostedSingleTenantVolume.requires_production_shape());
    assert!(RebornCompositionProfile::Production.requires_production_shape());
    assert!(RebornCompositionProfile::MigrationDryRun.requires_production_shape());
}

#[test]
fn profile_predicates_capture_hosted_volume_substrate_contract() {
    assert!(!RebornCompositionProfile::Disabled.uses_local_runtime_substrate());
    assert!(RebornCompositionProfile::LocalDev.uses_local_runtime_substrate());
    assert!(RebornCompositionProfile::LocalDevYolo.uses_local_runtime_substrate());
    assert!(RebornCompositionProfile::HostedSingleTenant.uses_local_runtime_substrate());
    assert!(RebornCompositionProfile::HostedSingleTenantVolume.uses_local_runtime_substrate());
    assert!(!RebornCompositionProfile::Production.uses_local_runtime_substrate());
    assert!(!RebornCompositionProfile::MigrationDryRun.uses_local_runtime_substrate());

    assert!(!RebornCompositionProfile::Disabled.uses_local_dev_storage_input());
    assert!(RebornCompositionProfile::LocalDev.uses_local_dev_storage_input());
    assert!(RebornCompositionProfile::LocalDevYolo.uses_local_dev_storage_input());
    assert!(!RebornCompositionProfile::HostedSingleTenant.uses_local_dev_storage_input());
    assert!(RebornCompositionProfile::HostedSingleTenantVolume.uses_local_dev_storage_input());
    assert!(!RebornCompositionProfile::Production.uses_local_dev_storage_input());
    assert!(!RebornCompositionProfile::MigrationDryRun.uses_local_dev_storage_input());

    assert!(!RebornCompositionProfile::Disabled.uses_hosted_extension_installation_state());
    assert!(!RebornCompositionProfile::LocalDev.uses_hosted_extension_installation_state());
    assert!(!RebornCompositionProfile::LocalDevYolo.uses_hosted_extension_installation_state());
    assert!(
        RebornCompositionProfile::HostedSingleTenant.uses_hosted_extension_installation_state()
    );
    assert!(
        RebornCompositionProfile::HostedSingleTenantVolume
            .uses_hosted_extension_installation_state()
    );
    assert!(!RebornCompositionProfile::Production.uses_hosted_extension_installation_state());
    assert!(!RebornCompositionProfile::MigrationDryRun.uses_hosted_extension_installation_state());

    assert!(!RebornCompositionProfile::Disabled.starts_live_runtime());
    assert!(RebornCompositionProfile::LocalDev.starts_live_runtime());
    assert!(RebornCompositionProfile::LocalDevYolo.starts_live_runtime());
    assert!(RebornCompositionProfile::HostedSingleTenant.starts_live_runtime());
    assert!(RebornCompositionProfile::HostedSingleTenantVolume.starts_live_runtime());
    assert!(RebornCompositionProfile::Production.starts_live_runtime());
    assert!(!RebornCompositionProfile::MigrationDryRun.starts_live_runtime());
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
fn hosted_single_tenant_volume_runtime_policy_is_processless_secure_default() {
    let policy =
        hosted_single_tenant_volume_runtime_policy().expect("hosted volume policy resolves");

    assert_eq!(policy.process_backend.as_str(), "none");
    assert_eq!(policy.filesystem_backend.as_str(), "scoped_virtual");
    assert_eq!(policy.secret_mode.as_str(), "brokered_handles");
    assert_eq!(policy.network_mode.as_str(), "brokered");
}

#[test]
fn disabled_readiness_is_redaction_safe() {
    let readiness = RebornReadiness::disabled();
    let json = serde_json::to_string(&readiness).unwrap();
    assert!(json.contains("disabled"));
    assert!(!json.contains("postgres://"));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("secret"));
    assert_eq!(readiness.state, RebornReadinessState::Disabled);
    assert_eq!(readiness.diagnostics.len(), 1);
    assert_eq!(
        readiness.diagnostics[0].reason,
        RebornReadinessDiagnosticReason::Disabled
    );
    assert_eq!(
        readiness.diagnostics[0].status,
        RebornReadinessDiagnosticStatus::Blocking
    );
    assert!(readiness.diagnostics[0].blocks_production);
}

#[test]
fn readiness_serializes_diagnostics_with_stable_redacted_vocabulary() {
    let readiness = readiness_for_contract(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        vec![production_blocker(
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
fn hosted_single_tenant_readiness_serializes_as_ready_single_tenant_profile() {
    let readiness = readiness_for_contract(
        RebornCompositionProfile::HostedSingleTenant,
        RebornReadinessState::HostedSingleTenantValidated,
        vec![RebornReadinessDiagnostic::hosted_single_tenant()],
    );

    let value = serde_json::to_value(readiness).unwrap();

    assert_eq!(
        value,
        json!({
            "profile": "hosted-single-tenant",
            "state": "hosted-single-tenant-validated",
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
                "profile": "hosted-single-tenant",
                "component": "composition_profile",
                "reason": "unverified",
                "status": "info",
                "blocks_production": false
            }]
        })
    );
}

#[test]
fn readiness_deserializes_diagnostics_payload_into_typed_enums() {
    let readiness: RebornReadiness = serde_json::from_value(json!({
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
    }))
    .unwrap();

    assert_eq!(
        readiness.diagnostics,
        vec![production_blocker(
            RebornCompositionProfile::Production,
            RebornReadinessDiagnosticComponent::RuntimeHttpEgress,
            RebornReadinessDiagnosticReason::Unverified,
        )]
    );
}

#[test]
fn readiness_diagnostic_unknown_wire_variants_deserialize_safely() {
    let diagnostic: RebornReadinessDiagnostic = serde_json::from_value(json!({
        "profile": "production",
        "component": "new_future_component",
        "reason": "new-future-reason",
        "status": "new-future-status",
        "blocks_production": true
    }))
    .unwrap();

    assert_eq!(diagnostic.profile, RebornCompositionProfile::Production);
    assert_eq!(
        diagnostic.component,
        RebornReadinessDiagnosticComponent::Unknown("new_future_component".to_owned())
    );
    assert_eq!(
        diagnostic.reason,
        RebornReadinessDiagnosticReason::Unknown("new-future-reason".to_owned())
    );
    assert_eq!(
        diagnostic.status,
        RebornReadinessDiagnosticStatus::Unknown("new-future-status".to_owned())
    );
    assert!(diagnostic.blocks_production);
}

#[test]
fn readiness_diagnostic_unknown_wire_variants_round_trip_losslessly() {
    let diagnostic: RebornReadinessDiagnostic = serde_json::from_value(json!({
        "profile": "production",
        "component": "runtime_future_proxy",
        "reason": "future-production-reason",
        "status": "future-status",
        "blocks_production": true
    }))
    .unwrap();

    let encoded = serde_json::to_value(diagnostic).unwrap();

    assert_eq!(
        encoded,
        json!({
            "profile": "production",
            "component": "runtime_future_proxy",
            "reason": "future-production-reason",
            "status": "future-status",
            "blocks_production": true
        })
    );
}

#[test]
fn readiness_diagnostic_round_trips_through_serde() {
    let diagnostic = RebornReadinessDiagnostic::production_blocker(
        RebornCompositionProfile::MigrationDryRun,
        RebornReadinessDiagnosticComponent::RuntimeProcessPort,
        RebornReadinessDiagnosticReason::Unsupported,
    )
    .expect("migration-dry-run is production-shaped");
    let encoded = serde_json::to_string(&diagnostic).unwrap();
    let decoded: RebornReadinessDiagnostic = serde_json::from_str(&encoded).unwrap();

    assert_eq!(diagnostic, decoded);
}

#[test]
fn production_blocker_rejects_non_production_shaped_profiles() {
    for profile in [
        RebornCompositionProfile::Disabled,
        RebornCompositionProfile::LocalDev,
        RebornCompositionProfile::LocalDevYolo,
        RebornCompositionProfile::HostedSingleTenant,
        RebornCompositionProfile::HostedSingleTenantVolume,
    ] {
        let diagnostic = RebornReadinessDiagnostic::production_blocker(
            profile,
            RebornReadinessDiagnosticComponent::RuntimeBackend,
            RebornReadinessDiagnosticReason::Missing,
        );

        assert_eq!(diagnostic, None, "profile: {profile:?}");
    }
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
        assert_eq!(diagnostic.status, RebornReadinessDiagnosticStatus::Blocking);
        assert!(diagnostic.blocks_production);
    }
}

#[test]
fn hosted_single_tenant_volume_is_visible_as_preview_readiness() {
    let diagnostic = RebornReadinessDiagnostic::hosted_single_tenant_volume();

    assert_eq!(
        diagnostic.profile,
        RebornCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        diagnostic.component,
        RebornReadinessDiagnosticComponent::CompositionProfile
    );
    assert_eq!(
        diagnostic.reason,
        RebornReadinessDiagnosticReason::HostedSingleTenantVolumePreview
    );
    assert_eq!(diagnostic.status, RebornReadinessDiagnosticStatus::Warning);
    assert!(diagnostic.blocks_production);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn hosted_single_tenant_volume_factory_readiness_includes_preview_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::HostedSingleTenantVolume,
        "readiness-contract-owner",
        dir.path().to_path_buf(),
        Default::default(),
    )
    .unwrap();
    let services = build_reborn_services(input).await.unwrap();

    assert_eq!(
        services.readiness.profile,
        RebornCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        services.readiness.state,
        RebornReadinessState::HostedSingleTenantVolumePreviewValidated
    );
    assert_eq!(
        services.readiness.diagnostics,
        vec![RebornReadinessDiagnostic::hosted_single_tenant_volume()]
    );
}

#[cfg(not(feature = "libsql"))]
#[test]
fn hosted_single_tenant_volume_input_errors_without_libsql_feature() {
    let dir = tempfile::tempdir().unwrap();
    // `RebornBuildInput` (the Ok type) is not `Debug`, so `unwrap_err()` won't
    // compile; match on the `Result` directly instead.
    let result = local_runtime_build_input_with_options(
        RebornCompositionProfile::HostedSingleTenantVolume,
        "readiness-contract-owner",
        dir.path().to_path_buf(),
        Default::default(),
    );

    assert!(matches!(
        result,
        Err(RebornRuntimeProfileError::MissingLibsqlFeature)
    ));
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

#[tokio::test]
async fn local_dev_yolo_factory_readiness_includes_non_production_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        "readiness-yolo-owner",
        dir.path().to_path_buf(),
        RebornRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .unwrap()
    .with_local_dev_confirmed_host_home_root(dir.path().to_path_buf());
    let services = build_reborn_services(input).await.unwrap();

    assert_eq!(
        services.readiness.profile,
        RebornCompositionProfile::LocalDevYolo
    );
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    assert_eq!(
        services.readiness.diagnostics,
        vec![RebornReadinessDiagnostic::local_dev_yolo()]
    );
}

#[test]
fn readiness_diagnostics_do_not_carry_sensitive_detail_fields() {
    let readiness = readiness_for_contract(
        RebornCompositionProfile::Production,
        RebornReadinessState::ProductionValidated,
        vec![
            production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::SecretStore,
                RebornReadinessDiagnosticReason::Missing,
            ),
            production_blocker(
                RebornCompositionProfile::Production,
                RebornReadinessDiagnosticComponent::ApprovalRequests,
                RebornReadinessDiagnosticReason::LocalOnly,
            ),
            production_blocker(
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

#[test]
fn production_wiring_issue_kinds_map_to_stable_readiness_reasons() {
    assert_eq!(
        RebornReadinessDiagnosticReason::from(ProductionWiringIssueKind::Missing),
        RebornReadinessDiagnosticReason::Missing
    );
    assert_eq!(
        RebornReadinessDiagnosticReason::from(ProductionWiringIssueKind::LocalOnlyImplementation),
        RebornReadinessDiagnosticReason::LocalOnly
    );
    assert_eq!(
        RebornReadinessDiagnosticReason::from(
            ProductionWiringIssueKind::UnverifiedProductionImplementation,
        ),
        RebornReadinessDiagnosticReason::Unverified
    );
    assert_eq!(
        RebornReadinessDiagnosticReason::from(ProductionWiringIssueKind::UnsupportedRequirement),
        RebornReadinessDiagnosticReason::Unsupported
    );
}

#[test]
fn production_wiring_components_keep_host_runtime_stable_names() {
    for component in [
        ProductionWiringComponent::RuntimeBackend,
        ProductionWiringComponent::RuntimePolicy,
        ProductionWiringComponent::TrustPolicy,
        ProductionWiringComponent::Filesystem,
        ProductionWiringComponent::ResourceGovernor,
        ProductionWiringComponent::ProcessStore,
        ProductionWiringComponent::ProcessResultStore,
        ProductionWiringComponent::RunState,
        ProductionWiringComponent::ApprovalRequests,
        ProductionWiringComponent::CapabilityLeases,
        ProductionWiringComponent::PersistentApprovalPolicies,
        ProductionWiringComponent::EventSink,
        ProductionWiringComponent::AuditSink,
        ProductionWiringComponent::SecretStore,
        ProductionWiringComponent::CredentialAccountStore,
        ProductionWiringComponent::CredentialSessionStore,
        ProductionWiringComponent::RuntimeHttpEgress,
        ProductionWiringComponent::RuntimeProcessPort,
        ProductionWiringComponent::WasmCredentialProvider,
        ProductionWiringComponent::ScriptRuntime,
        ProductionWiringComponent::McpRuntime,
        ProductionWiringComponent::WasmRuntime,
        ProductionWiringComponent::FirstPartyRuntime,
        ProductionWiringComponent::TurnState,
        ProductionWiringComponent::RunProfileResolver,
        ProductionWiringComponent::TurnRunWakeNotifier,
    ] {
        let expected = component.as_str();
        let readiness_component = RebornReadinessDiagnosticComponent::from(component);
        let serialized = serde_json::to_value(readiness_component).unwrap();

        assert_eq!(serialized, json!(expected));
    }
}

#[test]
fn production_wiring_report_with_no_issues_returns_empty_diagnostics() {
    let report = ProductionWiringReport::for_test(Vec::new());

    for profile in [
        RebornCompositionProfile::Production,
        RebornCompositionProfile::MigrationDryRun,
    ] {
        assert!(
            RebornReadinessDiagnostic::from_production_wiring_report(profile, &report).is_empty()
        );
    }
}

#[test]
fn production_wiring_report_skipped_for_non_production_profiles() {
    let report = ProductionWiringReport::for_test(vec![ProductionWiringIssue::for_test(
        ProductionWiringComponent::SecretStore,
        ProductionWiringIssueKind::Missing,
    )]);

    for profile in [
        RebornCompositionProfile::Disabled,
        RebornCompositionProfile::LocalDev,
        RebornCompositionProfile::LocalDevYolo,
        RebornCompositionProfile::HostedSingleTenant,
        RebornCompositionProfile::HostedSingleTenantVolume,
    ] {
        assert!(
            RebornReadinessDiagnostic::from_production_wiring_report(profile, &report).is_empty()
        );
    }
}

#[test]
fn production_wiring_report_maps_through_public_readiness_entrypoint() {
    let report = ProductionWiringReport::for_test(vec![
        ProductionWiringIssue::for_test(
            ProductionWiringComponent::SecretStore,
            ProductionWiringIssueKind::Missing,
        ),
        ProductionWiringIssue::for_test(
            ProductionWiringComponent::AuditSink,
            ProductionWiringIssueKind::UnverifiedProductionImplementation,
        ),
        ProductionWiringIssue::for_test(
            ProductionWiringComponent::RuntimeBackend,
            ProductionWiringIssueKind::UnsupportedRequirement,
        ),
    ]);

    for profile in [
        RebornCompositionProfile::Production,
        RebornCompositionProfile::MigrationDryRun,
    ] {
        let diagnostics =
            RebornReadinessDiagnostic::from_production_wiring_report(profile, &report);

        assert_eq!(diagnostics.len(), 3);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.status == RebornReadinessDiagnosticStatus::Blocking
                && diagnostic.blocks_production
        }));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            RebornReadinessDiagnosticComponent::SecretStore,
            RebornReadinessDiagnosticReason::Missing,
        )));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            RebornReadinessDiagnosticComponent::AuditSink,
            RebornReadinessDiagnosticReason::Unverified,
        )));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            RebornReadinessDiagnosticComponent::RuntimeBackend,
            RebornReadinessDiagnosticReason::Unsupported,
        )));
    }

    assert!(
        RebornReadinessDiagnostic::from_production_wiring_report(
            RebornCompositionProfile::LocalDev,
            &report,
        )
        .is_empty()
    );
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

fn production_blocker(
    profile: RebornCompositionProfile,
    component: RebornReadinessDiagnosticComponent,
    reason: RebornReadinessDiagnosticReason,
) -> RebornReadinessDiagnostic {
    RebornReadinessDiagnostic::production_blocker(profile, component, reason)
        .expect("test uses a production-shaped profile")
}
