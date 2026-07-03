use std::{ffi::OsString, path::Path};

use ironclaw_reborn_config::{REBORN_HOME_ENV, RebornConfigError, RebornHome, RebornHomeSource};

#[test]
fn explicit_reborn_home_wins_and_must_not_create_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let explicit = temp.path().join("reborn-home");

    let home = RebornHome::resolve_from_env_parts(
        Some(explicit.clone().into_os_string()),
        Some(temp.path().join("ignored-home").into_os_string()),
        None,
    )
    .expect("absolute explicit Reborn home should resolve");

    assert_eq!(home.path(), explicit.as_path());
    assert_eq!(home.source(), RebornHomeSource::Env);
    assert_eq!(home.source_label(), REBORN_HOME_ENV);
    assert!(!explicit.exists(), "resolver must be side-effect free");
}

#[test]
fn default_reborn_home_is_scoped_under_user_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw").join("reborn");

    let home = RebornHome::resolve_from_env_parts(None, Some(temp.path().into()), None)
        .expect("absolute HOME should resolve default Reborn home");

    assert_eq!(home.path(), expected.as_path());
    assert_eq!(home.source(), RebornHomeSource::Default);
    assert_eq!(home.source_label(), "default");
    assert!(!temp.path().join(".ironclaw").exists());
}

#[test]
fn userprofile_is_used_only_when_home_is_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw").join("reborn");

    let home = RebornHome::resolve_from_env_parts(None, None, Some(temp.path().into()))
        .expect("absolute USERPROFILE should resolve default Reborn home");

    assert_eq!(home.path(), expected.as_path());
}

#[test]
fn userprofile_is_used_when_home_is_empty_or_invalid() {
    let temp = tempfile::tempdir().expect("tempdir");
    let expected = temp.path().join(".ironclaw").join("reborn");

    let empty_home =
        RebornHome::resolve_from_env_parts(None, Some(OsString::new()), Some(temp.path().into()))
            .expect("valid USERPROFILE should be used when HOME is empty");
    assert_eq!(empty_home.path(), expected.as_path());

    let relative_home = RebornHome::resolve_from_env_parts(
        None,
        Some(OsString::from("relative-home")),
        Some(temp.path().into()),
    )
    .expect("valid USERPROFILE should be used when HOME is relative");
    assert_eq!(relative_home.path(), expected.as_path());
}

#[test]
fn rejects_reborn_home_equal_to_home_v1_state_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join(".ironclaw");
    let err = RebornHome::resolve_from_env_parts(
        Some(path.clone().into_os_string()),
        Some(temp.path().into()),
        None,
    )
    .expect_err("Reborn home must not target default v1 state root");

    assert!(
        matches!(err, RebornConfigError::V1StateRoot { name, path: actual } if name == REBORN_HOME_ENV && actual == path)
    );
}

#[test]
fn rejects_reborn_home_equal_to_userprofile_v1_state_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join(".ironclaw");
    let err = RebornHome::resolve_from_env_parts(
        Some(path.clone().into_os_string()),
        None,
        Some(temp.path().into()),
    )
    .expect_err("Reborn home must not target USERPROFILE v1 state root");

    assert!(
        matches!(err, RebornConfigError::V1StateRoot { name, path: actual } if name == REBORN_HOME_ENV && actual == path)
    );
}

#[cfg(unix)]
#[test]
fn rejects_reborn_home_symlink_to_home_v1_state_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let v1_root = temp.path().join(".ironclaw");
    std::fs::create_dir(&v1_root).expect("create v1 root");
    let reborn_link = temp.path().join("reborn-link");
    symlink(&v1_root, &reborn_link).expect("symlink reborn home to v1 root");

    let err = RebornHome::resolve_from_env_parts(
        Some(reborn_link.clone().into_os_string()),
        Some(temp.path().into()),
        None,
    )
    .expect_err("Reborn home symlink to v1 state root must fail");

    assert!(
        matches!(err, RebornConfigError::V1StateRoot { name, path } if name == REBORN_HOME_ENV && path == reborn_link)
    );
}

#[test]
fn rejects_parent_components_in_reborn_home_override() {
    let path = root_path().join("tmp").join("..");
    let err = RebornHome::resolve_from_env_parts(Some(path.clone().into_os_string()), None, None)
        .expect_err("parent components in override should fail");

    assert!(
        matches!(err, RebornConfigError::ParentPath { name, path: actual } if name == REBORN_HOME_ENV && actual == path)
    );
}

#[test]
fn rejects_parent_components_that_would_lexically_resolve_to_root() {
    let path = root_path().join("..");
    let err = RebornHome::resolve_from_env_parts(Some(path.clone().into_os_string()), None, None)
        .expect_err("root traversal override should fail");

    assert!(
        matches!(err, RebornConfigError::ParentPath { name, path: actual } if name == REBORN_HOME_ENV && actual == path)
    );
}

#[test]
fn rejects_parent_components_in_default_home_base() {
    let path = root_path().join("tmp").join("..");
    let err = RebornHome::resolve_from_env_parts(None, Some(path.clone().into_os_string()), None)
        .expect_err("parent components in HOME should fail");

    assert!(
        matches!(err, RebornConfigError::ParentPath { name, path: actual } if name == "HOME" && actual == path)
    );
}

#[test]
fn rejects_root_reborn_home_override() {
    let err = RebornHome::resolve_from_env_parts(Some(root_path().into()), None, None)
        .expect_err("root override should fail");

    assert!(matches!(err, RebornConfigError::RootPath { name, .. } if name == REBORN_HOME_ENV));
}

#[test]
fn rejects_root_default_home_base() {
    let err = RebornHome::resolve_from_env_parts(None, Some(root_path().into()), None)
        .expect_err("root HOME should fail");

    assert!(matches!(err, RebornConfigError::RootPath { name, .. } if name == "HOME"));
}

#[test]
fn rejects_empty_reborn_home_override() {
    let err = RebornHome::resolve_from_env_parts(Some(OsString::new()), None, None)
        .expect_err("empty override should fail");

    assert!(matches!(err, RebornConfigError::EmptyPath { name } if name == REBORN_HOME_ENV));
}

#[test]
fn rejects_relative_reborn_home_override() {
    let err =
        RebornHome::resolve_from_env_parts(Some(OsString::from("relative/reborn")), None, None)
            .expect_err("relative override should fail");

    assert!(
        matches!(err, RebornConfigError::RelativePath { name, path } if name == REBORN_HOME_ENV && path == Path::new("relative/reborn"))
    );
}

#[test]
fn rejects_missing_home_for_default_resolution() {
    let err =
        RebornHome::resolve_from_env_parts(None, None, None).expect_err("missing home should fail");

    assert_eq!(err, RebornConfigError::MissingHome);
}

#[test]
fn rejects_relative_default_home_when_no_valid_fallback_exists() {
    let err = RebornHome::resolve_from_env_parts(None, Some(OsString::from("relative-home")), None)
        .expect_err("relative HOME should fail");

    assert!(
        matches!(err, RebornConfigError::RelativePath { name, path } if name == "HOME" && path == Path::new("relative-home"))
    );
}

fn root_path() -> &'static Path {
    if cfg!(windows) {
        Path::new(r"C:\")
    } else {
        Path::new("/")
    }
}
