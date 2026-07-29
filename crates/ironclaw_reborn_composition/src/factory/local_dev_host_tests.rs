use ironclaw_host_api::MountPermissions;

use super::*;

mod approval_gates;

#[tokio::test]
async fn local_yolo_policy_mounts_confirmed_host_home_as_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let services = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home.clone()),
    )
    .await
    .expect("local-dev-yolo services build");
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local-dev runtime substrate");

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
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    let workspace_root = host_home.join("repo");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");

    let services = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
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
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local-dev runtime substrate");

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
    let storage_root = dir.path().join("local-dev");
    let host_home = dir.path().join("home");
    let host_home_link = dir.path().join("home-link");
    std::fs::create_dir_all(&host_home).expect("host home root"); // safety: test-only setup in #[cfg(test)] module.
    std::os::unix::fs::symlink(&host_home, &host_home_link).expect("host home symlink"); // safety: test-only setup in #[cfg(test)] module.

    let services = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-yolo-host-owner",
            storage_root,
        )
        .with_runtime_policy(local_yolo_policy())
        .with_local_dev_confirmed_host_home_root(host_home_link.clone()),
    )
    .await
    .expect("local-dev-yolo services build"); // safety: test-only assertion in #[cfg(test)] module.
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local-dev runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.

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
        crate::deployment::local_dev_build_input_with_profile(
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

    let error = build_runtime_substrate(
        crate::deployment::local_dev_build_input(
            "local-dev-host-owner",
            dir.path().join("local-dev"),
        )
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

    let error = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
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
    let error = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
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

/// CONFIRMED cross-tenant skills escape (ported from a prior audit, sandbox
/// credential-firewall program): unlike a `mount_local_per_leaf` mount, the
/// `/projects` disk mount backing `scoped_skill_management_mount_view`'s
/// `/skills` grant (`local_dev_project_filesystem`) is a plain `mount_local`
/// — its containment root is the ENTIRE local-dev storage root, not the
/// caller's own `tenants/{tenant}/users/{user}/skills` leaf. A symlink
/// planted inside tenant A's skills directory pointing at tenant B's skills
/// directory stays within the shared storage root, so containment checked
/// against the whole root (not a per-tenant leaf) does not catch it.
///
/// RED before the fix: the read succeeds and returns tenant B's bytes.
/// GREEN after: `ensure_scoped_mount` (`ironclaw_filesystem::DiskFilesystem`)
/// registers a mount rooted exactly at the resolved `/skills` grant target
/// before the read, so `RESOLVE_BENEATH` is anchored at that leaf and the
/// escape is rejected.
#[tokio::test]
async fn skills_shared_mount_cross_tenant_symlink_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    std::fs::create_dir_all(&root).expect("root");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    std::fs::create_dir_all(root.join("system/extensions")).expect("system extensions root");
    std::fs::create_dir_all(root.join("system/skills")).expect("system skills root");

    let filesystem = local_dev_project_filesystem(&root, &workspace_root, None)
        .expect("local-dev project filesystem, production shape");

    let scope_a = owner_scope_from_runtime_identity(
        ironclaw_host_api::UserId::new("user-a").expect("user id"),
        ironclaw_host_api::TenantId::new("tenant-a").expect("tenant id"),
        ironclaw_host_api::AgentId::new("agent").expect("agent id"),
    );
    let scope_b = owner_scope_from_runtime_identity(
        ironclaw_host_api::UserId::new("user-b").expect("user id"),
        ironclaw_host_api::TenantId::new("tenant-b").expect("tenant id"),
        ironclaw_host_api::AgentId::new("agent").expect("agent id"),
    );

    let skills_dir_for = |scope: &ironclaw_host_api::ResourceScope| {
        root.join("tenants")
            .join(scope.tenant_id.as_str())
            .join("users")
            .join(scope.user_id.as_str())
            .join("skills")
    };
    let skills_a = skills_dir_for(&scope_a);
    let skills_b = skills_dir_for(&scope_b);
    std::fs::create_dir_all(&skills_a).expect("tenant a skills dir");
    std::fs::create_dir_all(&skills_b).expect("tenant b skills dir");
    std::fs::write(skills_b.join("secret.txt"), b"tenant-b-only").expect("tenant b secret file");

    // Relative, not absolute: an absolute-target symlink is already rejected
    // outright by the fd-rooted traversal's symlink policy regardless of
    // where it points (see `ironclaw_filesystem::local::fd_resolve`'s
    // `walk_symlink_target`), so an absolute-path reproduction here would
    // pass even without this test's fix and prove nothing about the
    // cross-tenant containment boundary specifically. A relative target —
    // exactly what a real symlink authored *inside* the sandboxed tree would
    // use — is the actual escape vector this test exists to close: `skills_a`
    // is `tenants/tenant-a/users/user-a/skills` (4 levels below `tenants`),
    // so 4 `..` reach `tenants` and the walk descends into tenant B's leaf.
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        "../../../../tenant-b/users/user-b/skills/secret.txt",
        skills_a.join("evil-read"),
    )
    .expect("plant cross-tenant symlink inside tenant A's own skills leaf");

    let view = crate::local_dev_mounts::scoped_skill_management_mount_view(&scope_a)
        .expect("valid skill mounts");
    let grant = view
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/skills")
        .expect("/skills grant present");
    let target_path =
        ironclaw_host_api::VirtualPath::new(format!("{}/evil-read", grant.target.as_str()))
            .expect("virtual path");

    filesystem
        .ensure_scoped_mount(&grant.target)
        .await
        .expect("anchor mount for tenant A's own skills leaf");

    let escape = filesystem.read_file(&target_path).await;

    match escape {
        Err(error) => {
            assert!(
                matches!(
                    error,
                    ironclaw_filesystem::FilesystemError::SymlinkEscape { .. }
                ),
                "expected SymlinkEscape if rejected, got: {error:?}"
            );
        }
        Ok(bytes) => {
            panic!(
                "CROSS-TENANT ESCAPE CONFIRMED: tenant A's /skills grant read tenant B's data \
                 through a symlink: {:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}

/// Same-tenant CROSS-USER variant — the case a naive single-level
/// `mount_local_per_leaf`-style mirror (anchoring only at the tenant
/// segment) would NOT have caught, since two users under the SAME tenant
/// would still share that tenant-level anchor. `ensure_scoped_mount` closes
/// this too because the anchor is the grant's *full* target
/// (`tenants/{t}/users/{u}/skills`), not a fixed single segment.
#[tokio::test]
async fn skills_shared_mount_same_tenant_cross_user_symlink_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    std::fs::create_dir_all(&root).expect("root");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    std::fs::create_dir_all(root.join("system/extensions")).expect("system extensions root");
    std::fs::create_dir_all(root.join("system/skills")).expect("system skills root");

    let filesystem = local_dev_project_filesystem(&root, &workspace_root, None)
        .expect("local-dev project filesystem, production shape");

    let scope_a = owner_scope_from_runtime_identity(
        ironclaw_host_api::UserId::new("user-a").expect("user id"),
        ironclaw_host_api::TenantId::new("shared-tenant").expect("tenant id"),
        ironclaw_host_api::AgentId::new("agent").expect("agent id"),
    );
    let scope_b = owner_scope_from_runtime_identity(
        ironclaw_host_api::UserId::new("user-b").expect("user id"),
        ironclaw_host_api::TenantId::new("shared-tenant").expect("tenant id"),
        ironclaw_host_api::AgentId::new("agent").expect("agent id"),
    );

    let skills_dir_for = |scope: &ironclaw_host_api::ResourceScope| {
        root.join("tenants")
            .join(scope.tenant_id.as_str())
            .join("users")
            .join(scope.user_id.as_str())
            .join("skills")
    };
    let skills_a = skills_dir_for(&scope_a);
    let skills_b = skills_dir_for(&scope_b);
    std::fs::create_dir_all(&skills_a).expect("user a skills dir");
    std::fs::create_dir_all(&skills_b).expect("user b skills dir");
    std::fs::write(skills_b.join("secret.txt"), b"user-b-only").expect("user b secret file");

    // Relative (see the cross-tenant test above for why): `skills_a` is
    // `tenants/shared-tenant/users/user-a/skills`, so 2 `..` reach
    // `tenants/shared-tenant/users` and the walk descends into user B's leaf
    // — same tenant, sibling user.
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../user-b/skills/secret.txt", skills_a.join("evil-read"))
        .expect("plant cross-user symlink inside user A's own skills leaf, same tenant");

    let view = crate::local_dev_mounts::scoped_skill_management_mount_view(&scope_a)
        .expect("valid skill mounts");
    let grant = view
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/skills")
        .expect("/skills grant present");
    let target_path =
        ironclaw_host_api::VirtualPath::new(format!("{}/evil-read", grant.target.as_str()))
            .expect("virtual path");

    filesystem
        .ensure_scoped_mount(&grant.target)
        .await
        .expect("anchor mount for user A's own skills leaf");

    let escape = filesystem.read_file(&target_path).await;

    match escape {
        Err(error) => {
            assert!(
                matches!(
                    error,
                    ironclaw_filesystem::FilesystemError::SymlinkEscape { .. }
                ),
                "expected SymlinkEscape if rejected, got: {error:?}"
            );
        }
        Ok(bytes) => {
            panic!(
                "SAME-TENANT CROSS-USER ESCAPE CONFIRMED: user A's /skills grant read user B's \
                 data through a symlink: {:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}

/// The deliverable regression: the two tests above prove `ensure_scoped_mount`
/// itself closes the escape, but they call it directly on a hand-built
/// `DiskFilesystem` — never the production entry point. Production skill
/// reads go through `ScopedSkillManagementPort::read_content_for_scope`,
/// which builds a `SkillManagementContext` wrapping the filesystem in
/// `ScopedFilesystem` and never touched `ensure_scoped_mount` before this
/// fix (`ScopedSkillManagementPort`'s only filesystem handle is a
/// type-erased `Arc<dyn RootFilesystem>`, which has zero production callers
/// of the primitive — see the fix commit message). This test drives that
/// exact production call chain — `ScopedSkillManagementPort` ->
/// `SkillManagementContext` -> `ScopedFilesystem` -> `CompositeRootFilesystem`
/// -> `DiskFilesystem` — with no direct `ensure_scoped_mount` call anywhere
/// in the test itself.
///
/// Before the `scoped.rs` wiring fix, this test is RED: tenant A's `evil`
/// skill resolves through a relative symlink into tenant B's leaf and
/// `read_content_for_scope` returns tenant B's real skill content. After the
/// fix, `ScopedFilesystem::resolve_with_permission` calls
/// `ensure_scoped_mount(&grant.target)` on every resolution (including the
/// `stat`/`read_bytes_bounded` calls `read_skill_content` makes), narrowing
/// containment to tenant A's own leaf before either syscall — the escape is
/// rejected and the call errors instead of leaking tenant B's bytes.
#[tokio::test]
async fn skills_shared_mount_cross_tenant_symlink_read_through_scoped_skill_management_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    std::fs::create_dir_all(&root).expect("root");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    std::fs::create_dir_all(root.join("system/extensions")).expect("system extensions root");
    std::fs::create_dir_all(root.join("system/skills")).expect("system skills root");

    let filesystem: std::sync::Arc<dyn ironclaw_filesystem::RootFilesystem> = std::sync::Arc::new(
        local_dev_project_filesystem(&root, &workspace_root, None)
            .expect("local-dev project filesystem, production shape"),
    );

    let scope_a = owner_scope_from_runtime_identity(
        ironclaw_host_api::UserId::new("user-a").expect("user id"),
        ironclaw_host_api::TenantId::new("tenant-a").expect("tenant id"),
        ironclaw_host_api::AgentId::new("agent").expect("agent id"),
    );

    let skills_dir_for = |scope: &ironclaw_host_api::ResourceScope| {
        root.join("tenants")
            .join(scope.tenant_id.as_str())
            .join("users")
            .join(scope.user_id.as_str())
            .join("skills")
    };
    let skills_a = skills_dir_for(&scope_a);
    let skills_b = root
        .join("tenants")
        .join("tenant-b")
        .join("users")
        .join("user-b")
        .join("skills");
    std::fs::create_dir_all(&skills_a).expect("tenant a skills dir");
    let secret_skill_dir = skills_b.join("secret-skill");
    std::fs::create_dir_all(&secret_skill_dir).expect("tenant b secret skill dir");
    std::fs::write(
        secret_skill_dir.join("SKILL.md"),
        b"TENANT-B-SECRET-SKILL-CONTENT",
    )
    .expect("tenant b secret skill file");

    // Tenant A's own skills leaf gets a *real* directory named "evil"
    // (`validate_skill_name` requires a plain alnum/`.`/`-`/`_` name, so the
    // escape cannot itself be the leaf-named entry) containing a symlinked
    // `SKILL.md`. `evil/` is one level deeper than `skills_a` itself, so
    // reaching `tenants` from inside it takes 5 `..` (vs. the 4 the sibling
    // tests above use directly inside `skills_a`).
    #[cfg(unix)]
    {
        let evil_dir = skills_a.join("evil");
        std::fs::create_dir_all(&evil_dir).expect("tenant a evil skill dir");
        std::os::unix::fs::symlink(
            "../../../../../tenant-b/users/user-b/skills/secret-skill/SKILL.md",
            evil_dir.join("SKILL.md"),
        )
        .expect("plant cross-tenant symlink inside tenant A's own skills leaf");
    }

    let port = ironclaw_skills::ScopedSkillManagementPort::new_with_mount_resolver(
        scope_a.user_id.clone(),
        filesystem,
        std::sync::Arc::new(crate::local_dev_mounts::scoped_skill_management_mount_view),
    );

    let escape = port.read_content_for_scope(scope_a, "evil").await;

    match escape {
        Err(_error) => {
            // Any error is the closed-escape outcome: `stat`/`read_bytes_bounded`
            // surface `SymlinkEscape` once containment is narrowed, and
            // `ScopedSkillManagementPort` maps every `FilesystemError` through
            // its own error type rather than passing it through verbatim, so
            // the specific variant is an internal implementation detail this
            // test does not pin — only "did not leak tenant B's bytes" does.
        }
        Ok(result) => {
            panic!(
                "CROSS-TENANT ESCAPE CONFIRMED THROUGH THE PRODUCTION PATH: \
                 ScopedSkillManagementPort::read_content_for_scope returned \
                 tenant B's data: {:?}",
                result.content
            );
        }
    }
}
