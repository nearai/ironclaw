use std::{ffi::OsString, str::FromStr};

use ironclaw_config::{
    DeploymentSecurityEnvelope, DurableStateKind, LayoutManifest, LayoutRequirement,
    LegacyStorageSource, ProfileTransitionAdmission, REBORN_PROFILE_ENV, RebornBootConfig,
    RebornConfigError, RebornHome, RebornProfile, RebornStoragePaths, StateLayoutVersion,
    TenancyModel, WorkspaceAccessFloor,
};

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LegacySourceRecord {
    source: LegacyStorageSource,
}

#[test]
fn legacy_storage_sources_preserve_journal_wire_values_and_snapshot_paths() {
    struct Case {
        source: LegacyStorageSource,
        wire_value: &'static str,
        profile_directory: Option<&'static str>,
        requirement: LayoutRequirement,
    }

    let cases = [
        Case {
            source: LegacyStorageSource::LocalDev,
            wire_value: "local-dev",
            profile_directory: Some("local-dev"),
            requirement: layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                TenancyModel::SingleUser,
                WorkspaceAccessFloor::SingleTrustedOperator,
            ),
        },
        Case {
            source: LegacyStorageSource::HostedSingleTenant,
            wire_value: "hosted-single-tenant",
            profile_directory: Some("hosted-single-tenant"),
            requirement: layout_requirement(
                DurableStateKind::ExternalPostgres,
                TenancyModel::SingleUser,
                WorkspaceAccessFloor::SingleTrustedOperator,
            ),
        },
        Case {
            source: LegacyStorageSource::HostedSingleTenantVolume,
            wire_value: "hosted-single-tenant-volume",
            profile_directory: Some("hosted-single-tenant-volume"),
            requirement: layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                TenancyModel::MultiUser,
                WorkspaceAccessFloor::PerCallerIsolated,
            ),
        },
        Case {
            source: LegacyStorageSource::BareHome,
            wire_value: "bare-home",
            profile_directory: None,
            requirement: layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                TenancyModel::SingleUser,
                WorkspaceAccessFloor::SingleTrustedOperator,
            ),
        },
    ];
    let paths = RebornStoragePaths::from_installation_root("/stable-reborn-home");

    for case in cases {
        let record = LegacySourceRecord {
            source: case.source,
        };
        let encoded = toml::to_string(&record).expect("serialize source record");
        assert_eq!(encoded, format!("source = \"{}\"\n", case.wire_value));
        assert_eq!(
            toml::from_str::<LegacySourceRecord>(&encoded).expect("deserialize source record"),
            record
        );
        assert_eq!(case.source.profile_directory(), case.profile_directory);
        assert_eq!(
            case.source.snapshot_root(&paths),
            paths
                .runtime_root()
                .join("layout-adoption/snapshot")
                .join(case.wire_value)
        );
        assert_eq!(case.source.requirement(), case.requirement);
    }
}

#[test]
fn profile_wire_values_are_stable() {
    assert_eq!(RebornProfile::Standalone.as_str(), "local-dev");
    assert_eq!(
        RebornProfile::StandaloneUnrestricted.as_str(),
        "local-dev-yolo"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenant.as_str(),
        "hosted-single-tenant"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenantVolume.as_str(),
        "hosted-single-tenant-volume"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenantVolumeSandboxed.as_str(),
        "hosted-single-tenant-volume-sandboxed"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenantVolumeSandboxedRailway.as_str(),
        "hosted-single-tenant-volume-sandboxed-railway"
    );
    assert_eq!(RebornProfile::Production.as_str(), "production");
    assert_eq!(RebornProfile::MigrationDryRun.as_str(), "migration-dry-run");
}

#[test]
fn all_profiles_are_exposed_in_display_order() {
    assert_eq!(
        RebornProfile::all(),
        &[
            RebornProfile::Standalone,
            RebornProfile::StandaloneUnrestricted,
            RebornProfile::HostedSingleTenant,
            RebornProfile::HostedSingleTenantVolume,
            RebornProfile::HostedSingleTenantVolumeSandboxed,
            RebornProfile::HostedSingleTenantVolumeSandboxedRailway,
            RebornProfile::Production,
            RebornProfile::MigrationDryRun,
        ]
    );
}

