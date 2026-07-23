use std::{ffi::OsString, path::Path};

use ironclaw_config::{
    IRONCLAW_HOME_ENV, IronClawConfigError, IronClawHome, IronClawHomeSource,
    LEGACY_IRONCLAW_HOME_ENV,
};

#[test]
fn explicit_ironclaw_home_wins_and_must_not_create_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let explicit = temp.path().join("ironclaw-home");

    let home = IronClawHome::resolve_from_env_parts(
        Some(explicit.clone().into_os_string()),
        Some(temp.path().join("ignored-home").into_os_string()),
        None,
    )
    .expect("absolute explicit IronClaw home should resolve");

    assert_eq!(home.path(), explicit.as_path());
    assert_eq!(home.source(), IronClawHomeSource::Env);
    assert_eq!(home.source_label(), IRONCLAW_HOME_ENV);
    assert!(!explicit.exists(), "resolver must be side-effect free");
}

#[test]
fn default_ironclaw_home_is_scoped_under_user_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw");

    let home = IronClawHome::resolve_from_env_parts(None, Some(temp.path().into()), None)
        .expect("absolute HOME should resolve default IronClaw home");

    assert_eq!(home.path(), expected.as_path());
    assert_eq!(home.source(), IronClawHomeSource::Default);
    assert_eq!(home.source_label(), "default");
    assert!(!expected.exists());
}

#[test]
fn userprofile_is_used_only_when_home_is_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw");

    let home = IronClawHome::resolve_from_env_parts(None, None, Some(temp.path().into()))
        .expect("absolute USERPROFILE should resolve default IronClaw home");

    assert_eq!(home.path(), expected.as_path());
}

#[test]
fn userprofile_is_used_when_home_is_empty_or_invalid() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw");

    let empty_home =
        IronClawHome::resolve_from_env_parts(None, Some(OsString::new()), Some(temp.path().into()))
            .expect("valid USERPROFILE should be used when HOME is empty");
    assert_eq!(empty_home.path(), expected.as_path());

    let relative_home = IronClawHome::resolve_from_env_parts(
        None,
        Some(OsString::from("relative-home")),
        Some(temp.path().into()),
    )
    .expect("valid USERPROFILE should be used when HOME is relative");
    assert_eq!(relative_home.path(), expected.as_path());
}

#[test]
fn canonical_home_can_use_the_retired_v1_state_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join(".ironclaw");
    let home = IronClawHome::resolve_from_env_parts(
        Some(path.clone().into_os_string()),
        Some(temp.path().into()),
        None,
    )
    .expect("the default IronClaw home now owns the retired v1 root");

    assert_eq!(home.path(), path);
    assert_eq!(home.source(), IronClawHomeSource::Env);
}

#[test]
fn legacy_home_still_rejects_the_retired_v1_state_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join(".ironclaw");
    let err = IronClawHome::resolve_from_env_parts_with_legacy(
        None,
        Some(path.clone().into_os_string()),
        None,
        Some(temp.path().into()),
    )
    .expect_err("legacy IronClaw home must preserve its v1 collision guard");

    assert!(
        matches!(err, IronClawConfigError::V1StateRoot { name, path: actual } if name == LEGACY_IRONCLAW_HOME_ENV && actual == path)
    );
}

#[cfg(unix)]
#[test]
fn rejects_ironclaw_home_symlink_to_home_v1_state_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let v1_root = temp.path().join(".ironclaw");
    std::fs::create_dir(&v1_root).expect("create v1 root");
    let ironclaw_link = temp.path().join("ironclaw-link");
    symlink(&v1_root, &ironclaw_link).expect("symlink IronClaw home to v1 root");

    let err = IronClawHome::resolve_from_env_parts_with_legacy(
        None,
        Some(ironclaw_link.clone().into_os_string()),
        Some(temp.path().into()),
        None,
    )
    .expect_err("IronClaw home symlink to v1 state root must fail");

    assert!(
        matches!(err, IronClawConfigError::V1StateRoot { name, path } if name == LEGACY_IRONCLAW_HOME_ENV && path == ironclaw_link)
    );
}

