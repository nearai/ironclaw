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
        RebornProfile::from_str("production"),
        Ok(RebornProfile::Production)
    );
    assert_eq!(
        RebornProfile::from_str("migration-dry-run"),
        Ok(RebornProfile::MigrationDryRun)
    );
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