#[test]
fn profile_parsing_accepts_expected_values() {
    assert_eq!(
        RebornProfile::from_str("local-dev"),
        Ok(RebornProfile::Standalone)
    );
    assert_eq!(
        RebornProfile::from_str("local-dev-yolo"),
        Ok(RebornProfile::StandaloneUnrestricted)
    );
    assert_eq!(
        RebornProfile::from_str("hosted-single-tenant"),
        Ok(RebornProfile::HostedSingleTenant)
    );
    assert_eq!(
        RebornProfile::from_str("hosted-single-tenant-volume"),
        Ok(RebornProfile::HostedSingleTenantVolume)
    );
    assert_eq!(
        RebornProfile::from_str("hosted-single-tenant-volume-sandboxed"),
        Ok(RebornProfile::HostedSingleTenantVolumeSandboxed)
    );
    assert_eq!(
        RebornProfile::from_str("hosted-single-tenant-volume-sandboxed-railway"),
        Ok(RebornProfile::HostedSingleTenantVolumeSandboxedRailway)
    );
    assert_eq!(
        RebornProfile::from_str("production"),
        Ok(RebornProfile::Production)
    );
    assert_eq!(
        RebornProfile::from_str("migration-dry-run"),
        Ok(RebornProfile::MigrationDryRun)
    );
}

#[test]
fn profile_predicates_capture_hosted_listener_and_skill_management_contract() {
    assert!(!RebornProfile::Standalone.starts_hosted_single_tenant_listener());
    assert!(!RebornProfile::StandaloneUnrestricted.starts_hosted_single_tenant_listener());
    assert!(RebornProfile::HostedSingleTenant.starts_hosted_single_tenant_listener());
    assert!(RebornProfile::HostedSingleTenantVolume.starts_hosted_single_tenant_listener());
    assert!(
        RebornProfile::HostedSingleTenantVolumeSandboxed.starts_hosted_single_tenant_listener()
    );
    assert!(
        RebornProfile::HostedSingleTenantVolumeSandboxedRailway
            .starts_hosted_single_tenant_listener()
    );
    assert!(!RebornProfile::Production.starts_hosted_single_tenant_listener());
    assert!(!RebornProfile::MigrationDryRun.starts_hosted_single_tenant_listener());

    assert!(RebornProfile::Standalone.supports_local_runtime_skill_management());
    assert!(RebornProfile::StandaloneUnrestricted.supports_local_runtime_skill_management());
    assert!(RebornProfile::HostedSingleTenant.supports_local_runtime_skill_management());
    assert!(RebornProfile::HostedSingleTenantVolume.supports_local_runtime_skill_management());
    assert!(
        RebornProfile::HostedSingleTenantVolumeSandboxed.supports_local_runtime_skill_management()
    );
    assert!(
        RebornProfile::HostedSingleTenantVolumeSandboxedRailway
            .supports_local_runtime_skill_management()
    );
    assert!(!RebornProfile::Production.supports_local_runtime_skill_management());
    assert!(!RebornProfile::MigrationDryRun.supports_local_runtime_skill_management());
}

#[test]
fn profile_default_is_standalone_for_explicit_binary_invocations() {
    assert_eq!(RebornProfile::default(), RebornProfile::Standalone);
}

#[test]
fn invalid_profile_is_rejected() {
    let err = RebornProfile::from_str("prod").expect_err("invalid profile should fail");

    assert_eq!(
        err,
        RebornConfigError::InvalidProfile {
            name: REBORN_PROFILE_ENV,
            value: "prod".to_string(),
        }
    );
}

