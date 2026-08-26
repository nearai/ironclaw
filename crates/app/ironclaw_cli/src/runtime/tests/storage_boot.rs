//! Storage-layout boot and migration tests.
//!
//! The production orchestration lives in `runtime/storage_boot.rs`; these
//! tests stay adjacent to that owner while sharing the runtime test module's
//! process-environment guards. The two `pub(super)` helpers are used by later
//! runtime tests without duplicating their fixtures or filesystem snapshots.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::*;

pub(super) fn boot_config_with_config_toml(
    profile: &str,
    config_toml: &str,
) -> (tempfile::TempDir, RebornBootConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(reborn_home.join("config.toml"), config_toml).expect("write config");
    let config = RebornBootConfig::resolve_from_env_parts(
        Some(reborn_home.into_os_string()),
        None,
        None,
        Some(profile.into()),
    )
    .expect("boot config");
    (temp, config)
}

fn boot_config_with_file_profile(profile: &str) -> (tempfile::TempDir, RebornBootConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        format!("[boot]\nprofile = \"{profile}\"\n"),
    )
    .expect("write config");
    let config = RebornBootConfig::resolve_from_env_parts(
        Some(reborn_home.into_os_string()),
        None,
        None,
        None,
    )
    .expect("boot config without a profile environment override");
    (temp, config)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct LayoutEntrySnapshot {
    kind: &'static str,
    bytes: Option<Vec<u8>>,
    len: u64,
    modified: SystemTime,
    readonly: bool,
}

pub(super) fn layout_tree_snapshot(root: &Path) -> BTreeMap<PathBuf, LayoutEntrySnapshot> {
    fn collect(
        root: &Path,
        directory: &Path,
        entries: &mut BTreeMap<PathBuf, LayoutEntrySnapshot>,
    ) {
        for entry in std::fs::read_dir(directory).expect("read layout directory") {
            let entry = entry.expect("layout directory entry");
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).expect("layout entry metadata");
            let kind = if metadata.file_type().is_symlink() {
                "symlink"
            } else if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let bytes = metadata
                .is_file()
                .then(|| std::fs::read(&path).expect("read layout file for immutable snapshot"));
            let relative = path
                .strip_prefix(root)
                .expect("layout entry stays under root")
                .to_path_buf();
            entries.insert(
                relative,
                LayoutEntrySnapshot {
                    kind,
                    bytes,
                    len: metadata.len(),
                    modified: metadata.modified().expect("layout modification time"),
                    readonly: metadata.permissions().readonly(),
                },
            );
            if metadata.is_dir() {
                collect(root, &path, entries);
            }
        }
    }

    let mut entries = BTreeMap::new();
    collect(root, root, &mut entries);
    entries
}

fn seed_legacy_embedded_store(root: &Path) {
    std::fs::create_dir_all(root).expect("legacy root");
    let key = ironclaw_secrets::keychain::generate_master_key_hex();
    std::fs::write(
        root.join(ironclaw_composition::STANDALONE_SECRETS_MASTER_KEY_PATH),
        key,
    )
    .expect("legacy key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(
            root.join(ironclaw_composition::STANDALONE_SECRETS_MASTER_KEY_PATH),
            std::fs::Permissions::from_mode(0o600),
        )
        .expect("owner-only legacy key");
    }
    block_on_cli({
        let root = root.to_path_buf();
        async move {
            ironclaw_composition::open_standalone_secret_store(&root)
                .await
                .map(|_| ())
        }
    })
    .expect("seed legacy libSQL store");
}

#[test]
fn config_file_migration_dry_run_rejects_a_legacy_home_without_mutation() {
    // Break caught: dry-run admission derives its requirement from the raw
    // environment/default profile instead of the config-file-only profile
    // that normal boot admits.
    let _lock = lock_runtime_env();
    let _profile = EnvGuard::clear(ironclaw_config::REBORN_PROFILE_ENV);
    let (_temp, config) = boot_config_with_file_profile("migration-dry-run");
    assert_eq!(config.profile(), RebornProfile::Standalone);

    let legacy = config.home().path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let before = layout_tree_snapshot(config.home().path());

    super::ensure_ready_layout_for_active_profile(&config)
        .expect_err("config-file migration dry-run must reject the not-ready layout");

    assert_eq!(
        layout_tree_snapshot(config.home().path()),
        before,
        "migration dry-run must not alter durable layout files"
    );
}

