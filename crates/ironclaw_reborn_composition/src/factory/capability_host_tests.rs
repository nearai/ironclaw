//! Capability-host factory tests.

use ironclaw_host_api::mount::MountPermissions;

use super::*;

mod approval_gates;

#[tokio::test]
async fn local_yolo_policy_mounts_confirmed_host_home_as_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("standalone");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let services = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_runtime_confirmed_host_home_root(host_home.clone()),
    )
    .await
    .expect("standalone-unrestricted services build");
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("standalone runtime substrate");

    let host_mount = runtime_surfaces
        .workspace_mounts_for_test()
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/host")
        .expect("host mount exists");
    assert_eq!(host_mount.target.as_str(), "/projects/host");
    assert_eq!(host_mount.permissions, MountPermissions::read_write());

    let raw_host_home_alias = host_home
        .canonicalize()
        .expect("canonical host home")
        .to_string_lossy()
        .into_owned();
    let raw_host_home_mount = runtime_surfaces
        .workspace_mounts_for_test()
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == raw_host_home_alias)
        .expect("raw host home mount exists");
    assert_eq!(raw_host_home_mount.target.as_str(), "/projects/host");
    assert_eq!(
        raw_host_home_mount.permissions,
        MountPermissions::read_write()
    );
}

#[tokio::test]
async fn local_yolo_policy_allows_workspace_under_confirmed_host_home() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("standalone");
    let host_home = dir.path().join("home");
    let workspace_root = host_home.join("repo");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");

    let services = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_runtime_workspace_root(workspace_root)
        .with_local_runtime_confirmed_host_home_root(host_home),
    )
    .await
    .expect("standalone-unrestricted services build");
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("standalone runtime substrate");

    let workspace_mount = runtime_surfaces
        .workspace_mounts_for_test()
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/workspace")
        .expect("workspace mount exists");
    assert_eq!(workspace_mount.target.as_str(), "/projects/workspace");
    assert_eq!(workspace_mount.permissions, MountPermissions::read_write());

    let host_mount = runtime_surfaces
        .workspace_mounts_for_test()
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/host")
        .expect("host mount exists");
    assert_eq!(host_mount.target.as_str(), "/projects/host");
    assert_eq!(host_mount.permissions, MountPermissions::read_write());
}

#[cfg(unix)]
#[tokio::test]
async fn local_yolo_policy_keeps_symlinked_host_home_raw_alias() {
    let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
    let storage_root = dir.path().join("standalone");
    let host_home = dir.path().join("home");
    let host_home_link = dir.path().join("home-link");
    std::fs::create_dir_all(&host_home).expect("host home root"); // safety: test-only setup in #[cfg(test)] module.
    std::os::unix::fs::symlink(&host_home, &host_home_link).expect("host home symlink"); // safety: test-only setup in #[cfg(test)] module.

    let services = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_runtime_confirmed_host_home_root(host_home_link.clone()),
    )
    .await
    .expect("standalone-unrestricted services build"); // safety: test-only assertion in #[cfg(test)] module.
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("standalone runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.

    let raw_aliases = runtime_surfaces
        .workspace_mounts_for_test()
        .mounts
        .iter()
        .map(|mount| mount.alias.as_str())
        .collect::<Vec<_>>();
    let raw_alias_includes_original =
        raw_aliases.contains(&host_home_link.to_str().expect("utf-8 link path")); // safety: temp paths are test-owned.
    assert!(raw_alias_includes_original); // safety: test-only assertion in #[cfg(test)] module.
    let canonical_host_home = host_home
        .canonicalize()
        .expect("canonical home") // safety: test setup created this path.
        .to_str()
        .expect("utf-8 canonical path") // safety: temp paths are test-owned.
        .to_string();
    let raw_alias_includes_canonical = raw_aliases.contains(&canonical_host_home.as_str());
    assert!(raw_alias_includes_canonical); // safety: test-only assertion in #[cfg(test)] module.
}

#[tokio::test]
async fn local_yolo_policy_requires_confirmed_host_home_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            dir.path().join("standalone"),
        )
        .with_runtime_policy(local_yolo_policy()),
    )
    .await
    .expect_err("host home policy needs confirmed root");

    assert!(format!("{error}").contains("confirmed host home root"));
}

#[tokio::test]
async fn confirmed_host_home_root_is_rejected_without_matching_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let error = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input(
            "standalone-host-owner",
            dir.path().join("standalone"),
        )
        .with_runtime_policy(local_host_policy())
        .with_local_runtime_confirmed_host_home_root(host_home),
    )
    .await
    .expect_err("host home root needs matching policy");

    assert!(format!("{error}").contains("does not allow host home access"));
}

#[tokio::test]
async fn local_yolo_policy_rejects_confirmed_host_home_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home_file = dir.path().join("home-file");
    std::fs::write(&host_home_file, "not a directory").expect("host home file");

    let error = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            dir.path().join("standalone"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_runtime_confirmed_host_home_root(host_home_file),
    )
    .await
    .expect_err("host home root must be a directory");

    assert!(format!("{error}").contains("must be an existing directory"));
}

#[tokio::test]
async fn local_yolo_policy_rejects_confirmed_host_home_filesystem_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = build_runtime_substrate(
        crate::deployment::local_filesystem_build_input_with_profile(
            RebornCompositionProfile::StandaloneUnrestricted,
            "standalone-unrestricted-host-owner",
            dir.path().join("standalone"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_runtime_confirmed_host_home_root(filesystem_root()),
    )
    .await
    .expect_err("host home root must not be a filesystem root");

    assert!(format!("{error}").contains("must not be a filesystem root"));
}

fn local_yolo_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::standalone_unrestricted_runtime_policy(true).expect("local-yolo policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn local_host_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::standalone_runtime_policy().expect("standalone policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn filesystem_root() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("current dir"); // safety: test-only helper in #[cfg(test)] module.
    while let Some(parent) = path.parent() {
        path = parent.to_path_buf();
    }
    path
}
