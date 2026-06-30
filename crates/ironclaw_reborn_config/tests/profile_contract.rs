use std::{ffi::OsString, str::FromStr};

use ironclaw_reborn_config::{
    REBORN_PROFILE_ENV, RebornBootConfig, RebornConfigError, RebornProfile,
};

#[test]
fn profile_wire_values_are_stable() {
    assert_eq!(RebornProfile::LocalDev.as_str(), "local-dev");
    assert_eq!(RebornProfile::LocalDevYolo.as_str(), "local-dev-yolo");
    assert_eq!(
        RebornProfile::HostedSingleTenant.as_str(),
        "hosted-single-tenant"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenantVolume.as_str(),
        "hosted-single-tenant-volume"
    );
    assert_eq!(RebornProfile::Production.as_str(), "production");
    assert_eq!(RebornProfile::MigrationDryRun.as_str(), "migration-dry-run");
}

#[test]
fn all_profiles_are_exposed_in_display_order() {
    assert_eq!(
        RebornProfile::all(),
        &[
            RebornProfile::LocalDev,
            RebornProfile::LocalDevYolo,
            RebornProfile::HostedSingleTenant,
            RebornProfile::HostedSingleTenantVolume,
            RebornProfile::Production,
            RebornProfile::MigrationDryRun,
        ]
    );
}

#[test]
fn profile_parsing_accepts_expected_values() {
    assert_eq!(
        RebornProfile::from_str("local-dev"),
        Ok(RebornProfile::LocalDev)
    );
    assert_eq!(
        RebornProfile::from_str("local-dev-yolo"),
        Ok(RebornProfile::LocalDevYolo)
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
        RebornProfile::from_str("production"),
        Ok(RebornProfile::Production)
    );
    assert_eq!(
        RebornProfile::from_str("migration-dry-run"),
        Ok(RebornProfile::MigrationDryRun)
    );
}

#[test]
fn profile_predicates_capture_hosted_volume_local_runtime_contract() {
    assert!(!RebornProfile::LocalDev.starts_hosted_single_tenant_listener());
    assert!(!RebornProfile::LocalDevYolo.starts_hosted_single_tenant_listener());
    assert!(RebornProfile::HostedSingleTenant.starts_hosted_single_tenant_listener());
    assert!(RebornProfile::HostedSingleTenantVolume.starts_hosted_single_tenant_listener());
    assert!(!RebornProfile::Production.starts_hosted_single_tenant_listener());
    assert!(!RebornProfile::MigrationDryRun.starts_hosted_single_tenant_listener());

    assert!(RebornProfile::LocalDev.uses_standalone_local_runtime_volume());
    assert!(RebornProfile::LocalDevYolo.uses_standalone_local_runtime_volume());
    assert!(!RebornProfile::HostedSingleTenant.uses_standalone_local_runtime_volume());
    assert!(RebornProfile::HostedSingleTenantVolume.uses_standalone_local_runtime_volume());
    assert!(!RebornProfile::Production.uses_standalone_local_runtime_volume());
    assert!(!RebornProfile::MigrationDryRun.uses_standalone_local_runtime_volume());

    assert_eq!(
        RebornProfile::LocalDev.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        RebornProfile::LocalDevYolo.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenant.local_runtime_storage_subdir(),
        "hosted-single-tenant"
    );
    assert_eq!(
        RebornProfile::HostedSingleTenantVolume.local_runtime_storage_subdir(),
        "hosted-single-tenant-volume"
    );
    assert_eq!(
        RebornProfile::Production.local_runtime_storage_subdir(),
        "local-dev"
    );
    assert_eq!(
        RebornProfile::MigrationDryRun.local_runtime_storage_subdir(),
        "local-dev"
    );

    assert!(RebornProfile::LocalDev.supports_local_runtime_skill_management());
    assert!(RebornProfile::LocalDevYolo.supports_local_runtime_skill_management());
    assert!(RebornProfile::HostedSingleTenant.supports_local_runtime_skill_management());
    assert!(RebornProfile::HostedSingleTenantVolume.supports_local_runtime_skill_management());
    assert!(!RebornProfile::Production.supports_local_runtime_skill_management());
    assert!(!RebornProfile::MigrationDryRun.supports_local_runtime_skill_management());
}

#[test]
fn profile_default_is_local_dev_for_explicit_binary_invocations() {
    assert_eq!(RebornProfile::default(), RebornProfile::LocalDev);
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
fn boot_config_defaults_profile_to_local_dev() {
    let temp = tempfile::tempdir().expect("tempdir");

    let config =
        RebornBootConfig::resolve_from_env_parts(None, Some(temp.path().into()), None, None)
            .expect("boot config should resolve");

    assert_eq!(config.profile(), RebornProfile::LocalDev);
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
