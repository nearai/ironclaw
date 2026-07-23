use std::{ffi::OsString, str::FromStr};

use ironclaw_config::{
    IRONCLAW_PROFILE_ENV, IronClawBootConfig, IronClawConfigError, IronClawProfile,
};

#[test]
fn profile_wire_values_are_stable() {
    assert_eq!(IronClawProfile::LocalDev.as_str(), "local-dev");
    assert_eq!(IronClawProfile::LocalDevYolo.as_str(), "local-dev-yolo");
    assert_eq!(
        IronClawProfile::HostedSingleTenant.as_str(),
        "hosted-single-tenant"
    );
    assert_eq!(
        IronClawProfile::HostedSingleTenantVolume.as_str(),
        "hosted-single-tenant-volume"
    );
    assert_eq!(IronClawProfile::Production.as_str(), "production");
    assert_eq!(
        IronClawProfile::MigrationDryRun.as_str(),
        "migration-dry-run"
    );
}

#[test]
fn all_profiles_are_exposed_in_display_order() {
    assert_eq!(
        IronClawProfile::all(),
        &[
            IronClawProfile::LocalDev,
            IronClawProfile::LocalDevYolo,
            IronClawProfile::HostedSingleTenant,
            IronClawProfile::HostedSingleTenantVolume,
            IronClawProfile::Production,
            IronClawProfile::MigrationDryRun,
        ]
    );
}

#[test]
fn profile_parsing_accepts_expected_values() {
    assert_eq!(
        IronClawProfile::from_str("local-dev"),
        Ok(IronClawProfile::LocalDev)
    );
    assert_eq!(
        IronClawProfile::from_str("local-dev-yolo"),
        Ok(IronClawProfile::LocalDevYolo)
    );
    assert_eq!(
        IronClawProfile::from_str("hosted-single-tenant"),
        Ok(IronClawProfile::HostedSingleTenant)
    );
    assert_eq!(
        IronClawProfile::from_str("hosted-single-tenant-volume"),
        Ok(IronClawProfile::HostedSingleTenantVolume)
    );
    assert_eq!(
        IronClawProfile::from_str("production"),
        Ok(IronClawProfile::Production)
    );
    assert_eq!(
        IronClawProfile::from_str("migration-dry-run"),
        Ok(IronClawProfile::MigrationDryRun)
    );
}

#[test]
fn profile_predicates_capture_hosted_volume_local_runtime_contract() {
    assert!(!IronClawProfile::LocalDev.starts_hosted_single_tenant_listener());
    assert!(!IronClawProfile::LocalDevYolo.starts_hosted_single_tenant_listener());
    assert!(IronClawProfile::HostedSingleTenant.starts_hosted_single_tenant_listener());
    assert!(IronClawProfile::HostedSingleTenantVolume.starts_hosted_single_tenant_listener());
    assert!(!IronClawProfile::Production.starts_hosted_single_tenant_listener());
    assert!(!IronClawProfile::MigrationDryRun.starts_hosted_single_tenant_listener());

    assert!(IronClawProfile::LocalDev.uses_standalone_local_runtime_volume());
    assert!(IronClawProfile::LocalDevYolo.uses_standalone_local_runtime_volume());
    assert!(!IronClawProfile::HostedSingleTenant.uses_standalone_local_runtime_volume());
    assert!(IronClawProfile::HostedSingleTenantVolume.uses_standalone_local_runtime_volume());
    assert!(!IronClawProfile::Production.uses_standalone_local_runtime_volume());
    assert!(!IronClawProfile::MigrationDryRun.uses_standalone_local_runtime_volume());

    assert_eq!(
        IronClawProfile::LocalDev.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        IronClawProfile::LocalDevYolo.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        IronClawProfile::HostedSingleTenant.local_runtime_storage_subdir(),
        "hosted-single-tenant"
    );
    assert_eq!(
        IronClawProfile::HostedSingleTenantVolume.local_runtime_storage_subdir(),
        "hosted-single-tenant-volume"
    );
    assert_eq!(
        IronClawProfile::Production.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        IronClawProfile::MigrationDryRun.local_runtime_storage_subdir(),
        "local-dev"
    );

    assert!(IronClawProfile::LocalDev.supports_local_runtime_skill_management());
    assert!(IronClawProfile::LocalDevYolo.supports_local_runtime_skill_management());
    assert!(IronClawProfile::HostedSingleTenant.supports_local_runtime_skill_management());
    assert!(IronClawProfile::HostedSingleTenantVolume.supports_local_runtime_skill_management());
    assert!(!IronClawProfile::Production.supports_local_runtime_skill_management());
    assert!(!IronClawProfile::MigrationDryRun.supports_local_runtime_skill_management());
}

#[test]
fn profile_default_is_local_dev_for_explicit_binary_invocations() {
    assert_eq!(IronClawProfile::default(), IronClawProfile::LocalDev);
}

#[test]
fn invalid_profile_is_rejected() {
    let err = IronClawProfile::from_str("prod").expect_err("invalid profile should fail");

    assert_eq!(
        err,
        IronClawConfigError::InvalidProfile {
            name: IRONCLAW_PROFILE_ENV,
            value: "prod".to_string(),
        }
    );
}

#[test]
fn boot_config_resolves_home_and_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config = IronClawBootConfig::resolve_from_env_parts(
        Some(temp.path().join("ironclaw-home").into_os_string()),
        None,
        None,
        Some(OsString::from("production")),
    )
    .expect("boot config should resolve");

    assert_eq!(
        config.home().path(),
        temp.path().join("ironclaw-home").as_path()
    );
    assert_eq!(config.profile(), IronClawProfile::Production);
}

#[test]
fn boot_config_defaults_profile_to_local_dev() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config =
        IronClawBootConfig::resolve_from_env_parts(None, Some(temp.path().into()), None, None)
            .expect("boot config should resolve");

    assert_eq!(config.profile(), IronClawProfile::LocalDev);
}

#[test]
fn canonical_profile_wins_over_legacy_profile() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config = IronClawBootConfig::resolve_from_env_parts_with_legacy(
        Some(temp.path().join("ironclaw-home").into_os_string()),
        Some(temp.path().join("legacy-home").into_os_string()),
        None,
        None,
        Some(OsString::from("production")),
        Some(OsString::from("local-dev")),
    )
    .expect("canonical profile and home should win");

    assert_eq!(config.profile(), IronClawProfile::Production);
    assert_eq!(
        config.home().path(),
        temp.path().join("ironclaw-home").as_path()
    );
}

#[test]
fn boot_config_rejects_invalid_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = IronClawBootConfig::resolve_from_env_parts(
        Some(temp.path().join("ironclaw-home").into_os_string()),
        None,
        None,
        Some(OsString::from("prod")),
    )
    .expect_err("invalid boot profile should fail through the caller-level config path");

    assert_eq!(
        error,
        IronClawConfigError::InvalidProfile {
            name: IRONCLAW_PROFILE_ENV,
            value: "prod".to_string(),
        }
    );
}

#[test]
fn boot_config_rejects_empty_profile_from_env_parts() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = IronClawBootConfig::resolve_from_env_parts(
        Some(temp.path().join("ironclaw-home").into_os_string()),
        None,
        None,
        Some(OsString::from("")),
    )
    .expect_err("empty boot profile should fail through the caller-level config path");

    assert_eq!(
        error,
        IronClawConfigError::InvalidProfile {
            name: IRONCLAW_PROFILE_ENV,
            value: String::new(),
        }
    );
}
