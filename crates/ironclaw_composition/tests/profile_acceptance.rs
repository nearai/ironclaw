use ironclaw_composition::{
    IronClawBuildInput, IronClawCompositionProfile, IronClawFacadeReadiness, IronClawReadiness,
    IronClawReadinessDiagnostic, IronClawReadinessDiagnosticComponent,
    IronClawReadinessDiagnosticReason, IronClawReadinessDiagnosticStatus, IronClawReadinessState,
    IronClawRuntimeProfileOptions, IronClawWorkerReadiness, build_ironclaw_services,
    hosted_single_tenant_volume_runtime_policy, local_dev_yolo_runtime_policy,
    local_runtime_build_input_with_options,
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
        "disabled".parse::<IronClawCompositionProfile>().unwrap(),
        IronClawCompositionProfile::Disabled
    );
    assert_eq!(
        "local_dev".parse::<IronClawCompositionProfile>().unwrap(),
        IronClawCompositionProfile::LocalDev
    );
    assert_eq!(
        "local_dev_yolo"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::LocalDevYolo
    );
    assert_eq!(
        "local-dev-yolo"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::LocalDevYolo
    );
    assert_eq!(
        "hosted_single_tenant"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::HostedSingleTenant
    );
    assert_eq!(
        "hosted-single-tenant"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::HostedSingleTenant
    );
    assert_eq!(
        "hosted_single_tenant_volume"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        "hosted-single-tenant-volume"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        "migration-dry-run"
            .parse::<IronClawCompositionProfile>()
            .unwrap(),
        IronClawCompositionProfile::MigrationDryRun
    );
}

#[test]
fn full_graph_profiles_match_production_strictness() {
    assert!(!IronClawCompositionProfile::Disabled.requires_production_shape());
    assert!(!IronClawCompositionProfile::LocalDev.requires_production_shape());
    assert!(!IronClawCompositionProfile::LocalDevYolo.requires_production_shape());
    assert!(!IronClawCompositionProfile::HostedSingleTenant.requires_production_shape());
    assert!(!IronClawCompositionProfile::HostedSingleTenantVolume.requires_production_shape());
    assert!(IronClawCompositionProfile::Production.requires_production_shape());
    assert!(IronClawCompositionProfile::MigrationDryRun.requires_production_shape());
}

#[test]
fn profile_predicates_capture_hosted_volume_substrate_contract() {
    assert!(!IronClawCompositionProfile::Disabled.uses_local_runtime_substrate());
    assert!(IronClawCompositionProfile::LocalDev.uses_local_runtime_substrate());
    assert!(IronClawCompositionProfile::LocalDevYolo.uses_local_runtime_substrate());
    assert!(IronClawCompositionProfile::HostedSingleTenant.uses_local_runtime_substrate());
    assert!(IronClawCompositionProfile::HostedSingleTenantVolume.uses_local_runtime_substrate());
    assert!(!IronClawCompositionProfile::Production.uses_local_runtime_substrate());
    assert!(!IronClawCompositionProfile::MigrationDryRun.uses_local_runtime_substrate());

    assert!(!IronClawCompositionProfile::Disabled.uses_local_dev_storage_input());
    assert!(IronClawCompositionProfile::LocalDev.uses_local_dev_storage_input());
    assert!(IronClawCompositionProfile::LocalDevYolo.uses_local_dev_storage_input());
    assert!(!IronClawCompositionProfile::HostedSingleTenant.uses_local_dev_storage_input());
    assert!(IronClawCompositionProfile::HostedSingleTenantVolume.uses_local_dev_storage_input());
    assert!(!IronClawCompositionProfile::Production.uses_local_dev_storage_input());
    assert!(!IronClawCompositionProfile::MigrationDryRun.uses_local_dev_storage_input());

    assert!(!IronClawCompositionProfile::Disabled.uses_hosted_extension_installation_state());
    assert!(!IronClawCompositionProfile::LocalDev.uses_hosted_extension_installation_state());
    assert!(!IronClawCompositionProfile::LocalDevYolo.uses_hosted_extension_installation_state());
    assert!(
        IronClawCompositionProfile::HostedSingleTenant.uses_hosted_extension_installation_state()
    );
    assert!(
        IronClawCompositionProfile::HostedSingleTenantVolume
            .uses_hosted_extension_installation_state()
    );
    assert!(!IronClawCompositionProfile::Production.uses_hosted_extension_installation_state());
    assert!(
        !IronClawCompositionProfile::MigrationDryRun.uses_hosted_extension_installation_state()
    );

    assert!(!IronClawCompositionProfile::Disabled.starts_live_runtime());
    assert!(IronClawCompositionProfile::LocalDev.starts_live_runtime());
    assert!(IronClawCompositionProfile::LocalDevYolo.starts_live_runtime());
    assert!(IronClawCompositionProfile::HostedSingleTenant.starts_live_runtime());
    assert!(IronClawCompositionProfile::HostedSingleTenantVolume.starts_live_runtime());
    assert!(IronClawCompositionProfile::Production.starts_live_runtime());
    assert!(!IronClawCompositionProfile::MigrationDryRun.starts_live_runtime());
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
    let readiness = IronClawReadiness::disabled();
    let json = serde_json::to_string(&readiness).unwrap();
    assert!(json.contains("disabled"));
    assert!(!json.contains("postgres://"));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("secret"));
    assert_eq!(readiness.state, IronClawReadinessState::Disabled);
    assert_eq!(readiness.diagnostics.len(), 1);
    assert_eq!(
        readiness.diagnostics[0].reason,
        IronClawReadinessDiagnosticReason::Disabled
    );
    assert_eq!(
        readiness.diagnostics[0].status,
        IronClawReadinessDiagnosticStatus::Blocking
    );
    assert!(readiness.diagnostics[0].blocks_production);
}