#[test]
fn boot_config_resolves_home_and_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config = RebornBootConfig::resolve_from_env_parts(
        Some(temp.path().join("reborn-home").into_os_string()),
        None,
        None,
        Some(OsString::from("production")),
    )
    .expect("boot config should resolve");

    assert_eq!(
        config.home().path(),
        temp.path().join("reborn-home").as_path()
    );
    assert_eq!(config.profile(), RebornProfile::Production);
}

#[test]
fn boot_config_defaults_profile_to_standalone() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config =
        RebornBootConfig::resolve_from_env_parts(None, Some(temp.path().into()), None, None)
            .expect("boot config should resolve");

    assert_eq!(config.profile(), RebornProfile::Standalone);
}

#[test]
fn boot_config_rejects_invalid_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = RebornBootConfig::resolve_from_env_parts(
        Some(temp.path().join("reborn-home").into_os_string()),
        None,
        None,
        Some(OsString::from("prod")),
    )
    .expect_err("invalid boot profile should fail through the caller-level config path");

    assert_eq!(
        error,
        RebornConfigError::InvalidProfile {
            name: REBORN_PROFILE_ENV,
            value: "prod".to_string(),
        }
    );
}

#[test]
fn boot_config_rejects_empty_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = RebornBootConfig::resolve_from_env_parts(
        Some(temp.path().join("reborn-home").into_os_string()),
        None,
        None,
        Some(OsString::from("")),
    )
    .expect_err("empty boot profile should fail through the caller-level config path");

    assert_eq!(
        error,
        RebornConfigError::InvalidProfile {
            name: REBORN_PROFILE_ENV,
            value: String::new(),
        }
    );
}

#[test]
fn storage_paths_are_derived_from_reborn_home_without_creating_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected_home = temp.path().join("reborn-home");
    let home = RebornHome::resolve_from_env_parts(
        Some(expected_home.clone().into_os_string()),
        None,
        None,
    )
    .expect("Reborn home should resolve");

    let paths = RebornStoragePaths::from_home(&home);

    assert_eq!(paths.state_root(), expected_home.join("state"));
    assert_eq!(paths.system_root(), expected_home.join("system"));
    assert_eq!(paths.workspace_root(), expected_home.join("workspaces"));
    assert_eq!(paths.runtime_root(), expected_home.join("runtime"));
    assert_eq!(paths.logs_root(), expected_home.join("logs"));
    assert_eq!(paths.cache_root(), expected_home.join("cache"));
    assert_eq!(paths.temp_root(), expected_home.join("tmp"));
    assert_eq!(
        paths
            .canonical_namespace_roots()
            .map(|path| path.file_name().expect("namespace name")),
        [
            "state",
            "system",
            "workspaces",
            "runtime",
            "logs",
            "cache",
            "tmp"
        ]
    );
    assert!(
        !expected_home.exists(),
        "deriving pure layout paths must not create the Reborn home"
    );
}