#[test]
fn rejects_parent_components_in_ironclaw_home_override() {
    let path = root_path().join("tmp").join("..");
    let err = IronClawHome::resolve_from_env_parts(Some(path.clone().into_os_string()), None, None)
        .expect_err("parent components in override should fail");

    assert!(
        matches!(err, IronClawConfigError::ParentPath { name, path: actual } if name == IRONCLAW_HOME_ENV && actual == path)
    );
}

#[test]
fn rejects_parent_components_that_would_lexically_resolve_to_root() {
    let path = root_path().join("..");
    let err = IronClawHome::resolve_from_env_parts(Some(path.clone().into_os_string()), None, None)
        .expect_err("root traversal override should fail");

    assert!(
        matches!(err, IronClawConfigError::ParentPath { name, path: actual } if name == IRONCLAW_HOME_ENV && actual == path)
    );
}

#[test]
fn rejects_parent_components_in_default_home_base() {
    let path = root_path().join("tmp").join("..");
    let err = IronClawHome::resolve_from_env_parts(None, Some(path.clone().into_os_string()), None)
        .expect_err("parent components in HOME should fail");

    assert!(
        matches!(err, IronClawConfigError::ParentPath { name, path: actual } if name == "HOME" && actual == path)
    );
}

#[test]
fn rejects_root_ironclaw_home_override() {
    let err = IronClawHome::resolve_from_env_parts(Some(root_path().into()), None, None)
        .expect_err("root override should fail");

    assert!(matches!(err, IronClawConfigError::RootPath { name, .. } if name == IRONCLAW_HOME_ENV));
}

#[test]
fn rejects_root_default_home_base() {
    let err = IronClawHome::resolve_from_env_parts(None, Some(root_path().into()), None)
        .expect_err("root HOME should fail");

    assert!(matches!(err, IronClawConfigError::RootPath { name, .. } if name == "HOME"));
}

#[test]
fn rejects_empty_ironclaw_home_override() {
    let err = IronClawHome::resolve_from_env_parts(Some(OsString::new()), None, None)
        .expect_err("empty override should fail");

    assert!(matches!(err, IronClawConfigError::EmptyPath { name } if name == IRONCLAW_HOME_ENV));
}

#[test]
fn rejects_relative_ironclaw_home_override() {
    let err =
        IronClawHome::resolve_from_env_parts(Some(OsString::from("relative/ironclaw")), None, None)
            .expect_err("relative override should fail");

    assert!(
        matches!(err, IronClawConfigError::RelativePath { name, path } if name == IRONCLAW_HOME_ENV && path == Path::new("relative/ironclaw"))
    );
}

#[test]
fn canonical_home_wins_over_legacy_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = temp.path().join("canonical");
    let legacy = temp.path().join("legacy");

    let home = IronClawHome::resolve_from_env_parts_with_legacy(
        Some(canonical.clone().into_os_string()),
        Some(legacy.into_os_string()),
        None,
        None,
    )
    .expect("canonical home should win");

    assert_eq!(home.path(), canonical);
    assert_eq!(home.source(), IronClawHomeSource::Env);
    assert_eq!(home.source_label(), IRONCLAW_HOME_ENV);
}

#[test]
fn existing_legacy_default_is_adopted_without_moving_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let legacy = temp.path().join(".ironclaw").join("reborn");
    std::fs::create_dir_all(&legacy).expect("create legacy default");

    let home = IronClawHome::resolve_from_env_parts(None, Some(temp.path().into()), None)
        .expect("existing legacy default should resolve");

    assert_eq!(home.path(), legacy);
    assert_eq!(home.source(), IronClawHomeSource::LegacyDefault);
    assert_eq!(home.source_label(), "legacy-default");
}

#[test]
fn rejects_missing_home_for_default_resolution() {
    let err = IronClawHome::resolve_from_env_parts(None, None, None)
        .expect_err("missing home should fail");

    assert_eq!(err, IronClawConfigError::MissingHome);
}

#[test]
fn rejects_relative_default_home_when_no_valid_fallback_exists() {
    let err =
        IronClawHome::resolve_from_env_parts(None, Some(OsString::from("relative-home")), None)
            .expect_err("relative HOME should fail");

    assert!(
        matches!(err, IronClawConfigError::RelativePath { name, path } if name == "HOME" && path == Path::new("relative-home"))
    );
}

fn root_path() -> &'static Path {
    if cfg!(windows) {
        Path::new(r"C:\")
    } else {
        Path::new("/")
    }
}