#[test]
fn readiness_serializes_diagnostics_with_stable_redacted_vocabulary() {
    let readiness = readiness_for_contract(
        IronClawCompositionProfile::Production,
        IronClawReadinessState::ProductionValidated,
        vec![production_blocker(
            IronClawCompositionProfile::Production,
            IronClawReadinessDiagnosticComponent::RuntimeHttpEgress,
            IronClawReadinessDiagnosticReason::Unverified,
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
    let readiness: IronClawReadiness = serde_json::from_value(json!({
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
    assert_eq!(readiness.state, IronClawReadinessState::ProductionValidated);
}

#[test]
fn hosted_single_tenant_readiness_serializes_as_ready_single_tenant_profile() {
    let readiness = readiness_for_contract(
        IronClawCompositionProfile::HostedSingleTenant,
        IronClawReadinessState::HostedSingleTenantValidated,
        vec![IronClawReadinessDiagnostic::hosted_single_tenant()],
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
    let readiness: IronClawReadiness = serde_json::from_value(json!({
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
            IronClawCompositionProfile::Production,
            IronClawReadinessDiagnosticComponent::RuntimeHttpEgress,
            IronClawReadinessDiagnosticReason::Unverified,
        )]
    );
}

#[test]
fn readiness_diagnostic_unknown_wire_variants_deserialize_safely() {
    let diagnostic: IronClawReadinessDiagnostic = serde_json::from_value(json!({
        "profile": "production",
        "component": "new_future_component",
        "reason": "new-future-reason",
        "status": "new-future-status",
        "blocks_production": true
    }))
    .unwrap();

    assert_eq!(diagnostic.profile, IronClawCompositionProfile::Production);
    assert_eq!(
        diagnostic.component,
        IronClawReadinessDiagnosticComponent::Unknown("new_future_component".to_owned())
    );
    assert_eq!(
        diagnostic.reason,
        IronClawReadinessDiagnosticReason::Unknown("new-future-reason".to_owned())
    );
    assert_eq!(
        diagnostic.status,
        IronClawReadinessDiagnosticStatus::Unknown("new-future-status".to_owned())
    );
    assert!(diagnostic.blocks_production);
}

#[test]
fn readiness_diagnostic_unknown_wire_variants_round_trip_losslessly() {
    let diagnostic: IronClawReadinessDiagnostic = serde_json::from_value(json!({
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
    let diagnostic = IronClawReadinessDiagnostic::production_blocker(
        IronClawCompositionProfile::MigrationDryRun,
        IronClawReadinessDiagnosticComponent::RuntimeProcessPort,
        IronClawReadinessDiagnosticReason::Unsupported,
    )
    .expect("migration-dry-run is production-shaped");
    let encoded = serde_json::to_string(&diagnostic).unwrap();
    let decoded: IronClawReadinessDiagnostic = serde_json::from_str(&encoded).unwrap();

    assert_eq!(diagnostic, decoded);
}

#[test]
fn production_blocker_rejects_non_production_shaped_profiles() {
    for profile in [
        IronClawCompositionProfile::Disabled,
        IronClawCompositionProfile::LocalDev,
        IronClawCompositionProfile::LocalDevYolo,
        IronClawCompositionProfile::HostedSingleTenant,
        IronClawCompositionProfile::HostedSingleTenantVolume,
    ] {
        let diagnostic = IronClawReadinessDiagnostic::production_blocker(
            profile,
            IronClawReadinessDiagnosticComponent::RuntimeBackend,
            IronClawReadinessDiagnosticReason::Missing,
        );

        assert_eq!(diagnostic, None, "profile: {profile:?}");
    }
}

#[test]
fn dev_only_profiles_are_visible_non_production_in_readiness() {
    for (profile, diagnostic) in [
        (
            IronClawCompositionProfile::LocalDev,
            IronClawReadinessDiagnostic::local_dev(),
        ),
        (
            IronClawCompositionProfile::LocalDevYolo,
            IronClawReadinessDiagnostic::local_dev_yolo(),
        ),
    ] {
        assert_eq!(diagnostic.profile, profile);
        assert_eq!(
            diagnostic.component,
            IronClawReadinessDiagnosticComponent::CompositionProfile
        );
        assert_eq!(
            diagnostic.reason,
            IronClawReadinessDiagnosticReason::DevOnlyProfile
        );
        assert_eq!(
            diagnostic.status,
            IronClawReadinessDiagnosticStatus::Blocking
        );
        assert!(diagnostic.blocks_production);
    }
}

#[test]
fn hosted_single_tenant_volume_is_visible_as_preview_readiness() {
    let diagnostic = IronClawReadinessDiagnostic::hosted_single_tenant_volume();

    assert_eq!(
        diagnostic.profile,
        IronClawCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        diagnostic.component,
        IronClawReadinessDiagnosticComponent::CompositionProfile
    );
    assert_eq!(
        diagnostic.reason,
        IronClawReadinessDiagnosticReason::HostedSingleTenantVolumePreview
    );
    assert_eq!(
        diagnostic.status,
        IronClawReadinessDiagnosticStatus::Warning
    );
    assert!(diagnostic.blocks_production);
}

#[tokio::test]
async fn hosted_single_tenant_volume_factory_readiness_includes_preview_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let input = local_runtime_build_input_with_options(
        IronClawCompositionProfile::HostedSingleTenantVolume,
        "readiness-contract-owner",
        dir.path().to_path_buf(),
        Default::default(),
    )
    .unwrap();
    let services = build_ironclaw_services(input).await.unwrap();

    assert_eq!(
        services.readiness.profile,
        IronClawCompositionProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        services.readiness.state,
        IronClawReadinessState::HostedSingleTenantVolumePreviewValidated
    );
    assert_eq!(
        services.readiness.diagnostics,
        vec![IronClawReadinessDiagnostic::hosted_single_tenant_volume()]
    );
}

#[tokio::test]
async fn local_dev_factory_readiness_includes_non_production_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_ironclaw_services(IronClawBuildInput::local_dev(
        "readiness-contract-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();

    assert_eq!(
        services.readiness.profile,
        IronClawCompositionProfile::LocalDev
    );
    assert_eq!(services.readiness.state, IronClawReadinessState::DevOnly);
    assert_eq!(
        services.readiness.diagnostics,
        vec![IronClawReadinessDiagnostic::local_dev()]
    );
}

#[tokio::test]
async fn local_dev_yolo_factory_readiness_includes_non_production_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let input = local_runtime_build_input_with_options(
        IronClawCompositionProfile::LocalDevYolo,
        "readiness-yolo-owner",
        dir.path().to_path_buf(),
        IronClawRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .unwrap()
    .with_local_dev_confirmed_host_home_root(dir.path().to_path_buf());
    let services = build_ironclaw_services(input).await.unwrap();

    assert_eq!(
        services.readiness.profile,
        IronClawCompositionProfile::LocalDevYolo
    );
    assert_eq!(services.readiness.state, IronClawReadinessState::DevOnly);
    assert_eq!(
        services.readiness.diagnostics,
        vec![IronClawReadinessDiagnostic::local_dev_yolo()]
    );
}

#[test]
fn readiness_diagnostics_do_not_carry_sensitive_detail_fields() {
    let readiness = readiness_for_contract(
        IronClawCompositionProfile::Production,
        IronClawReadinessState::ProductionValidated,
        vec![
            production_blocker(
                IronClawCompositionProfile::Production,
                IronClawReadinessDiagnosticComponent::SecretStore,
                IronClawReadinessDiagnosticReason::Missing,
            ),
            production_blocker(
                IronClawCompositionProfile::Production,
                IronClawReadinessDiagnosticComponent::ApprovalRequests,
                IronClawReadinessDiagnosticReason::LocalOnly,
            ),
            production_blocker(
                IronClawCompositionProfile::Production,
                IronClawReadinessDiagnosticComponent::RuntimeBackend,
                IronClawReadinessDiagnosticReason::Unsupported,
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
        IronClawReadinessDiagnosticReason::from(ProductionWiringIssueKind::Missing),
        IronClawReadinessDiagnosticReason::Missing
    );
    assert_eq!(
        IronClawReadinessDiagnosticReason::from(ProductionWiringIssueKind::LocalOnlyImplementation),
        IronClawReadinessDiagnosticReason::LocalOnly
    );
    assert_eq!(
        IronClawReadinessDiagnosticReason::from(
            ProductionWiringIssueKind::UnverifiedProductionImplementation,
        ),
        IronClawReadinessDiagnosticReason::Unverified
    );
    assert_eq!(
        IronClawReadinessDiagnosticReason::from(ProductionWiringIssueKind::UnsupportedRequirement),
        IronClawReadinessDiagnosticReason::Unsupported
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
        let readiness_component = IronClawReadinessDiagnosticComponent::from(component);
        let serialized = serde_json::to_value(readiness_component).unwrap();

        assert_eq!(serialized, json!(expected));
    }
}

#[test]
fn production_wiring_report_with_no_issues_returns_empty_diagnostics() {
    let report = ProductionWiringReport::for_test(Vec::new());

    for profile in [
        IronClawCompositionProfile::Production,
        IronClawCompositionProfile::MigrationDryRun,
    ] {
        assert!(
            IronClawReadinessDiagnostic::from_production_wiring_report(profile, &report).is_empty()
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
        IronClawCompositionProfile::Disabled,
        IronClawCompositionProfile::LocalDev,
        IronClawCompositionProfile::LocalDevYolo,
        IronClawCompositionProfile::HostedSingleTenant,
        IronClawCompositionProfile::HostedSingleTenantVolume,
    ] {
        assert!(
            IronClawReadinessDiagnostic::from_production_wiring_report(profile, &report).is_empty()
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
        IronClawCompositionProfile::Production,
        IronClawCompositionProfile::MigrationDryRun,
    ] {
        let diagnostics =
            IronClawReadinessDiagnostic::from_production_wiring_report(profile, &report);

        assert_eq!(diagnostics.len(), 3);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.status == IronClawReadinessDiagnosticStatus::Blocking
                && diagnostic.blocks_production
        }));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            IronClawReadinessDiagnosticComponent::SecretStore,
            IronClawReadinessDiagnosticReason::Missing,
        )));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            IronClawReadinessDiagnosticComponent::AuditSink,
            IronClawReadinessDiagnosticReason::Unverified,
        )));
        assert!(diagnostics.contains(&production_blocker(
            profile,
            IronClawReadinessDiagnosticComponent::RuntimeBackend,
            IronClawReadinessDiagnosticReason::Unsupported,
        )));
    }

    assert!(
        IronClawReadinessDiagnostic::from_production_wiring_report(
            IronClawCompositionProfile::LocalDev,
            &report,
        )
        .is_empty()
    );
}

fn readiness_for_contract(
    profile: IronClawCompositionProfile,
    state: IronClawReadinessState,
    diagnostics: Vec<IronClawReadinessDiagnostic>,
) -> IronClawReadiness {
    IronClawReadiness {
        profile,
        state,
        facades: IronClawFacadeReadiness {
            host_runtime: true,
            turn_coordinator: true,
            product_auth: true,
        },
        workers: IronClawWorkerReadiness {
            turn_runner: false,
            trigger_poller: false,
        },
        diagnostics,
    }
}

fn production_blocker(
    profile: IronClawCompositionProfile,
    component: IronClawReadinessDiagnosticComponent,
    reason: IronClawReadinessDiagnosticReason,
) -> IronClawReadinessDiagnostic {
    IronClawReadinessDiagnostic::production_blocker(profile, component, reason)
        .expect("test uses a production-shaped profile")
}