#[test]
fn layout_manifest_v1_toml_wire_values_are_stable() {
    struct Case {
        name: &'static str,
        requirement: LayoutRequirement,
        expected_toml: &'static str,
    }

    let cases = [
        Case {
            name: "embedded single trusted operator",
            requirement: LayoutRequirement {
                durable_state: DurableStateKind::EmbeddedLibSql,
                security: DeploymentSecurityEnvelope {
                    tenancy: TenancyModel::SingleUser,
                    workspace_access_floor: WorkspaceAccessFloor::SingleTrustedOperator,
                },
            },
            expected_toml: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
        },
        Case {
            name: "embedded single isolated",
            requirement: LayoutRequirement {
                durable_state: DurableStateKind::EmbeddedLibSql,
                security: DeploymentSecurityEnvelope {
                    tenancy: TenancyModel::SingleUser,
                    workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
                },
            },
            expected_toml: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"per-caller-isolated\"\n",
        },
        Case {
            name: "external multi trusted operator",
            requirement: LayoutRequirement {
                durable_state: DurableStateKind::ExternalPostgres,
                security: DeploymentSecurityEnvelope {
                    tenancy: TenancyModel::MultiUser,
                    workspace_access_floor: WorkspaceAccessFloor::SingleTrustedOperator,
                },
            },
            expected_toml: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"external-postgres\"\n\n[security]\ntenancy = \"multi-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
        },
        Case {
            name: "external multi isolated",
            requirement: LayoutRequirement {
                durable_state: DurableStateKind::ExternalPostgres,
                security: DeploymentSecurityEnvelope {
                    tenancy: TenancyModel::MultiUser,
                    workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
                },
            },
            expected_toml: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"external-postgres\"\n\n[security]\ntenancy = \"multi-user\"\nworkspace_access_floor = \"per-caller-isolated\"\n",
        },
    ];

    for case in cases {
        let manifest = LayoutManifest::new(case.requirement);

        assert_eq!(manifest.schema_version(), 1, "case: {}", case.name);
        assert_eq!(
            manifest.state_layout_version(),
            StateLayoutVersion::V1,
            "case: {}",
            case.name
        );
        assert_eq!(
            toml::to_string(&manifest).expect("manifest should serialize"),
            case.expected_toml,
            "case: {}",
            case.name
        );
        assert_eq!(
            toml::from_str::<LayoutManifest>(case.expected_toml)
                .expect("manifest should deserialize"),
            manifest,
            "case: {}",
            case.name
        );
    }
}

#[test]
fn layout_manifest_rejects_unsupported_or_unowned_wire_fields() {
    struct Case {
        name: &'static str,
        manifest: &'static str,
        expected_error_fragment: &'static str,
    }

    let cases = [
        Case {
            name: "unsupported schema version",
            manifest: "schema_version = 2\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "unsupported layout manifest schema_version 2",
        },
        Case {
            name: "unsupported state layout version",
            manifest: "schema_version = 1\nstate_layout_version = 2\ndurable_state = \"embedded-libsql\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "unsupported state layout version 2",
        },
        Case {
            name: "non kebab case durable state",
            manifest: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded_libsql\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "embedded_libsql",
        },
        Case {
            name: "profile name",
            manifest: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\nprofile = \"local-dev\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "profile",
        },
        Case {
            name: "state path",
            manifest: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\nstate_root = \"/operator/state\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "state_root",
        },
        Case {
            name: "process backend",
            manifest: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\nprocess_backend = \"docker\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "process_backend",
        },
        Case {
            name: "transient execution authority",
            manifest: "schema_version = 1\nstate_layout_version = 1\ndurable_state = \"embedded-libsql\"\nruntime_authority = \"unrestricted\"\n\n[security]\ntenancy = \"single-user\"\nworkspace_access_floor = \"single-trusted-operator\"\n",
            expected_error_fragment: "runtime_authority",
        },
    ];

    for case in cases {
        let error = toml::from_str::<LayoutManifest>(case.manifest)
            .expect_err("unsupported manifest input must fail closed");

        assert!(
            error.to_string().contains(case.expected_error_fragment),
            "case: {} error: {error}",
            case.name
        );
    }
}

#[test]
fn layout_manifest_round_trips_the_persisted_memory_provider_namespace() {
    let requirement = LayoutRequirement {
        durable_state: DurableStateKind::EmbeddedLibSql,
        security: DeploymentSecurityEnvelope {
            tenancy: TenancyModel::SingleUser,
            workspace_access_floor: WorkspaceAccessFloor::SingleTrustedOperator,
        },
    };
    let app_id = ironclaw_config::legacy_memory_provider_app_id(std::path::Path::new(
        "/var/lib/ironclaw/local-dev",
    ));
    let manifest = LayoutManifest::new(requirement).with_memory_provider_app_id(app_id.clone());
    let serialized = toml::to_string(&manifest).expect("manifest serializes");
    let decoded: LayoutManifest = toml::from_str(&serialized).expect("manifest deserializes");

    assert_eq!(decoded, manifest);
    assert_eq!(decoded.memory_provider_app_id(), Some(app_id.as_str()));
}