#[test]
fn command_admission_uses_the_config_file_sandboxed_profile_requirement() {
    // Break caught: a config-file-only sandboxed profile is checked as the
    // default standalone profile, rejecting its own safe multi-user layout.
    let _lock = lock_runtime_env();
    let _profile = EnvGuard::clear(ironclaw_config::REBORN_PROFILE_ENV);
    let (_temp, config) = boot_config_with_file_profile("hosted-single-tenant-volume-sandboxed");
    assert_eq!(config.profile(), RebornProfile::Standalone);

    let sandboxed_profile = RebornProfile::HostedSingleTenantVolumeSandboxed;
    let sandboxed_requirement =
        super::storage_boot::storage_layout_requirement_for_profile(sandboxed_profile)
            .expect("sandboxed layout requirement");
    let expected_paths =
        super::storage_layout::ensure_ready_layout(config.home(), sandboxed_requirement)
            .expect("prepare a ready sandboxed layout");
    let before = layout_tree_snapshot(config.home().path());

    let normal_boot_paths = super::ensure_ready_layout_for_active_profile(&config)
        .expect("normal boot admits the config-file sandboxed layout");
    assert_eq!(normal_boot_paths, expected_paths);
    assert_eq!(
        layout_tree_snapshot(config.home().path()),
        before,
        "admitting an already-ready sandboxed layout must not rewrite it"
    );
}

#[test]
fn run_first_then_serve_durably_upgrades_the_workspace_floor_and_rejects_later_run() {
    let _lock = lock_runtime_env();
    let (_temp, config) = boot_config_with_config_toml("local-dev", "");

    super::ensure_startup_layout(&config, RebornProfile::Standalone, false)
        .expect("run-first admission initializes the standalone layout");
    super::ensure_startup_layout(&config, RebornProfile::Standalone, true)
        .expect("serve admission strengthens the established layout");

    let error = super::ensure_startup_layout(&config, RebornProfile::Standalone, false)
        .expect_err("a later run must not weaken the served layout's durable floor");
    assert!(
        error
            .to_string()
            .contains("workspace access floor cannot weaken"),
        "unexpected admission error: {error:#}"
    );
}

#[test]
fn automatic_startup_rejects_unsafe_legacy_shapes_without_mutation() {
    let _lock = lock_runtime_env();
    let _policy = EnvGuard::clear(super::storage_layout::StorageMigrationPolicy::ENV);
    let _postgres_url = EnvGuard::set("IRONCLAW_REBORN_POSTGRES_URL", "not-a-postgres-url");
    let _master_key = EnvGuard::set(
        "IRONCLAW_REBORN_SECRET_MASTER_KEY",
        &ironclaw_secrets::keychain::generate_master_key_hex(),
    );
    let storage_config = r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#;

    for case in ["unknown-entry", "unreleased-sandbox"] {
        let (_temp, config) = boot_config_with_config_toml("hosted-single-tenant", storage_config);
        let home = config.home().path();
        match case {
            "unknown-entry" => {
                let legacy = home.join("hosted-single-tenant");
                std::fs::create_dir_all(&legacy).expect("legacy root");
                std::fs::write(legacy.join("unknown.bin"), b"unknown")
                    .expect("unknown legacy entry");
            }
            "unreleased-sandbox" => {
                seed_legacy_embedded_store(&home.join("hosted-single-tenant-volume-sandboxed"));
            }
            _ => unreachable!("table is exhaustive"),
        }
        let before = layout_tree_snapshot(home);

        let error = super::ensure_startup_layout(&config, RebornProfile::HostedSingleTenant, false)
            .expect_err("unsafe legacy state must fail closed");

        assert!(
            !error.to_string().contains("PostgreSQL store"),
            "{case} must fail during filesystem classification: {error:#}"
        );
        assert_eq!(
            layout_tree_snapshot(home),
            before,
            "{case} must fail without filesystem mutation"
        );
    }
}

#[test]
fn manual_migration_policy_defers_startup_migration_without_mutation() {
    let _lock = lock_runtime_env();
    let _policy = EnvGuard::set(
        super::storage_layout::StorageMigrationPolicy::ENV,
        super::storage_layout::StorageMigrationPolicy::MANUAL,
    );
    let (_temp, config) = boot_config_with_config_toml("local-dev", "");
    let legacy = config.home().path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let before = layout_tree_snapshot(config.home().path());

    let error = super::ensure_startup_layout(&config, RebornProfile::Standalone, false)
        .expect_err("manual policy defers boot-time migration");

    assert!(
        error
            .to_string()
            .contains(super::storage_layout::StorageMigrationPolicy::ENV),
        "{error:#}"
    );
    assert_eq!(
        layout_tree_snapshot(config.home().path()),
        before,
        "deferred migration must not alter durable layout files"
    );
}

