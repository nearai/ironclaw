use ironclaw_host_api::MountPermissions;

use super::*;

mod approval_gates;

#[tokio::test]
async fn local_yolo_policy_mounts_confirmed_host_home_as_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home.clone()),
    )
    .await
    .expect("local-dev-yolo services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");

    let host_mount = local_runtime
        .workspace_mounts
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
    let raw_host_home_mount = local_runtime
        .workspace_mounts
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
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    let workspace_root = host_home.join("repo");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_workspace_root(workspace_root)
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .await
    .expect("local-dev-yolo services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");

    let workspace_mount = local_runtime
        .workspace_mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/workspace")
        .expect("workspace mount exists");
    assert_eq!(workspace_mount.target.as_str(), "/projects/workspace");
    assert_eq!(workspace_mount.permissions, MountPermissions::read_write());

    let host_mount = local_runtime
        .workspace_mounts
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
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    let host_home_link = dir.path().join("home-link");
    std::fs::create_dir_all(&host_home).expect("host home root"); // safety: test-only setup in #[cfg(test)] module.
    std::os::unix::fs::symlink(&host_home, &host_home_link).expect("host home symlink"); // safety: test-only setup in #[cfg(test)] module.

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home_link.clone()),
    )
    .await
    .expect("local-dev-yolo services build"); // safety: test-only assertion in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.

    let raw_aliases = local_runtime
        .workspace_mounts
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
    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
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

    let error = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-host-owner", dir.path().join("local-dev"))
            .with_runtime_policy(local_dev_policy())
            .with_local_dev_confirmed_host_home_root(host_home),
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

    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home_file),
    )
    .await
    .expect_err("host home root must be a directory");

    assert!(format!("{error}").contains("must be an existing directory"));
}

#[tokio::test]
async fn local_yolo_policy_rejects_confirmed_host_home_filesystem_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(filesystem_root()),
    )
    .await
    .expect_err("host home root must not be a filesystem root");

    assert!(format!("{error}").contains("must not be a filesystem root"));
}

fn local_yolo_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn local_dev_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    crate::local_dev_runtime_policy().expect("local-dev policy resolves") // safety: test-only helper in #[cfg(test)] module.
}

fn filesystem_root() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("current dir"); // safety: test-only helper in #[cfg(test)] module.
    while let Some(parent) = path.parent() {
        path = parent.to_path_buf();
    }
    path
}

/// Stub `SandboxCommandTransport` so `is_sandboxed_profile` (gated on
/// `runtime_process_binding` being `TenantSandbox`, not on the deployment
/// profile alone) actually flips true for these tests — mirrors
/// `approval_gates::RecordingSandboxTransport`, never invoked here since
/// these tests exercise the filesystem mount, not shell execution.
#[derive(Debug, Default)]
struct NoopSandboxTransport;

#[async_trait::async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for NoopSandboxTransport {
    async fn run_command(
        &self,
        _request: ironclaw_host_runtime::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_runtime::CommandExecutionOutput,
        ironclaw_host_runtime::RuntimeProcessError,
    > {
        unimplemented!("workspace-mount tests never execute shell commands")
    }
}

fn tenant_sandbox_process_binding_for_test() -> RebornRuntimeProcessBinding {
    let process_port = Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        Arc::new(NoopSandboxTransport),
    ));
    RebornRuntimeProcessBinding::tenant_sandbox(process_port)
}

#[tokio::test]
async fn sandboxed_profile_workspace_mount_is_per_user_and_shares_bytes_with_host_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("hosted-sandboxed");

    let services = build_reborn_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::HostedSingleTenantVolumeSandboxed,
            "sandbox-owner",
            storage_root.clone(),
        )
        .with_runtime_policy(
            crate::hosted_single_tenant_volume_sandboxed_runtime_policy()
                .expect("sandboxed policy resolves"),
        )
        .with_runtime_process_binding(tenant_sandbox_process_binding_for_test()),
    )
    .await
    .expect("hosted-single-tenant-volume-sandboxed services build");
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate");

    let workspace_grant = local_runtime
        .workspace_mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/workspace")
        .expect("/workspace grant exists for the sandboxed profile");
    assert_eq!(workspace_grant.target.as_str(), "/workspace");
    assert_eq!(workspace_grant.permissions, MountPermissions::read_write());

    // write_file (abstract FS) -> assert on the real per-user host dir.
    let path = ironclaw_host_api::VirtualPath::new("/workspace/f.txt").expect("virtual path");
    local_runtime
        .extension_filesystem
        .write_file(&path, b"from-fs-tools")
        .await
        .expect("write through composite /workspace mount");

    let owner_scope = default_runtime_owner_scope(
        ironclaw_host_api::UserId::new("sandbox-owner").expect("owner id"),
    )
    .expect("owner scope resolves");
    let canonical_root = storage_root.canonicalize().expect("canonical storage root");
    let host_workspace_dir = ironclaw_host_runtime::RebornSandboxUserKey::from_scope(&owner_scope)
        .workspace_path(&canonical_root);
    assert_eq!(
        std::fs::read(host_workspace_dir.join("f.txt")).expect("host file exists"),
        b"from-fs-tools"
    );

    // reverse: write directly on the host dir (what a shell `echo` inside the
    // container does), read back through the abstract FS /workspace mount.
    std::fs::write(host_workspace_dir.join("g.txt"), b"from-shell").expect("host write");
    let bytes = local_runtime
        .extension_filesystem
        .read_file(&ironclaw_host_api::VirtualPath::new("/workspace/g.txt").expect("virtual path"))
        .await
        .expect("read through composite /workspace mount");
    assert_eq!(bytes, b"from-shell");
}

#[tokio::test]
async fn sandbox_user_workspace_directories_do_not_overlap_across_owners() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    std::fs::create_dir_all(&root).expect("root");
    let canonical_root = root.canonicalize().expect("canonical root");

    let scope_a =
        default_runtime_owner_scope(ironclaw_host_api::UserId::new("user-a").expect("user id"))
            .expect("owner scope resolves");
    let scope_b =
        default_runtime_owner_scope(ironclaw_host_api::UserId::new("user-b").expect("user id"))
            .expect("owner scope resolves");

    let path_a = ironclaw_host_runtime::RebornSandboxUserKey::from_scope(&scope_a)
        .workspace_path(&canonical_root);
    let path_b = ironclaw_host_runtime::RebornSandboxUserKey::from_scope(&scope_b)
        .workspace_path(&canonical_root);

    assert_ne!(
        path_a, path_b,
        "different owners must not share a workspace directory"
    );
    assert!(
        !path_a.starts_with(&path_b) && !path_b.starts_with(&path_a),
        "one user's workspace directory must not nest inside another's: {path_a:?} vs {path_b:?}"
    );

    // The mount registration itself must fail closed if it were ever pointed
    // at a shared parent instead of the digest leaf: assert mounting user A's
    // path denies access to a file only present under user B's path.
    std::fs::create_dir_all(&path_a).expect("user a dir");
    std::fs::create_dir_all(&path_b).expect("user b dir");
    std::fs::write(path_b.join("secret.txt"), b"user-b-only").expect("user b file");

    let mut composite = CompositeRootFilesystem::new();
    mount_sandbox_user_workspace_root(&mut composite, &path_a).expect("mount user a workspace");
    let escape = composite
        .read_file(
            &ironclaw_host_api::VirtualPath::new("/workspace/secret.txt").expect("virtual path"),
        )
        .await;
    assert!(
        escape.is_err(),
        "user A's /workspace mount must not see user B's file"
    );
}