#[test]
fn released_and_canonical_memory_provider_derivations_remain_distinct_and_stable() {
    let root = std::path::Path::new("/var/lib/ironclaw");
    assert_eq!(
        ironclaw_config::legacy_memory_provider_app_id(root),
        "ws-f4f432cf4db72cc2"
    );
    assert_eq!(
        ironclaw_config::canonical_memory_provider_app_id(root),
        "ws-f0d6f77ada36695664007e305f03546485e25e5f295cb273657c4370f4aaab01"
    );
}

#[test]
fn canonical_memory_provider_derivation_normalizes_equivalent_path_spelling() {
    let canonical = std::path::Path::new("/var/lib/ironclaw");
    for equivalent in [
        std::path::Path::new("/var/lib/ironclaw/"),
        std::path::Path::new("/var//lib/./ironclaw"),
    ] {
        assert_eq!(
            ironclaw_config::canonical_memory_provider_app_id(equivalent),
            ironclaw_config::canonical_memory_provider_app_id(canonical),
            "equivalent spelling must not create a second durable memory namespace: {}",
            equivalent.display()
        );
    }
}

#[test]
fn layout_manifest_transition_admission_has_an_explicit_durable_state_matrix() {
    struct Case {
        name: &'static str,
        stored: DurableStateKind,
        requested: DurableStateKind,
        expected_reason: Option<&'static str>,
    }

    let cases = [
        Case {
            name: "embedded libsql remains embedded libsql",
            stored: DurableStateKind::EmbeddedLibSql,
            requested: DurableStateKind::EmbeddedLibSql,
            expected_reason: None,
        },
        Case {
            name: "embedded libsql changes to external postgres",
            stored: DurableStateKind::EmbeddedLibSql,
            requested: DurableStateKind::ExternalPostgres,
            expected_reason: Some(
                "durable state transition from embedded-libsql to external-postgres requires an explicit storage migration",
            ),
        },
        Case {
            name: "external postgres changes to embedded libsql",
            stored: DurableStateKind::ExternalPostgres,
            requested: DurableStateKind::EmbeddedLibSql,
            expected_reason: Some(
                "durable state transition from external-postgres to embedded-libsql requires an explicit storage migration",
            ),
        },
        Case {
            name: "external postgres remains external postgres",
            stored: DurableStateKind::ExternalPostgres,
            requested: DurableStateKind::ExternalPostgres,
            expected_reason: None,
        },
    ];

    for case in cases {
        assert_layout_transition(
            case.name,
            layout_requirement(
                case.stored,
                TenancyModel::SingleUser,
                WorkspaceAccessFloor::SingleTrustedOperator,
            ),
            layout_requirement(
                case.requested,
                TenancyModel::SingleUser,
                WorkspaceAccessFloor::SingleTrustedOperator,
            ),
            case.expected_reason,
        );
    }
}

#[test]
fn layout_manifest_transition_admission_has_an_explicit_tenancy_matrix() {
    struct Case {
        name: &'static str,
        stored: TenancyModel,
        requested: TenancyModel,
        expected_reason: Option<&'static str>,
    }

    let cases = [
        Case {
            name: "single user remains single user",
            stored: TenancyModel::SingleUser,
            requested: TenancyModel::SingleUser,
            expected_reason: None,
        },
        Case {
            name: "single user changes to multi user",
            stored: TenancyModel::SingleUser,
            requested: TenancyModel::MultiUser,
            expected_reason: Some(
                "tenancy transition from single-user to multi-user requires an explicit ownership migration",
            ),
        },
        Case {
            name: "multi user changes to single user",
            stored: TenancyModel::MultiUser,
            requested: TenancyModel::SingleUser,
            expected_reason: Some(
                "tenancy transition from multi-user to single-user requires an explicit ownership migration",
            ),
        },
        Case {
            name: "multi user remains multi user",
            stored: TenancyModel::MultiUser,
            requested: TenancyModel::MultiUser,
            expected_reason: None,
        },
    ];

    for case in cases {
        assert_layout_transition(
            case.name,
            layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                case.stored,
                WorkspaceAccessFloor::PerCallerIsolated,
            ),
            layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                case.requested,
                WorkspaceAccessFloor::PerCallerIsolated,
            ),
            case.expected_reason,
        );
    }
}