#[test]
fn automatic_startup_migrates_a_legacy_local_dev_home() {
    let _lock = lock_runtime_env();
    let _policy = EnvGuard::clear(super::storage_layout::StorageMigrationPolicy::ENV);
    let (_temp, config) = boot_config_with_config_toml("local-dev", "");
    let legacy = config.home().path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let paths = super::ensure_startup_layout(&config, RebornProfile::Standalone, false)
        .expect("automatic startup migration");

    assert!(paths.state_root().join("reborn-local-dev.db").is_file());
    assert!(config.home().path().join("layout.toml").is_file());
    assert!(!legacy.join("reborn-local-dev.db").exists());

    // A second startup admits the migrated layout without further writes.
    let before = layout_tree_snapshot(config.home().path());
    let readmitted = super::ensure_startup_layout(&config, RebornProfile::Standalone, false)
        .expect("ready admission after migration");
    assert_eq!(readmitted, paths);
    assert_eq!(layout_tree_snapshot(config.home().path()), before);
}

#[test]
fn compatible_base_docker_railway_layout_admission_never_rewrites_the_ready_layout() {
    let _lock = lock_runtime_env();
    let (_enabled, _interval) = clear_trigger_poller_env();
    let (_temp, base_config) = boot_config_with_config_toml("hosted-single-tenant-volume", "");
    let home = base_config.home().path().to_path_buf();

    let paths = super::ensure_ready_layout_for_profile(
        &base_config,
        RebornProfile::HostedSingleTenantVolume,
    )
    .expect("base profile initializes the canonical layout");
    let ready_layout = layout_tree_snapshot(&home);

    for profile in [
        RebornProfile::HostedSingleTenantVolumeSandboxed,
        RebornProfile::HostedSingleTenantVolumeSandboxedRailway,
    ] {
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(home.clone().into_os_string()),
            None,
            None,
            Some(profile.as_str().into()),
        )
        .expect("profile-specific boot config");
        let admitted = super::ensure_ready_layout_for_profile(&config, profile)
            .expect("compatible sandbox profile admits the ready layout");

        assert_eq!(
            admitted, paths,
            "compatible profile {profile} must resolve the same canonical paths"
        );
        assert_eq!(
            layout_tree_snapshot(&home),
            ready_layout,
            "normal boot for compatible profile {profile} must not perform migration writes"
        );
    }
}

#[test]
fn runtime_input_refuses_unsafe_legacy_state_before_constructing_runtime_services() {
    let _lock = lock_runtime_env();
    let (_enabled, _interval) = clear_trigger_poller_env();
    let (_temp, config) = boot_config_with_config_toml("local-dev", "");
    let legacy_root = config.home().path().join("local-dev");
    std::fs::create_dir_all(&legacy_root).expect("create legacy root");
    // An embedded database without its cached master key is an unsafe shape:
    // migrating it would make every encrypted secret unreadable.
    std::fs::write(
        legacy_root.join("reborn-local-dev.db"),
        b"legacy state sentinel",
    )
    .expect("seed legacy state");

    let error = match build_runtime_input(&config, RuntimeInputCaller::Run) {
        Ok(_) => panic!("unsafe legacy state must be rejected before runtime construction"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("master key"),
        "expected the missing-master-key refusal, got {error:#}"
    );
    assert!(
        !config
            .home()
            .path()
            .join("state")
            .join("reborn-local-dev.db")
            .exists(),
        "runtime input admission must not open or create canonical state"
    );
}

#[test]
fn local_state_root_is_the_canonical_state_namespace_for_every_profile() {
    for &profile in ironclaw_config::RebornProfile::all() {
        let (_temp, config) = boot_config_with_config_toml(profile.as_str(), "");
        assert_eq!(
            local_state_root(&config),
            config.home().path().join("state")
        );
    }
}

#[test]
fn cli_secret_store_refuses_external_postgres_profile_before_opening_local_libsql() {
    let _lock = lock_runtime_env();
    let _profile = EnvGuard::clear(ironclaw_config::REBORN_PROFILE_ENV);
    let (_temp, config) = boot_config_with_file_profile("hosted-single-tenant");

    let error = super::ensure_embedded_secret_store_for_active_profile(&config)
        .expect_err("hosted CLI secret writes must not create a shadow libSQL store");

    assert!(
        error.to_string().contains("external PostgreSQL"),
        "{error:#}"
    );
    assert!(
        !config
            .home()
            .path()
            .join("state/reborn-local-dev.db")
            .exists()
    );
}