#[test]
fn layout_manifest_transition_admission_has_an_explicit_workspace_access_floor_matrix() {
    struct Case {
        name: &'static str,
        stored: WorkspaceAccessFloor,
        requested: WorkspaceAccessFloor,
        expected_reason: Option<&'static str>,
    }

    let cases = [
        Case {
            name: "single trusted operator remains single trusted operator",
            stored: WorkspaceAccessFloor::SingleTrustedOperator,
            requested: WorkspaceAccessFloor::SingleTrustedOperator,
            expected_reason: None,
        },
        Case {
            name: "single trusted operator tightens to per caller isolation",
            stored: WorkspaceAccessFloor::SingleTrustedOperator,
            requested: WorkspaceAccessFloor::PerCallerIsolated,
            expected_reason: None,
        },
        Case {
            name: "per caller isolation weakens to single trusted operator",
            stored: WorkspaceAccessFloor::PerCallerIsolated,
            requested: WorkspaceAccessFloor::SingleTrustedOperator,
            expected_reason: Some(
                "workspace access floor cannot weaken from per-caller-isolated to single-trusted-operator",
            ),
        },
        Case {
            name: "per caller isolation remains per caller isolation",
            stored: WorkspaceAccessFloor::PerCallerIsolated,
            requested: WorkspaceAccessFloor::PerCallerIsolated,
            expected_reason: None,
        },
    ];

    for case in cases {
        assert_layout_transition(
            case.name,
            layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                TenancyModel::SingleUser,
                case.stored,
            ),
            layout_requirement(
                DurableStateKind::EmbeddedLibSql,
                TenancyModel::SingleUser,
                case.requested,
            ),
            case.expected_reason,
        );
    }
}

#[test]
fn layout_manifest_transition_admission_preserves_rejection_precedence() {
    assert_layout_transition(
        "durable state rejection precedes tenancy and workspace access floor changes",
        layout_requirement(
            DurableStateKind::ExternalPostgres,
            TenancyModel::MultiUser,
            WorkspaceAccessFloor::PerCallerIsolated,
        ),
        layout_requirement(
            DurableStateKind::EmbeddedLibSql,
            TenancyModel::SingleUser,
            WorkspaceAccessFloor::SingleTrustedOperator,
        ),
        Some(
            "durable state transition from external-postgres to embedded-libsql requires an explicit storage migration",
        ),
    );

    assert_layout_transition(
        "tenancy rejection precedes a workspace access floor weakening",
        layout_requirement(
            DurableStateKind::EmbeddedLibSql,
            TenancyModel::MultiUser,
            WorkspaceAccessFloor::PerCallerIsolated,
        ),
        layout_requirement(
            DurableStateKind::EmbeddedLibSql,
            TenancyModel::SingleUser,
            WorkspaceAccessFloor::SingleTrustedOperator,
        ),
        Some(
            "tenancy transition from multi-user to single-user requires an explicit ownership migration",
        ),
    );
}

fn layout_requirement(
    durable_state: DurableStateKind,
    tenancy: TenancyModel,
    workspace_access_floor: WorkspaceAccessFloor,
) -> LayoutRequirement {
    LayoutRequirement {
        durable_state,
        security: DeploymentSecurityEnvelope {
            tenancy,
            workspace_access_floor,
        },
    }
}

fn assert_layout_transition(
    name: &str,
    stored: LayoutRequirement,
    requested: LayoutRequirement,
    expected_reason: Option<&str>,
) {
    let expected = match expected_reason {
        Some(reason) => ProfileTransitionAdmission::Rejected {
            reason: reason.to_owned(),
        },
        None => ProfileTransitionAdmission::Allowed,
    };

    assert_eq!(
        LayoutManifest::new(stored).admit(requested),
        expected,
        "case: {name}",
    );
}
